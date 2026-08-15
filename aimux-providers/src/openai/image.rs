//! OpenAI image model — implements the `ImageModel` trait.
//!
//! Aligned with Vercel AI SDK `OpenAIImageModel`
//! (`reference/ai/packages/openai/src/image/openai-image-model.ts`).
//!
//! - Generation: `POST {base_url}/images/generations` (JSON body)
//! - Editing:    `POST {base_url}/images/edits`       (multipart form data)
//!
//! Both endpoints return the same JSON response shape (`openaiImageResponseSchema`).

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult, ImageUsage,
};
use aimux_core::shared::{SharedProviderMetadata, Warning};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send};

use super::OpenAIConfig;

/// Models that default to `b64_json` response format and therefore do NOT need
/// the explicit `response_format` field. Identified by prefix.
const DEFAULT_RESPONSE_FORMAT_PREFIXES: &[&str] = &["chatgpt-image-", "gpt-image-"];

/// Returns `true` if the model ID has a default response format (and thus the
/// `response_format` field should be omitted from the request).
fn has_default_response_format(model_id: &str) -> bool {
    DEFAULT_RESPONSE_FORMAT_PREFIXES
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
}

/// Returns the maximum number of images per call for the given model ID.
///
/// Mirrors TS `getMaxImagesPerCall`:
/// - `dall-e-3` → 1
/// - `dall-e-2`, `gpt-image-*`, `chatgpt-image-*` → 10
/// - unknown models starting with `gpt-image-` → 10
/// - everything else → 1
fn get_max_images_per_call(model_id: &str) -> u32 {
    match model_id {
        "dall-e-3" => 1,
        "dall-e-2"
        | "gpt-image-1"
        | "gpt-image-1-mini"
        | "gpt-image-1.5"
        | "gpt-image-2"
        | "chatgpt-image-latest" => 10,
        _ => {
            if model_id.starts_with("gpt-image-") {
                10
            } else {
                1
            }
        }
    }
}

/// An OpenAI-compatible image generation/editing model.
pub struct OpenAIImageModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIImageModel {
    #[must_use]
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
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
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn generations_endpoint(&self) -> String {
        format!("{}/images/generations", self.config.base_url)
    }

    fn edits_endpoint(&self) -> String {
        format!("{}/images/edits", self.config.base_url)
    }
}

#[async_trait]
impl ImageModel for OpenAIImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_images_per_call(&self) -> Option<u32> {
        Some(get_max_images_per_call(&self.model_id))
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();

        if options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "aspectRatio".to_string(),
                details: Some(
                    "This model does not support aspect ratio. Use `size` instead.".to_string(),
                ),
            });
        }

        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".to_string(),
                details: None,
            });
        }

        let timestamp = chrono::Utc::now().to_rfc3339();
        let headers = self.build_headers(options.headers.as_ref());

        let (body_value, response_headers, _response_body) = if options.files.is_some() {
            // ── Edit path: multipart form data ──
            let openai_options = parse_edit_provider_options(&options.provider_options);
            let (form_body, content_type) =
                build_edit_multipart(&self.model_id, options, &openai_options);

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Post,
                    url: self.edits_endpoint(),
                    headers: build_header_list(&headers),
                    // Content-Type is set by the http layer from the `Bytes` body.
                    body: HttpBody::Bytes(form_body, content_type),

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                self.config.retry_config,
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            let val: Value = serde_json::from_slice(&resp.body)?;
            (val, resp.headers, None)
        } else {
            // ── Generation path: JSON body ──
            let openai_options = parse_generation_provider_options(&options.provider_options);
            let body = build_generation_body(&self.model_id, options, &openai_options);

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Post,
                    url: self.generations_endpoint(),
                    headers: build_header_list(&headers),
                    body: HttpBody::Json(Value::Object(body.clone())),

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                self.config.retry_config,
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            let val: Value = serde_json::from_slice(&resp.body)?;
            (val, resp.headers, Some(Value::Object(body)))
        };

        let images = extract_images(&body_value);
        let usage = extract_usage(&body_value);
        let provider_metadata = extract_provider_metadata(&body_value);

        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: Some(provider_metadata),
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a `HashMap<String, String>` into the `Vec<(String, String)>` header
/// list expected by [`aimux_provider_utils::HttpRequest`].
///
/// `Content-Type` is intentionally not added here: the http layer sets it
/// automatically from the [`HttpBody`] (`Json` → `application/json`,
/// `Bytes` → the supplied content-type).
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// OpenAI generation provider options (camelCase → snake_case mapping).
struct GenerationOptions {
    quality: Option<Value>,
    style: Option<Value>,
    background: Option<Value>,
    moderation: Option<Value>,
    output_format: Option<Value>,
    output_compression: Option<Value>,
    user: Option<Value>,
}

