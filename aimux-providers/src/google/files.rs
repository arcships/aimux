//! Google Files — implements the `Files` trait for uploading files to the
//! Google Generative Language API.
//!
//! Aligned with Vercel AI SDK `GoogleFiles`
//! (`reference/ai/packages/google/src/google-files.ts`).
//!
//! Google uses a resumable upload protocol:
//! 1. POST to `/upload/v1beta/files` with `X-Goog-Upload-Protocol: resumable`
//!    and `X-Goog-Upload-Command: start` — returns an upload URL in the
//!    `x-goog-upload-url` response header.
//! 2. POST the raw file bytes to the upload URL with
//!    `X-Goog-Upload-Command: upload, finalize`.
//! 3. If the file state is `PROCESSING`, poll `{base_url}/{file.name}` until
//!    the state becomes `ACTIVE` or `FAILED`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData, UploadFileResult};
use aimux_core::shared::{FileBytes, SharedProviderOptions};
use aimux_core::types::Warning;

use aimux_provider_utils::{HttpBody, HttpRequest, sleep_or_abort};

use super::options::GOOGLE;
use crate::shared::EndpointConfig;

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
/// Google provider-specific file upload options.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleFilesUploadOptions {
    display_name: Option<String>,
    poll_interval_ms: Option<f64>,
    poll_timeout_ms: Option<f64>,
}

fn parse_google_files_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<GoogleFilesUploadOptions, AiMuxError> {
    let Some(google) = provider_options.and_then(|options| options.get(GOOGLE)) else {
        return Ok(GoogleFilesUploadOptions::default());
    };
    let options: GoogleFilesUploadOptions = serde_json::from_value(Value::Object(google.clone()))
        .map_err(|error| {
        AiMuxError::InvalidArgument(format!("Invalid Google file options: {error}"))
    })?;
    if options.poll_interval_ms.is_some_and(|value| value <= 0.0)
        || options.poll_timeout_ms.is_some_and(|value| value <= 0.0)
    {
        return Err(AiMuxError::InvalidArgument(
            "Google file polling intervals must be positive".into(),
        ));
    }
    Ok(options)
}

/// Convert `UploadFileData` to raw bytes.
fn data_to_bytes(data: &UploadFileData) -> Result<Vec<u8>, AiMuxError> {
    match data {
        UploadFileData::Data { data } => match data {
            FileBytes::Binary(bytes) => Ok(bytes.clone()),
            FileBytes::Base64(b64) => {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                    .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
            }
        },
        UploadFileData::Text { text } => Ok(text.as_bytes().to_vec()),
    }
}

// Preserve file names in a single path segment, including dot segments.
fn file_poll_path(name: &str) -> String {
    fn encode(segment: &str) -> String {
        if segment == "." {
            return "%252E".to_string();
        }
        if segment == ".." {
            return "%252E%252E".to_string();
        }
        const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
            .remove(b'-')
            .remove(b'_')
            .remove(b'.')
            .remove(b'!')
            .remove(b'~')
            .remove(b'*')
            .remove(b'\'')
            .remove(b'(')
            .remove(b')');
        percent_encoding::utf8_percent_encode(segment, COMPONENT).to_string()
    }
    match name.strip_prefix("files/") {
        Some(segment) if !segment.is_empty() && !segment.contains('/') => {
            format!("files/{}", encode(segment))
        }
        _ => encode(name),
    }
}

/// The file resource returned by the Google upload API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleFileResource {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    mime_type: String,
    #[serde(default)]
    size_bytes: Option<String>,
    #[serde(default)]
    create_time: Option<String>,
    #[serde(default)]
    update_time: Option<String>,
    #[serde(default)]
    expiration_time: Option<String>,
    #[serde(default)]
    sha256_hash: Option<String>,
    uri: String,
    state: String,
}

/// The response from the upload endpoint: `{ "file": { ... } }`.
#[derive(Debug, Deserialize)]
struct UploadResponse {
    file: GoogleFileResource,
}

/// A Google Files interface for uploading files.
///
/// Aligned with TS `GoogleFiles`. Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers
/// uses the process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct GoogleFiles {
    config: EndpointConfig,
}

