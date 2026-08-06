//! Google image model — implements the `ImageModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleImageModel`
//! (`reference/ai/packages/google/src/google-image-model.ts`).
//!
//! Two code paths:
//! - **Imagen** models (non-`gemini-*`): `POST {base_url}/models/{id}:predict`
//! - **Gemini** image models (`gemini-*`): `POST {base_url}/models/{id}:generateContent`

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult, ImageUsage,
};
use aimux_core::shared::{SharedProviderMetadata, Warning};

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::GoogleConfig;

/// Google error structure: `{ "error": { "message": "...", "status": "..." } }`.
const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

/// Returns `true` if the model ID is a Gemini image model.
fn is_gemini_model(model_id: &str) -> bool {
    model_id.starts_with("gemini-")
}

/// Settings for the Google image model.
#[derive(Debug, Clone, Default)]
pub struct GoogleImageSettings {
    /// Override the maximum number of images per call.
    pub max_images_per_call: Option<u32>,
}

/// A Google image generation model (Imagen or Gemini).
///
/// Does **not** hold an HTTP client — `http::send` uses the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleImageModel {
    model_id: String,
    settings: GoogleImageSettings,
    config: GoogleConfig,
}

impl GoogleImageModel {
    pub fn new(model_id: String, settings: GoogleImageSettings, config: GoogleConfig) -> Self {
        Self {
            model_id,
            settings,
            config,
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-goog-api-key".to_string(), self.config.api_key.clone());
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn predict_endpoint(&self) -> String {
        format!("{}/models/{}:predict", self.config.base_url, self.model_id)
    }

    fn generate_content_endpoint(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!("{}/{}:generateContent", self.config.base_url, model_path)
    }

    fn max_images(&self) -> u32 {
        if let Some(max) = self.settings.max_images_per_call {
            return max;
        }
        if is_gemini_model(&self.model_id) {
            10
        } else {
            4
        }
    }

    // ── Imagen path ─────────────────────────────────────────────────────────

    async fn do_generate_imagen(
        &self,
        options: &ImageCallOptions,
    ) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();

        // Imagen API endpoints do not support image editing
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            return Err(AiMuxError::Unsupported(
                "Google Gemini API does not support image editing with Imagen models. \
                     Use Google Vertex AI (@ai-sdk/google-vertex) for image editing capabilities."
                    .to_string(),
            ));
        }

        if options.mask.is_some() {
            return Err(AiMuxError::Unsupported(
                "Google Gemini API does not support image editing with masks. \
                 Use Google Vertex AI (@ai-sdk/google-vertex) for image editing capabilities."
                    .to_string(),
            ));
        }

        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".to_string(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .to_string(),
                ),
            });
        }

        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".to_string(),
                details: Some(
                    "This model does not support the `seed` option through this provider."
                        .to_string(),
                ),
            });
        }

        let google_options = parse_google_image_options(&options.provider_options);

        let mut parameters = Map::new();
        parameters.insert("sampleCount".to_string(), json!(options.n));

        if let Some(ar) = options.aspect_ratio {
            parameters.insert("aspectRatio".to_string(), json!(ar.to_string()));
        }

        if let Some(pg) = google_options.person_generation {
            parameters.insert("personGeneration".to_string(), json!(pg));
        }

        if google_options.google_search.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "googleSearch".to_string(),
                details: Some(
                    "Google Search grounding is only supported on Gemini image models.".to_string(),
                ),
            });
        }

        let body = json!({
            "instances": [{ "prompt": options.prompt }],
            "parameters": parameters,
        });

        let headers = self.build_headers(options.headers.as_ref());
        let header_list = build_header_list(&headers);

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.predict_endpoint(),
                headers: header_list,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let response_body: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("invalid JSON response: {e}")))?;

        let images = extract_imagen_images(&response_body);
        let provider_metadata = extract_imagen_metadata(&response_body);

        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: Some(provider_metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage: None,
        })
    }

    // ── Gemini path ─────────────────────────────────────────────────────────

    async fn do_generate_gemini(
        &self,
        options: &ImageCallOptions,
    ) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();

        // Gemini does not support mask-based inpainting
        if options.mask.is_some() {
            return Err(AiMuxError::Unsupported(
                "Gemini image models do not support mask-based image editing.".to_string(),
            ));
        }

        // Gemini does not support generating multiple images per call via n parameter
        if options.n > 1 {
            return Err(AiMuxError::Unsupported(
                "Gemini image models do not support generating a set number of images per call. \
                 Use n=1 or omit the n parameter."
                    .to_string(),
            ));
        }

        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".to_string(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .to_string(),
                ),
            });
        }

        let google_options = parse_google_image_options(&options.provider_options);

        // Build contents parts
        let mut parts: Vec<Value> = Vec::new();
        if let Some(ref prompt) = options.prompt {
            parts.push(json!({ "text": prompt }));
        }

        if let Some(ref files) = options.files {
            for file in files {
                match file {
                    ImageFile::Url { url } => {
                        return Err(AiMuxError::Unsupported(format!(
                            "URL-based input images with media type \"image/*\" are not passed as \
                             inline bytes. URL: {url}"
                        )));
                    }
                    ImageFile::File { media_type, data } => {
                        let data_str = match data {
                            ImageFileData::Base64(b64) => b64.clone(),
                            ImageFileData::Binary(bytes) => base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                bytes,
                            ),
                        };
                        parts.push(json!({
                            "inlineData": {
                                "mimeType": media_type,
                                "data": data_str,
                            }
                        }));
                    }
                }
            }
        }

        let contents = json!([{ "role": "user", "parts": parts }]);

        // Build generationConfig
        let mut generation_config = Map::new();
        generation_config.insert("responseModalities".to_string(), json!(["IMAGE"]));

        if let Some(ar) = options.aspect_ratio {
            generation_config.insert(
                "imageConfig".to_string(),
                json!({ "aspectRatio": ar.to_string() }),
            );
        }

        if let Some(seed) = options.seed {
            generation_config.insert("seed".to_string(), json!(seed));
        }

        // Passthrough provider options (excluding googleSearch)
        if let Some(google) = options
            .provider_options
            .get("google")
            .and_then(|v| v.as_object())
        {
            for (key, value) in google {
                if key == "googleSearch" || key == "personGeneration" || key == "aspectRatio" {
                    continue;
                }
                generation_config.insert(key.clone(), value.clone());
            }
        }

        let mut body = Map::new();
        body.insert("contents".to_string(), contents);
        body.insert(
            "generationConfig".to_string(),
            Value::Object(generation_config),
        );

        // Tools (googleSearch)
        if let Some(ref gs) = google_options.google_search {
            body.insert("tools".to_string(), json!([{ "googleSearch": gs }]));
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_list = build_header_list(&headers);

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generate_content_endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let response_body: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("invalid JSON response: {e}")))?;

        let (images, provider_metadata, usage) = extract_gemini_result(&response_body);

        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: Some(provider_metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage,
        })
    }
}