/// Parse generation provider options from the `"openai"` key.
fn parse_generation_provider_options(
    provider_options: &HashMap<String, Value>,
) -> GenerationOptions {
    let openai = provider_options.get("openai");
    GenerationOptions {
        quality: openai.and_then(|o| o.get("quality")).cloned(),
        style: openai.and_then(|o| o.get("style")).cloned(),
        background: openai.and_then(|o| o.get("background")).cloned(),
        moderation: openai.and_then(|o| o.get("moderation")).cloned(),
        output_format: openai.and_then(|o| o.get("outputFormat")).cloned(),
        output_compression: openai.and_then(|o| o.get("outputCompression")).cloned(),
        user: openai.and_then(|o| o.get("user")).cloned(),
    }
}

/// OpenAI edit provider options.
struct EditOptions {
    quality: Option<Value>,
    background: Option<Value>,
    output_format: Option<Value>,
    output_compression: Option<Value>,
    input_fidelity: Option<Value>,
    user: Option<Value>,
}

/// Parse edit provider options from the `"openai"` key.
fn parse_edit_provider_options(provider_options: &HashMap<String, Value>) -> EditOptions {
    let openai = provider_options.get("openai");
    EditOptions {
        quality: openai.and_then(|o| o.get("quality")).cloned(),
        background: openai.and_then(|o| o.get("background")).cloned(),
        output_format: openai.and_then(|o| o.get("outputFormat")).cloned(),
        output_compression: openai.and_then(|o| o.get("outputCompression")).cloned(),
        input_fidelity: openai.and_then(|o| o.get("inputFidelity")).cloned(),
        user: openai.and_then(|o| o.get("user")).cloned(),
    }
}

/// Build the JSON body for `/images/generations`.
///
/// Fields with `None` values are omitted (matching TS behaviour where
/// `undefined` values are stripped by `postJsonToApi`).
fn build_generation_body(
    model_id: &str,
    options: &ImageCallOptions,
    openai: &GenerationOptions,
) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model_id));
    if let Some(ref prompt) = options.prompt {
        body.insert("prompt".to_string(), json!(prompt));
    }
    body.insert("n".to_string(), json!(options.n));
    if let Some(size) = options.size {
        body.insert("size".to_string(), json!(size.to_string()));
    }
    if let Some(ref v) = openai.quality {
        body.insert("quality".to_string(), v.clone());
    }
    if let Some(ref v) = openai.style {
        body.insert("style".to_string(), v.clone());
    }
    if let Some(ref v) = openai.background {
        body.insert("background".to_string(), v.clone());
    }
    if let Some(ref v) = openai.moderation {
        body.insert("moderation".to_string(), v.clone());
    }
    if let Some(ref v) = openai.output_format {
        body.insert("output_format".to_string(), v.clone());
    }
    if let Some(ref v) = openai.output_compression {
        body.insert("output_compression".to_string(), v.clone());
    }
    if let Some(ref v) = openai.user {
        body.insert("user".to_string(), v.clone());
    }
    if !has_default_response_format(model_id) {
        body.insert("response_format".to_string(), json!("b64_json"));
    }
    body
}

