//! Stability image provider (image modality only).
//!
//! Aligned with the Stability AI Stable Image generate API
//! (`https://api.stability.ai/v2beta/stable-image/generate/{ultra|core|sd3}`).
//!
//! Text-to-image generation via `multipart/form-data`. The model ID selects the
//! endpoint sub-path: `stable-image-ultra` → `/generate/ultra`,
//! `stable-image-core` → `/generate/core`, `sd3` → `/generate/sd3`. Any other
//! model ID is used as-is for the sub-path. Responses are image binary
//! (`Accept: image/*`) or base64-encoded JSON.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_core::{LanguageModel, Provider};
use aimux_provider_utils::response::{ErrorStructure, parse_provider_error};
use aimux_provider_utils::{MultipartForm, load_api_key, without_trailing_slash};

/// Stability error response structure: `{ "id": "...", "name": "...", "errors": ["..."] }`.
///
/// The human-readable summary lives in the `name` field (e.g. `"unauthorized"`,
/// `"bad_request"`); the detailed messages are in the `errors` array, which the
/// shared error parser cannot index into, so we surface `name` as the message.
const STABILITY_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["name"],
    type_path: &["name"],
};

/// Map a Stability model ID to its generate-endpoint sub-path.
///
/// Known canonical IDs are mapped to their short path; any other (still
/// legitimate) ID is passed through unchanged so the provider degrades safely
/// instead of panicking.
fn model_id_to_subpath(model_id: &str) -> &str {
    match model_id {
        "stable-image-ultra" => "ultra",
        "stable-image-core" => "core",
        "sd3" => "sd3",
        other => other,
    }
}

/// Configuration for the Stability provider.
#[derive(Debug, Clone)]
pub struct StabilityConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl StabilityConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.stability.ai".to_string(),
            headers: None,
        }
    }
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "STABILITY_API_KEY", "Stability")?;
        Ok(Self::new(api_key))
    }
}

pub struct StabilityProvider {
    config: StabilityConfig,
    client: Client,
}

impl StabilityProvider {
    pub fn new(config: StabilityConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
    pub fn image(&self, model_id: &str) -> StabilityImageModel {
        StabilityImageModel::new(
            model_id.to_string(),
            self.config.clone(),
            self.client.clone(),
        )
    }
}

impl Provider for StabilityProvider {
    fn name(&self) -> &str {
        "stability"
    }
    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(
            "Stability is an image-only provider and does not support language models.".into(),
        ))
    }
}

/// A Stability image generation model.
pub struct StabilityImageModel {
    model_id: String,
    config: StabilityConfig,
    client: Client,
}

impl StabilityImageModel {
    pub fn new(model_id: String, config: StabilityConfig, client: Client) -> Self {
        Self {
            model_id,
            config,
            client,
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert(
            "Authorization".into(),
            format!("Bearer {}", self.config.api_key),
        );
        // Request the image binary directly; a caller may override this with
        // `application/json` via config/extra headers to receive base64 JSON.
        h.insert("Accept".into(), "image/*".into());
        if let Some(ref ch) = self.config.headers {
            for (k, v) in ch {
                h.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.insert(k.clone(), v.clone());
            }
        }
        h
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v2beta/stable-image/generate/{}",
            self.config.base_url,
            model_id_to_subpath(&self.model_id)
        )
    }
}

/// Greatest common divisor of two `u32` values.
fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Derive an aspect-ratio string (`"W:H"`) from a pixel [`Size`], if possible.
fn convert_size_to_aspect_ratio(size: &aimux_core::shared::Size) -> Option<String> {
    let (w, h) = (size.width(), size.height());
    if w == 0 || h == 0 {
        return None;
    }
    let g = gcd(w, h);
    Some(format!("{}:{}", w / g, h / g))
}