impl GoogleFiles {
    pub(crate) fn from_config(config: EndpointConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Files for GoogleFiles {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    async fn upload_file(
        &self,
        options: &UploadFileCallOptions,
    ) -> Result<UploadFileResult, AiMuxError> {
        let google_options = parse_google_files_options(options.provider_options.as_ref())?;
        let exchange = self.config.exchange(None).await?;
        // The upload endpoint hangs off the origin: `base_url` without
        // `/v1beta`.
        let init_endpoint = format!(
            "{}/upload/v1beta/files",
            exchange.base_url().trim_end_matches("/v1beta")
        );

        let mut warnings = Vec::new();
        if options.filename.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "filename".to_string(),
                details: None,
            });
        }

        let file_bytes = data_to_bytes(&options.data)?;
        let media_type = &options.media_type;
        let display_name = &google_options.display_name;

        // Build the init body: `{ "file": { "display_name": "..." } }` or
        // `{ "file": {} }` when no display_name is provided.
        let init_body_value: Value = if let Some(name) = display_name {
            json!({ "file": { "display_name": name } })
        } else {
            json!({ "file": {} })
        };

        let mut init_headers = exchange.headers();
        init_headers.push((
            "X-Goog-Upload-Protocol".to_string(),
            "resumable".to_string(),
        ));
        init_headers.push(("X-Goog-Upload-Command".to_string(), "start".to_string()));
        init_headers.push((
            "X-Goog-Upload-Header-Content-Length".to_string(),
            file_bytes.len().to_string(),
        ));
        init_headers.push((
            "X-Goog-Upload-Header-Content-Type".to_string(),
            media_type.clone(),
        ));
        init_headers.push(("Content-Type".to_string(), "application/json".to_string()));

        // Nothing retries a file upload: the init, upload and poll exchanges
        // each run once, and a failure is reported as it happened (a replayed
        // upload would resend the file body).
        let init_resp = aimux_provider_utils::post_json_to_api(
            exchange.with_transport(HttpRequest {
                url: init_endpoint,
                headers: init_headers,
                abort_signal: options.abort_signal.clone(),
                ..Default::default()
            }),
            init_body_value,
            aimux_provider_utils::ResponseHandler::new(|input| async move {
                let headers =
                    aimux_provider_utils::extract_response_headers::extract_response_headers(
                        input.response.headers(),
                    );
                Ok(aimux_provider_utils::ResponseHandlerOutput {
                    value: (),
                    raw_value: None,
                    response_headers: headers,
                })
            }),
            super::google_failed_response_handler(),
        )
        .await
        .map_err(|e| match e {
            AiMuxError::ApiCall(d) => AiMuxError::ApiCall(Box::new(ApiCallError {
                message: format!("Failed to initiate resumable upload: {}", d.message),
                ..*d
            })),
            e => e,
        })?;