/// Decode an `ImageFile` to raw bytes.
fn image_file_to_bytes(file: &ImageFile) -> Result<Vec<u8>, AiMuxError> {
    match file {
        ImageFile::File { data, .. } => match data {
            ImageFileData::Binary(bytes) => Ok(bytes.clone()),
            ImageFileData::Base64(b64) => {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                    .map_err(|e| AiMuxError::InvalidArgument(format!("invalid base64: {e}")))
            }
        },
        ImageFile::Url { .. } => Err(AiMuxError::InvalidArgument(
            "OpenAI image edit does not support URL files in this implementation".to_string(),
        )),
    }
}

/// Get the media type from an `ImageFile`.
fn image_file_media_type(file: &ImageFile) -> &str {
    match file {
        ImageFile::File { media_type, .. } => media_type,
        ImageFile::Url { .. } => "application/octet-stream",
    }
}

/// Build the multipart form-data body for `/images/edits`.
///
/// Returns `(body_bytes, content_type)`.
fn build_edit_multipart(
    model_id: &str,
    options: &ImageCallOptions,
    openai: &EditOptions,
) -> (Vec<u8>, String) {
    let boundary = generate_boundary();
    let mut body = Vec::new();

    // Text fields.
    write_text_field(&mut body, &boundary, "model", model_id);
    if let Some(ref prompt) = options.prompt {
        write_text_field(&mut body, &boundary, "prompt", prompt);
    }
    write_text_field(&mut body, &boundary, "n", &options.n.to_string());
    if let Some(size) = options.size {
        write_text_field(&mut body, &boundary, "size", &size.to_string());
    }
    if let Some(ref v) = openai.quality
        && let Some(s) = v.as_str()
    {
        write_text_field(&mut body, &boundary, "quality", s);
    }
    if let Some(ref v) = openai.background
        && let Some(s) = v.as_str()
    {
        write_text_field(&mut body, &boundary, "background", s);
    }
    if let Some(ref v) = openai.output_format
        && let Some(s) = v.as_str()
    {
        write_text_field(&mut body, &boundary, "output_format", s);
    }
    if let Some(ref v) = openai.output_compression {
        write_text_field(&mut body, &boundary, "output_compression", &v.to_string());
    }
    if let Some(ref v) = openai.input_fidelity
        && let Some(s) = v.as_str()
    {
        write_text_field(&mut body, &boundary, "input_fidelity", s);
    }
    if let Some(ref v) = openai.user
        && let Some(s) = v.as_str()
    {
        write_text_field(&mut body, &boundary, "user", s);
    }

    // Image file(s).
    let files = options.files.as_ref().unwrap();
    if files.len() == 1 {
        let file = &files[0];
        let media_type = image_file_media_type(file);
        let bytes = image_file_to_bytes(file).unwrap_or_default();
        write_file_field(&mut body, &boundary, "image", "image", media_type, &bytes);
    } else {
        for file in files {
            let media_type = image_file_media_type(file);
            let bytes = image_file_to_bytes(file).unwrap_or_default();
            write_file_field(&mut body, &boundary, "image[]", "image", media_type, &bytes);
        }
    }

    // Mask (optional).
    if let Some(ref mask) = options.mask {
        let media_type = image_file_media_type(mask);
        let bytes = image_file_to_bytes(mask).unwrap_or_default();
        write_file_field(&mut body, &boundary, "mask", "mask", media_type, &bytes);
    }

    // Closing boundary.
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    let content_type = format!("multipart/form-data; boundary={boundary}");
    (body, content_type)
}