#[async_trait]
impl ImageModel for StabilityImageModel {
    fn provider(&self) -> &str {
        "stability"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        let stability_opts = options.provider_options.get("stability");

        // Resolve aspect ratio: explicit option > provider option > derived from size.
        let aspect_ratio: Option<String> = if let Some(ar) = options.aspect_ratio {
            Some(ar.to_string())
        } else if let Some(ar) = stability_opts
            .and_then(|o| o.get("aspect_ratio"))
            .and_then(|v| v.as_str())
            .map(String::from)
        {
            Some(ar)
        } else if let Some(size) = options.size {
            if let Some(ar) = convert_size_to_aspect_ratio(&size) {
                warnings.push(Warning::Unsupported {
                    feature: "size".into(),
                    details: Some(
                        "Deriving aspect_ratio from size. Stability uses aspect_ratio, not \
                         explicit width/height, for these models."
                            .into(),
                    ),
                });
                Some(ar)
            } else {
                None
            }
        } else {
            None
        };

        // Resolve seed: standard option > provider option.
        let seed = options.seed.or_else(|| {
            stability_opts
                .and_then(|o| o.get("seed"))
                .and_then(|v| v.as_u64())
        });

        // Resolve output format (defaults to png).
        let output_format = stability_opts
            .and_then(|o| o.get("output_format"))
            .and_then(|v| v.as_str())
            .unwrap_or("png");

        // Build the multipart/form-data body.
        let mut form = MultipartForm::new();
        if let Some(ref p) = options.prompt {
            form.text("prompt", p);
        }
        if let Some(neg) = stability_opts
            .and_then(|o| o.get("negative_prompt"))
            .and_then(|v| v.as_str())
        {
            form.text("negative_prompt", neg);
        }
        if let Some(ref ar) = aspect_ratio {
            form.text("aspect_ratio", ar);
        }
        if let Some(seed) = seed {
            form.text("seed", &seed.to_string());
        }
        form.text("output_format", output_format);

        // Forward remaining stability provider options as scalar form fields.
        if let Some(stab) = stability_opts.and_then(|v| v.as_object()) {
            for (k, v) in stab {
                if matches!(
                    k.as_str(),
                    "negative_prompt" | "aspect_ratio" | "seed" | "output_format"
                ) {
                    continue;
                }
                let text = match v {
                    Value::String(s) => s.clone(),
                    Value::Number(_) | Value::Bool(_) => v.to_string(),
                    // Skip complex values; multipart scalar fields only.
                    _ => continue,
                };
                form.text(k, &text);
            }
        }

        let (body_bytes, content_type) = form.finish();

        let headers = self.build_headers(options.headers.as_ref());
        let hm: reqwest::header::HeaderMap = headers
            .iter()
            .filter_map(|(k, v)| {
                reqwest::header::HeaderName::try_from(k)
                    .ok()
                    .zip(reqwest::header::HeaderValue::try_from(v).ok())
            })
            .collect();

        let resp = self
            .client
            .post(self.endpoint())
            .header("Content-Type", &content_type)
            .headers(hm)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?;

        let status = resp.status();
        let rh: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(parse_provider_error(
                status.as_u16(),
                &t,
                &STABILITY_ERROR_STRUCTURE,
            ));
        }

        let response_content_type = rh.get("content-type").cloned().unwrap_or_default();

        // The image is returned either as raw binary (Accept: image/*) or as
        // base64 inside a JSON body (Accept: application/json).
        let image_bytes = if response_content_type.starts_with("application/json") {
            let body = resp
                .bytes()
                .await
                .map_err(|e| AiMuxError::Http(e.to_string()))?
                .to_vec();
            let v: Value = serde_json::from_slice(&body).map_err(AiMuxError::Json)?;
            let b64 = v.get("image").and_then(|i| i.as_str()).ok_or_else(|| {
                AiMuxError::Provider("Stability response missing `image` field".into())
            })?;
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                .map_err(|e| AiMuxError::Provider(format!("invalid base64 image: {e}")))?
        } else {
            resp.bytes()
                .await
                .map_err(|e| AiMuxError::Http(e.to_string()))?
                .to_vec()
        };

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: None,
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }
}
