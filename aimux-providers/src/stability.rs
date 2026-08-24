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
use bytes::Bytes;
use serde_json::Value;

use aimux_core::Provider;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::{
    HttpBody, HttpRequest, MultipartForm, load_api_key, without_trailing_slash,
};

/// Stability error response structure: `{ "id": "...", "name": "...", "errors": ["..."] }`.
///
/// The human-readable summary lives in the `name` field (e.g. `"unauthorized"`,
/// `"bad_request"`); the detailed messages are in the `errors` array, which the
/// shared error parser cannot index into, so we surface `name` as the message.
fn stability_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let name = data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Stability request failed");
        aimux_provider_utils::ProviderErrorParts {
            message: name.to_string(),
            provider_code: Some(name.to_string()),
        }
    })
}

enum StabilitySuccess {
    Json(Value),
    Binary(Bytes),
}

fn stability_successful_response_handler() -> aimux_provider_utils::ResponseHandler<StabilitySuccess>
{
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let status = input.response.status().as_u16();
        let headers = aimux_provider_utils::extract_response_headers::extract_response_headers(
            input.response.headers(),
        );
        let is_json = input
            .response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
        let bytes =
            aimux_provider_utils::read_response_with_size_limit::read_response_with_size_limit(
                input.response,
                &input.url,
                &input.request_body_values,
                aimux_provider_utils::read_response_with_size_limit::DEFAULT_MAX_DOWNLOAD_SIZE,
                input.abort_signal.as_ref(),
            )
            .await?;
        let value = if is_json {
            let value = serde_json::from_slice(&bytes).map_err(|error| {
                AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                    status_code: Some(status),
                    response_headers: Some(headers.clone()),
                    response_body: Some(String::from_utf8_lossy(&bytes).into_owned()),
                    ..aimux_core::ApiCallError::new(
                        format!("Invalid JSON response: {error}"),
                        input.url,
                        input.request_body_values,
                    )
                }))
            })?;
            StabilitySuccess::Json(value)
        } else {
            StabilitySuccess::Binary(bytes)
        };
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: Some(headers),
        })
    })
}

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
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }
    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }
    /// Create from the `STABILITY_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "STABILITY_API_KEY", "Stability")?;
        Ok(Self::new(api_key))
    }
}

pub struct StabilityProvider {
    config: StabilityConfig,
}

impl StabilityProvider {
    #[must_use]
    pub fn new(config: StabilityConfig) -> Self {
        Self { config }
    }
    #[must_use]
    pub fn image(&self, model_id: &str) -> StabilityImageModel {
        StabilityImageModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for StabilityProvider {
    fn name(&self) -> &str {
        "stability"
    }
}

/// A Stability image generation model.
pub struct StabilityImageModel {
    model_id: String,
    config: StabilityConfig,
}

impl StabilityImageModel {
    #[must_use]
    pub fn new(model_id: String, config: StabilityConfig) -> Self {
        Self { model_id, config }
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
                .and_then(serde_json::Value::as_u64)
        });

        // Resolve output format (defaults to png).
        let output_format = stability_opts
            .and_then(|o| o.get("output_format"))
            .and_then(|v| v.as_str())
            .unwrap_or("png");

        // Build the multipart/form-data body.
        let mut form = MultipartForm::new();
        if let Some(ref p) = options.prompt {
            form.text("prompt", p)?;
        }
        if let Some(neg) = stability_opts
            .and_then(|o| o.get("negative_prompt"))
            .and_then(|v| v.as_str())
        {
            form.text("negative_prompt", neg)?;
        }
        if let Some(ref ar) = aspect_ratio {
            form.text("aspect_ratio", ar)?;
        }
        if let Some(seed) = seed {
            form.text("seed", &seed.to_string())?;
        }
        form.text("output_format", output_format)?;

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
                form.text(k, &text)?;
            }
        }

        let (body_bytes, content_type) = form.finish();

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let resp = aimux_provider_utils::post_to_api(
            HttpRequest {
                url: self.endpoint(),
                headers: header_list,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            HttpBody::Bytes(body_bytes, content_type),
            stability_successful_response_handler(),
            stability_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers.unwrap_or_default();
        let image_bytes = match resp.value {
            StabilitySuccess::Json(v) => {
                let b64 = v.get("image").and_then(|i| i.as_str()).ok_or_else(|| {
                    AiMuxError::InvalidResponseData(
                        "Stability response missing `image` field".into(),
                    )
                })?;
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).map_err(
                    |e| AiMuxError::InvalidResponseData(format!("invalid base64 image: {e}")),
                )?
            }
            StabilitySuccess::Binary(bytes) => bytes.to_vec(),
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
