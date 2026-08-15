//! Anthropic Files — implements the `Files` trait for uploading files to the
//! Anthropic API.
//!
//! Aligned with Vercel AI SDK `AnthropicFiles`
//! (`reference/ai/packages/anthropic/src/anthropic-files.ts`).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData, UploadFileResult};
use aimux_core::shared::FileBytes;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send};

use super::AnthropicConfig;

/// The beta header value for the Anthropic Files API.
const FILES_BETA_HEADER: &str = "files-api-2025-04-14";

/// Convert `UploadFileData` to raw bytes.
///
/// Mirrors the TS `convertInlineFileDataToUint8Array`.
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

/// The response from the Anthropic `/v1/files` endpoint.
#[derive(Debug, Deserialize)]
struct AnthropicFilesResponse {
    id: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default, rename = "mime_type")]
    mime_type: Option<String>,
    #[serde(default, rename = "size_bytes")]
    size_bytes: Option<u64>,
    #[serde(default, rename = "created_at")]
    created_at: Option<String>,
    #[serde(default)]
    downloadable: Option<bool>,
}

/// An Anthropic Files interface for uploading files.
///
/// Aligned with TS `AnthropicFiles`. Does **not** hold an HTTP client —
/// `http::send` uses the process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct AnthropicFiles {
    config: AnthropicConfig,
}

impl AnthropicFiles {
    #[must_use]
    pub fn new(config: AnthropicConfig) -> Self {
        Self { config }
    }

    fn build_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "anthropic-version".to_string(),
            self.config.api_version.clone(),
        );
        // Auth: prefer bearer token, fall back to x-api-key.
        if let Some(token) = &self.config.auth_token {
            headers.insert("authorization".to_string(), format!("Bearer {token}"));
        } else {
            headers.insert("x-api-key".to_string(), self.config.api_key.clone());
        }
        // Extra config-level headers.
        if let Some(cfg_headers) = &self.config.headers {
            for (k, v) in cfg_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/files", self.config.base_url)
    }
}

#[async_trait]
impl Files for AnthropicFiles {
    fn provider(&self) -> &str {
        "anthropic.files"
    }

    async fn upload_file(
        &self,
        options: &UploadFileCallOptions,
    ) -> Result<UploadFileResult, AiMuxError> {
        let file_bytes = data_to_bytes(&options.data)?;
        let filename = options
            .filename
            .clone()
            .unwrap_or_else(|| "blob".to_string());

        // Build multipart/form-data body manually.
        let (body, content_type) = build_multipart_form(
            &format!("form-data; name=\"file\"; filename=\"{filename}\""),
            &options.media_type,
            &file_bytes,
            &[],
            None,
        );

        let mut headers = self.build_headers();
        headers.insert("anthropic-beta".to_string(), FILES_BETA_HEADER.to_string());
        let header_list: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // `send()` returns Ok only for 2xx; non-2xx responses are mapped to an
        // error internally using the shared error structure. The multipart body
        // and its content-type are carried by `HttpBody::Bytes` — the HTTP layer
        // sets `Content-Type` from it, so it is intentionally not added to the
        // header list above.
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: header_list,
                body: HttpBody::Bytes(body, content_type),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let data: AnthropicFilesResponse =
            serde_json::from_slice::<AnthropicFilesResponse>(&resp.body)?;

        // Build provider metadata.
        let mut metadata = serde_json::Map::new();
        if let Some(ref filename) = data.filename {
            metadata.insert("filename".to_string(), json!(filename));
        }
        if let Some(ref mime_type) = data.mime_type {
            metadata.insert("mimeType".to_string(), json!(mime_type));
        }
        if let Some(size_bytes) = data.size_bytes {
            metadata.insert("sizeBytes".to_string(), json!(size_bytes));
        }
        if let Some(ref created_at) = data.created_at {
            metadata.insert("createdAt".to_string(), json!(created_at));
        }
        if let Some(downloadable) = data.downloadable {
            metadata.insert("downloadable".to_string(), json!(downloadable));
        }

        let mut provider_ref = HashMap::new();
        provider_ref.insert("anthropic".to_string(), data.id);

        let result_media_type = data
            .mime_type
            .clone()
            .unwrap_or_else(|| options.media_type.clone());
        let result_filename = data.filename.or(options.filename.clone());

        Ok(UploadFileResult {
            provider_reference: provider_ref,
            media_type: Some(result_media_type),
            filename: result_filename,
            provider_metadata: Some(
                std::iter::once(("anthropic".to_string(), Value::Object(metadata))).collect(),
            ),
            warnings: Vec::new(),
        })
    }
}

/// Build a multipart/form-data body manually.
///
/// This avoids the `mime_guess` dependency that reqwest's `multipart` feature
/// pulls in (its build script is blocked on some Windows environments).
fn build_multipart_form(
    file_content_disposition: &str,
    file_media_type: &str,
    file_bytes: &[u8],
    text_fields: &[(&str, &str)],
    extra_text_field: Option<(&str, String)>,
) -> (Vec<u8>, String) {
    let boundary = generate_boundary();
    let mut body = Vec::new();

    // Text fields.
    for (name, value) in text_fields {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // Optional extra text field.
    if let Some((name, value)) = extra_text_field {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // File part.
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        format!("Content-Disposition: {file_content_disposition}\r\n").as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {file_media_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");

    // Closing boundary.
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    let content_type = format!("multipart/form-data; boundary={boundary}");
    (body, content_type)
}

/// Generate a random-ish multipart boundary string.
fn generate_boundary() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("----WebKitFormBoundary{nanos:025x}")
}