#[async_trait]
impl ImageModel for GoogleImageModel {
    fn provider(&self) -> &str {
        "google.generative-ai"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_images_per_call(&self) -> Option<u32> {
        Some(self.max_images())
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        if is_gemini_model(&self.model_id) {
            self.do_generate_gemini(options).await
        } else {
            self.do_generate_imagen(options).await
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

/// Parsed Google image provider options.
struct GoogleImageOptions {
    person_generation: Option<String>,
    google_search: Option<Value>,
}

/// Parse Google image provider options from the `"google"` key.
fn parse_google_image_options(provider_options: &HashMap<String, Value>) -> GoogleImageOptions {
    let google = provider_options.get("google");
    GoogleImageOptions {
        person_generation: google
            .and_then(|g| g.get("personGeneration"))
            .and_then(|v| v.as_str())
            .map(String::from),
        google_search: google.and_then(|g| g.get("googleSearch")).cloned(),
    }
}

/// Extract base64 images from an Imagen response.
fn extract_imagen_images(response: &Value) -> ImageOutputs {
    let images: Vec<String> = response
        .get("predictions")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    p.get("bytesBase64Encoded")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    ImageOutputs::Base64(images)
}

/// Extract provider metadata from an Imagen response.
fn extract_imagen_metadata(response: &Value) -> SharedProviderMetadata {
    let mut metadata = HashMap::new();
    let predictions = response
        .get("predictions")
        .and_then(|p| p.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let images: Vec<Value> = (0..predictions).map(|_| json!({})).collect();
    let mut google_meta = Map::new();
    google_meta.insert("images".to_string(), json!(images));
    metadata.insert("google".to_string(), Value::Object(google_meta));
    metadata
}

/// Extract images, provider metadata, and usage from a Gemini generateContent response.
fn extract_gemini_result(
    response: &Value,
) -> (ImageOutputs, SharedProviderMetadata, Option<ImageUsage>) {
    let mut images: Vec<String> = Vec::new();
    let mut grounding_metadata: Option<Value> = None;

    if let Some(candidates) = response.get("candidates").and_then(|c| c.as_array()) {
        for candidate in candidates {
            // Extract grounding metadata
            if let Some(gm) = candidate.get("groundingMetadata") {
                grounding_metadata = Some(gm.clone());
            }
            // Extract images from content parts
            if let Some(parts) = candidate
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
            {
                for part in parts {
                    if let Some(inline_data) = part.get("inlineData")
                        && let Some(mime_type) =
                            inline_data.get("mimeType").and_then(|m| m.as_str())
                        && mime_type.starts_with("image/")
                        && let Some(data) = inline_data.get("data").and_then(|d| d.as_str())
                    {
                        images.push(data.to_string());
                    }
                }
            }
        }
    }

    // Usage
    let usage = response.get("usageMetadata").map(|u| {
        let input = u
            .get("promptTokenCount")
            .and_then(|v| v.as_u64())
            .map(|x| x as u32);
        let output = u
            .get("candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .map(|x| x as u32);
        let total = u
            .get("totalTokenCount")
            .and_then(|v| v.as_u64())
            .map(|x| x as u32);
        ImageUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: total.or_else(|| Some(input.unwrap_or(0) + output.unwrap_or(0))),
        }
    });

    // Provider metadata
    let mut metadata = HashMap::new();
    let mut google_meta = Map::new();
    let image_metas: Vec<Value> = images.iter().map(|_| json!({})).collect();
    google_meta.insert("images".to_string(), json!(image_metas));
    if let Some(gm) = grounding_metadata {
        google_meta.insert("groundingMetadata".to_string(), gm);
    }
    metadata.insert("google".to_string(), Value::Object(google_meta));

    (ImageOutputs::Base64(images), metadata, usage)
}
