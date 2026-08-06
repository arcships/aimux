//! OpenAI Files — implements the `Files` trait for uploading files to the
//! OpenAI API.
//!
//! Aligned with Vercel AI SDK `OpenAIFiles`
//! (`reference/ai/packages/openai/src/files/openai-files.ts`).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData, UploadFileResult};
use aimux_core::shared::FileBytes;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send};

use super::OpenAIConfig;

/// OpenAI provider-specific file upload options.
///
/// Aligned with TS `OpenAIFilesOptions`.
#[derive(Debug, Clone, Default)]
struct OpenAIFilesOptions {
    /// The purpose of the file (defaults to `"assistants"`).
    purpose: Option<String>,
    /// Seconds after which the file expires.
    expires_after: Option<u64>,
}

/// Parse provider options for the OpenAI files provider.
fn parse_openai_files_options(
    provider_options: Option<&HashMap<String, Value>>,
) -> OpenAIFilesOptions {
    let mut opts = OpenAIFilesOptions::default();
    if let Some(po) = provider_options
        && let Some(openai) = po.get("openai")
    {
        if let Some(purpose) = openai.get("purpose").and_then(|v| v.as_str()) {
            opts.purpose = Some(purpose.to_string());
        }
        if let Some(expires_after) = openai.get("expiresAfter").and_then(|v| v.as_u64()) {
            opts.expires_after = Some(expires_after);
        }
    }
    opts
}

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

/// The response from the OpenAI `/files` endpoint.
#[derive(Debug, Deserialize)]
struct OpenAIFilesResponse {
    id: String,
    #[serde(default)]
    bytes: Option<u64>,
    #[serde(default, rename = "created_at")]
    created_at: Option<u64>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "expires_at")]
    expires_at: Option<u64>,
}

/// An OpenAI Files interface for uploading files.
///
/// Aligned with TS `OpenAIFiles`. Does **not** hold an HTTP client — `http::send`
/// uses the process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIFiles {
    config: OpenAIConfig,
}

impl OpenAIFiles {
    pub fn new(config: OpenAIConfig) -> Self {
        Self { config }
    }

    fn build_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(ref project) = self.config.project {
            headers.insert("OpenAI-Project".to_string(), project.clone());
        }
        if let Some(ref extra) = self.config.headers {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/files", self.config.base_url)
    }
}

#[async_trait]
impl Files for OpenAIFiles {
    fn provider(&self) -> &str {
        "openai.files"
    }

    async fn upload_file(
        &self,
        options: &UploadFileCallOptions,
    ) -> Result<UploadFileResult, AiMuxError> {
        let openai_options = parse_openai_files_options(options.provider_options.as_ref());
        let purpose = openai_options
            .purpose
            .unwrap_or_else(|| "assistants".to_string());

        let file_bytes = data_to_bytes(&options.data)?;

        let filename = options
            .filename
            .clone()
            .unwrap_or_else(|| "blob".to_string());

        // Build multipart/form-data body manually.
        let (body, content_type) = build_multipart_form(
            &format!("form-data; name=\"file\"; filename=\"{}\"", filename),
            &options.media_type,
            &file_bytes,
            &[("purpose", purpose.as_str())],
            openai_options
                .expires_after
                .map(|v| ("expires_after", v.to_string())),
        );

        let headers = self.build_headers();
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

        let data: OpenAIFilesResponse = serde_json::from_slice::<OpenAIFilesResponse>(&resp.body)
            .map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Build provider metadata.
        let mut metadata = serde_json::Map::new();
        if let Some(ref filename) = data.filename {
            metadata.insert("filename".to_string(), json!(filename));
        }
        if let Some(ref purpose) = data.purpose {
            metadata.insert("purpose".to_string(), json!(purpose));
        }
        if let Some(bytes) = data.bytes {
            metadata.insert("bytes".to_string(), json!(bytes));
        }
        if let Some(created_at) = data.created_at {
            metadata.insert("createdAt".to_string(), json!(created_at));
        }
        if let Some(ref status) = data.status {
            metadata.insert("status".to_string(), json!(status));
        }
        if let Some(expires_at) = data.expires_at {
            metadata.insert("expiresAt".to_string(), json!(expires_at));
        }

        let mut provider_ref = HashMap::new();
        provider_ref.insert("openai".to_string(), data.id);

        // Filename: prefer response filename, fall back to options filename.
        let result_filename = data.filename.or(options.filename.clone());

        Ok(UploadFileResult {
            provider_reference: provider_ref,
            media_type: Some(options.media_type.clone()),
            filename: result_filename,
            provider_metadata: Some(
                std::iter::once(("openai".to_string(), Value::Object(metadata))).collect(),
            ),
            warnings: Vec::new(),
        })
    }
}

/// Build a multipart/form-data body manually.
///
/// This avoids the `mime_guess` dependency that reqwest's `multipart` feature
/// pulls in (its build script is blocked on some Windows environments).
///
/// - `file_content_disposition`: the full `Content-Disposition` value for the
///   file part (e.g. `form-data; name="file"; filename="test.csv"`).
/// - `file_media_type`: the media type for the file part.
/// - `file_bytes`: the raw file content.
/// - `text_fields`: name/value pairs for simple text fields (e.g. `purpose`).
/// - `extra_text_field`: an optional additional text field.
fn build_multipart_form(
    file_content_disposition: &str,
    file_media_type: &str,
    file_bytes: &[u8],
    text_fields: &[(&str, &str)],
    extra_text_field: Option<(&str, String)>,
) -> (Vec<u8>, String) {
    let boundary = generate_boundary();
    let mut body = Vec::new();

    // Text fields first.
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