        let upload_url = init_resp
            .response_headers
            .get("x-goog-upload-url")
            .cloned()
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData(
                    "No upload URL returned from initiation request".to_string(),
                )
            })?;

        // Step 2: Upload file data.
        let upload_headers: Vec<(String, String)> = vec![
            ("X-Goog-Upload-Offset".to_string(), "0".to_string()),
            (
                "X-Goog-Upload-Command".to_string(),
                "upload, finalize".to_string(),
            ),
        ];

        let upload_request_url = upload_url.clone();
        // The upload URL comes from the init response's x-goog-upload-url
        // header and receives the user's file bytes; validate it. (AI SDK
        // fetches this URL unvalidated — kept stricter here deliberately.)
        let upload_resp = aimux_provider_utils::post_to_api(
            exchange.with_transport(HttpRequest {
                url: upload_url,
                headers: upload_headers,
                abort_signal: options.abort_signal.clone(),
                validate_url: true,
                trusted_origin: Some(exchange.base_url().to_string()),
                credentialed_origin: Some(exchange.base_url().to_string()),
                ..Default::default()
            }),
            HttpBody::Bytes(file_bytes, media_type.clone()),
            aimux_provider_utils::create_json_response_handler::<UploadResponse>(),
            super::google_failed_response_handler(),
        )
        .await
        .map_err(|e| match e {
            AiMuxError::ApiCall(d) => AiMuxError::ApiCall(Box::new(ApiCallError {
                message: format!("Failed to upload file data: {}", d.message),
                ..*d
            })),
            e => e,
        })?;

        let mut file = upload_resp.value.file;
        let mut raw_file = upload_resp
            .raw_value
            .as_ref()
            .and_then(|value| value.get("file"))
            .cloned()
            .unwrap_or(Value::Null);

        // Step 3: Poll if file is PROCESSING.
        let poll_interval_ms = google_options.poll_interval_ms.unwrap_or(2000.0);
        let poll_timeout_ms = google_options.poll_timeout_ms.unwrap_or(300000.0);
        let start_time = Instant::now();

        // Seed evidence from the upload response so a file that is already
        // FAILED (never polled) still carries the observed status + raw body.
        let mut last_poll_body = upload_resp.raw_value.map(|value| value.to_string());
        let mut last_poll_status: Option<u16> = Some(200);
        let mut last_poll_url = upload_request_url;
        while file.state == "PROCESSING" {
            if start_time.elapsed().as_secs_f64() * 1000.0 > poll_timeout_ms {
                return Err(AiMuxError::Timeout(format!(
                    "Google file upload polling for {} timed out after {}ms",
                    file.name, poll_timeout_ms
                )));
            }

            sleep_or_abort(
                Duration::try_from_secs_f64(poll_interval_ms / 1000.0)
                    .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?,
                options.abort_signal.as_ref(),
            )
            .await?;

            let poll_url = exchange.url(&format!("/{}", file_poll_path(&file.name)));

            let poll_resp = aimux_provider_utils::get_from_api(
                exchange.with_transport(HttpRequest {
                    url: poll_url.clone(),
                    headers: exchange.headers(),
                    abort_signal: options.abort_signal.clone(),
                    ..Default::default()
                }),
                aimux_provider_utils::create_json_response_handler::<GoogleFileResource>(),
                super::google_failed_response_handler(),
            )
            .await?;

            file = poll_resp.value;
            raw_file = poll_resp.raw_value.clone().unwrap_or(Value::Null);
            last_poll_body = poll_resp.raw_value.map(|value| value.to_string());
            last_poll_status = Some(200);
            last_poll_url = poll_url;
        }

        if file.state == "FAILED" {
            // Provider-declared job failure inside a 2xx envelope: stays ApiCall.
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: last_poll_status,
                provider_code: Some("FAILED".to_string()),
                message: format!("File processing failed for {}", file.name),
                response_body: last_poll_body,
                ..ApiCallError::new(
                    format!("File processing failed for {}", file.name),
                    last_poll_url,
                    serde_json::json!({}),
                )
            })));
        }

        // Build provider metadata.
        let mut metadata = serde_json::Map::new();
        metadata.insert("name".to_string(), json!(file.name));
        if file.display_name.is_some() || raw_file.get("displayName").is_some() {
            metadata.insert("displayName".to_string(), json!(file.display_name));
        }
        metadata.insert("mimeType".to_string(), json!(file.mime_type));
        if file.size_bytes.is_some() || raw_file.get("sizeBytes").is_some() {
            metadata.insert("sizeBytes".to_string(), json!(file.size_bytes));
        }
        metadata.insert("state".to_string(), json!(file.state));
        metadata.insert("uri".to_string(), json!(file.uri));
        if let Some(ref create_time) = file.create_time {
            metadata.insert("createTime".to_string(), json!(create_time));
        }
        if let Some(ref update_time) = file.update_time {
            metadata.insert("updateTime".to_string(), json!(update_time));
        }
        if let Some(ref expiration_time) = file.expiration_time {
            metadata.insert("expirationTime".to_string(), json!(expiration_time));
        }
        if let Some(ref sha256_hash) = file.sha256_hash {
            metadata.insert("sha256Hash".to_string(), json!(sha256_hash));
        }

        let mut provider_ref = HashMap::new();
        provider_ref.insert(GOOGLE.to_string(), file.uri.clone());

        let result_media_type = if file.mime_type.is_empty() {
            Some(options.media_type.clone())
        } else {
            Some(file.mime_type.clone())
        };

        Ok(UploadFileResult {
            provider_reference: provider_ref,
            media_type: result_media_type,
            filename: None,
            provider_metadata: Some(HashMap::from([(GOOGLE.to_string(), metadata)])),
            warnings,
        })
    }
}
