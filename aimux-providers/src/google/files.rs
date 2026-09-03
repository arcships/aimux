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
use aimux_core::shared::FileBytes;
use aimux_core::types::Warning;

use aimux_provider_utils::{HttpBody, HttpRequest, sleep_or_abort};

use super::GoogleConfig;

/// Google-specific error structure: `{ "error": { "message": "..." } }`.
/// Google provider-specific file upload options.
#[derive(Debug, Clone, Default)]
struct GoogleFilesUploadOptions {
    display_name: Option<String>,
    poll_interval_ms: Option<u64>,
    poll_timeout_ms: Option<u64>,
}

/// Parse provider options for the Google files provider.
fn parse_google_files_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> GoogleFilesUploadOptions {
    let mut opts = GoogleFilesUploadOptions::default();
    if let Some(po) = provider_options
        && let Some(google) = po.get("google")
    {
        if let Some(display_name) = google.get("displayName").and_then(|v| v.as_str()) {
            opts.display_name = Some(display_name.to_string());
        }
        if let Some(interval) = google
            .get("pollIntervalMs")
            .and_then(serde_json::Value::as_u64)
        {
            opts.poll_interval_ms = Some(interval);
        }
        if let Some(timeout) = google
            .get("pollTimeoutMs")
            .and_then(serde_json::Value::as_u64)
        {
            opts.poll_timeout_ms = Some(timeout);
        }
    }
    opts
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
    config: GoogleConfig,
}

impl GoogleFiles {
    #[must_use]
    pub fn new(config: GoogleConfig) -> Self {
        Self { config }
    }

    fn build_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-goog-api-key".to_string(), self.config.api_key.clone());
        headers
    }

    /// The base origin is `base_url` with `/v1beta` stripped.
    fn base_origin(&self) -> String {
        self.config.base_url.replace("/v1beta", "")
    }

    fn init_endpoint(&self) -> String {
        format!("{}/upload/v1beta/files", self.base_origin())
    }
}

#[async_trait]
impl Files for GoogleFiles {
    fn provider(&self) -> &str {
        "google.generative-ai"
    }

    async fn upload_file(
        &self,
        options: &UploadFileCallOptions,
    ) -> Result<UploadFileResult, AiMuxError> {
        let google_options = parse_google_files_options(options.provider_options.as_ref());
        let resolved_headers = self.build_headers();

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

        let mut init_headers: Vec<(String, String)> = resolved_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
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

        // Nothing above `upload_file` retries it (§9.4), so the retry lives
        // here — per stage, not around the whole upload: a failure in the
        // upload or poll stage must not replay the init exchange that minted
        // `upload_url`, nor resend the file body.
        let retries = aimux_core::retry::prepare_retries(
            None,
            self.config.retry_config,
            options.abort_signal.clone(),
        );
        let init_resp = retries
            .retry(|| async {
                aimux_provider_utils::post_json_to_api(
                    HttpRequest {
                        url: self.init_endpoint(),
                        headers: init_headers.clone(),

                        abort_signal: options.abort_signal.clone(),
                        call_id: None,
                        recording_context: None,
                        response_timeout: None,
                        max_json_response_bytes: None,
                        validate_url: false,
                        trusted_origin: None,
                        credentialed_origin: None,
                    },
                    init_body_value.clone(),
                    aimux_provider_utils::ResponseHandler::new(|input| async move {
                        let headers = aimux_provider_utils::extract_response_headers::extract_response_headers(
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
                // Per attempt, so a `RetryError`'s saved `errors` carry it too.
                .map_err(|e| match e {
                    AiMuxError::ApiCall(d) => AiMuxError::ApiCall(Box::new(ApiCallError {
                        message: format!("Failed to initiate resumable upload: {}", d.message),
                        ..*d
                    })),
                    e => e,
                })
            })
            .await?;

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
        let upload_resp = retries
            .retry(|| async {
                aimux_provider_utils::post_to_api(
                    HttpRequest {
                        url: upload_url.clone(),
                        headers: upload_headers.clone(),

                        abort_signal: options.abort_signal.clone(),
                        call_id: None,
                        recording_context: None,
                        response_timeout: None,
                        max_json_response_bytes: None,
                        validate_url: true,
                        trusted_origin: Some(self.config.base_url.clone()),
                        credentialed_origin: Some(self.config.base_url.clone()),
                    },
                    HttpBody::Bytes(file_bytes.clone(), media_type.clone()),
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
                })
            })
            .await?;

        let mut file = upload_resp.value.file;

        // Step 3: Poll if file is PROCESSING.
        let poll_interval_ms = google_options.poll_interval_ms.unwrap_or(2000);
        let poll_timeout_ms = google_options.poll_timeout_ms.unwrap_or(300000);
        let start_time = Instant::now();
        let poll_header_list: Vec<(String, String)> = resolved_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Seed evidence from the upload response so a file that is already
        // FAILED (never polled) still carries the observed status + raw body.
        let mut last_poll_body = upload_resp.raw_value.map(|value| value.to_string());
        let mut last_poll_status: Option<u16> = Some(200);
        let mut last_poll_url = upload_request_url;
        while file.state == "PROCESSING" {
            if start_time.elapsed() > Duration::from_millis(poll_timeout_ms) {
                return Err(AiMuxError::Timeout(format!(
                    "Google file upload polling for {} timed out after {}ms",
                    file.name, poll_timeout_ms
                )));
            }

            sleep_or_abort(
                Duration::from_millis(poll_interval_ms),
                options.abort_signal.as_ref(),
            )
            .await?;

            let poll_url = format!("{}/{}", self.config.base_url, file.name);

            let poll_resp = retries
                .retry(|| {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: poll_url.clone(),
                            headers: poll_header_list.clone(),

                            abort_signal: options.abort_signal.clone(),
                            call_id: None,
                            recording_context: None,
                            response_timeout: None,
                            max_json_response_bytes: None,
                            validate_url: false,
                            trusted_origin: None,
                            credentialed_origin: None,
                        },
                        aimux_provider_utils::create_json_response_handler::<GoogleFileResource>(),
                        super::google_failed_response_handler(),
                    )
                })
                .await?;

            file = poll_resp.value;
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
        metadata.insert("displayName".to_string(), json!(file.display_name));
        metadata.insert("mimeType".to_string(), json!(file.mime_type));
        metadata.insert("sizeBytes".to_string(), json!(file.size_bytes));
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
        provider_ref.insert("google".to_string(), file.uri.clone());

        let result_media_type = if file.mime_type.is_empty() {
            Some(options.media_type.clone())
        } else {
            Some(file.mime_type.clone())
        };

        Ok(UploadFileResult {
            provider_reference: provider_ref,
            media_type: result_media_type,
            filename: None,
            provider_metadata: Some(
                std::iter::once(("google".to_string(), Value::Object(metadata))).collect(),
            ),
            warnings,
        })
    }
}
