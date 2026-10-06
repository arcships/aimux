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
use aimux_core::shared::{SharedProviderMetadata, SharedProviderOptions, Warning};

use super::options::{GOOGLE, google_options as read_google_options};
use crate::shared::EndpointConfig;

/// Google error structure: `{ "error": { "message": "...", "status": "..." } }`.
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
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct GoogleImageModel {
    model_id: String,
    settings: GoogleImageSettings,
    config: EndpointConfig,
}

impl GoogleImageModel {
    pub(crate) fn from_config(
        model_id: String,
        settings: GoogleImageSettings,
        config: EndpointConfig,
    ) -> Self {
        Self {
            model_id,
            settings,
            config,
        }
    }

    fn predict_path(&self) -> String {
        format!("/models/{}:predict", self.model_id)
    }

    fn generate_content_path(&self) -> String {
        let model_path = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!("/{model_path}:generateContent")
    }

    fn max_images(&self) -> u32 {
        if let Some(max) = self.settings.max_images_per_call {
            return max;
        }
        1
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
            return Err(AiMuxError::UnsupportedFunctionality(
                "Google Gemini API does not support image editing with Imagen models. \
                     Use Google Vertex AI (@ai-sdk/google-vertex) for image editing capabilities."
                    .to_string(),
            ));
        }

        if options.mask.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.predict_path()), options),
            exchange.transform_body(body),
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let response_body: Value = resp.value;

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
            return Err(AiMuxError::UnsupportedFunctionality(
                "Gemini image models do not support mask-based image editing.".to_string(),
            ));
        }

        // Gemini does not support generating multiple images per call via n parameter
        if options.n > 1 {
            return Err(AiMuxError::UnsupportedFunctionality(
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
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
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
        if let Some(google) = read_google_options(Some(&options.provider_options)) {
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(&self.generate_content_path()), options),
            exchange.transform_body(Value::Object(body)),
            aimux_provider_utils::create_json_response_handler(),
            super::google_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let response_body: Value = resp.value;

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
        &self.config.provider
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

/// Parsed Google image provider options.
struct GoogleImageOptions {
    person_generation: Option<String>,
    google_search: Option<Value>,
}

/// Parse Google image provider options from the `"google"` key.
fn parse_google_image_options(provider_options: &SharedProviderOptions) -> GoogleImageOptions {
    let google = read_google_options(Some(provider_options));
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
        .map(std::vec::Vec::len)
        .unwrap_or(0);

    let images: Vec<Value> = (0..predictions).map(|_| json!({})).collect();
    let mut google_meta = Map::new();
    google_meta.insert("images".to_string(), json!(images));
    metadata.insert(GOOGLE.to_string(), google_meta);
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
            .and_then(serde_json::Value::as_u64)
            .map(|x| x as u32);
        let output = u
            .get("candidatesTokenCount")
            .and_then(serde_json::Value::as_u64)
            .map(|x| x as u32);
        let total = u
            .get("totalTokenCount")
            .and_then(serde_json::Value::as_u64)
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
    metadata.insert(GOOGLE.to_string(), google_meta);

    (ImageOutputs::Base64(images), metadata, usage)
}