fn write_text_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn write_file_field(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    filename: &str,
    media_type: &str,
    bytes: &[u8],
) {
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {media_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
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

/// Extract base64 images from the response.
fn extract_images(response: &Value) -> ImageOutputs {
    let images: Vec<String> = response
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("b64_json")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    ImageOutputs::Base64(images)
}

/// Extract usage information from the response.
fn extract_usage(response: &Value) -> Option<ImageUsage> {
    response.get("usage").map(|u| ImageUsage {
        input_tokens: u
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .map(|x| x as u32),
        output_tokens: u
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .map(|x| x as u32),
        total_tokens: u
            .get("total_tokens")
            .and_then(serde_json::Value::as_u64)
            .map(|x| x as u32),
    })
}

/// Distribute input token details evenly across images, with the remainder
/// assigned to the last image so that summing across all entries gives the
/// exact total.
///
/// Mirrors TS `distributeTokenDetails`.
fn distribute_token_details(details: Option<&Value>, index: usize, total: usize) -> Option<Value> {
    let details = details?;
    let mut result = Map::new();

    if let Some(image_tokens) = details
        .get("image_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        let total = total as u64;
        let base = image_tokens / total;
        let remainder = image_tokens - base * (total - 1);
        let value = if index == total as usize - 1 {
            remainder
        } else {
            base
        };
        result.insert("imageTokens".to_string(), json!(value));
    }

    if let Some(text_tokens) = details
        .get("text_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        let total = total as u64;
        let base = text_tokens / total;
        let remainder = text_tokens - base * (total - 1);
        let value = if index == total as usize - 1 {
            remainder
        } else {
            base
        };
        result.insert("textTokens".to_string(), json!(value));
    }

    if result.is_empty() {
        None
    } else {
        Some(Value::Object(result))
    }
}

/// Extract provider metadata from the response.
///
/// Builds `{ "openai": { "images": [ ... ] } }` where each entry includes
/// `revisedPrompt` (if present), `created`, `size`, `quality`, `background`,
/// `outputFormat`, and distributed token details. Fields with null/absent
/// values are omitted (matching JSON serialization of TS `undefined`).
fn extract_provider_metadata(response: &Value) -> SharedProviderMetadata {
    let mut metadata = HashMap::new();

    let data = response.get("data").and_then(|d| d.as_array());
    let created = response.get("created").and_then(serde_json::Value::as_u64);
    let size = response.get("size").and_then(|v| v.as_str());
    let quality = response.get("quality").and_then(|v| v.as_str());
    let background = response.get("background").and_then(|v| v.as_str());
    let output_format = response.get("output_format").and_then(|v| v.as_str());
    let input_tokens_details = response
        .get("usage")
        .and_then(|u| u.get("input_tokens_details"));

    let images: Vec<Value> = if let Some(arr) = data {
        let total = arr.len();
        arr.iter()
            .enumerate()
            .map(|(index, item)| {
                let mut entry = Map::new();
                if let Some(rp) = item.get("revised_prompt").and_then(|v| v.as_str()) {
                    entry.insert("revisedPrompt".to_string(), json!(rp));
                }
                if let Some(c) = created {
                    entry.insert("created".to_string(), json!(c));
                }
                if let Some(s) = size {
                    entry.insert("size".to_string(), json!(s));
                }
                if let Some(q) = quality {
                    entry.insert("quality".to_string(), json!(q));
                }
                if let Some(b) = background {
                    entry.insert("background".to_string(), json!(b));
                }
                if let Some(of) = output_format {
                    entry.insert("outputFormat".to_string(), json!(of));
                }
                if let Some(token_details) =
                    distribute_token_details(input_tokens_details, index, total)
                    && let Value::Object(td) = token_details
                {
                    for (k, v) in td {
                        entry.insert(k, v);
                    }
                }
                Value::Object(entry)
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut openai_meta = Map::new();
    openai_meta.insert("images".to_string(), json!(images));
    metadata.insert("openai".to_string(), Value::Object(openai_meta));
    metadata
}
