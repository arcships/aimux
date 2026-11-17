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
//!
//! [`create_stability`] takes [`StabilityProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`StabilityProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `STABILITY_API_KEY`.
//! [`stability()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, ImageResult};
use aimux_core::shared::Warning;
use aimux_provider_utils::{AuthScheme, HttpBody, MultipartForm};

use crate::shared::{
    EndpointConfig, define_vendor_provider, image_response, namespaced_options,
    size_to_aspect_ratio, unsupported,
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
    aimux_provider_utils::ResponseHandler::new_with_source(|input, body_source| async move {
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
        let read_source = body_source.clone();
        let bytes =
            aimux_provider_utils::read_response_with_size_limit::read_response_with_size_limit_lazy(
                input.response,
                &input.url,
                move || read_source.materialize(),
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
                    cause: Some(aimux_core::ErrorCause::source(&error)),
                    origin: aimux_core::ApiCallErrorOrigin::HttpResponse,
                    ..aimux_core::ApiCallError::new(
                        format!("Invalid JSON response: {error}"),
                        input.url,
                        body_source.materialize(),
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
            response_headers: headers,
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

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "stability";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.stability.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "STABILITY_API_KEY";
const DEFAULT_NAME: &str = "stability";

define_vendor_provider! {
    settings: StabilityProviderSettings,
    provider: StabilityProvider,
    create: create_stability,
    singleton: stability,
    display: "Stability",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: vec![("Accept".to_string(), "image/*".to_string())],
    package: None,
}

impl StabilityProvider {
    /// An image model (e.g. `"stable-image-core"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> StabilityImageModel {
        StabilityImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(StabilityProvider, image_model, |p, id| p.image(id));

/// A Stability image generation model.
pub struct StabilityImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl StabilityImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl ImageModel for StabilityImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();

        let stability_opts = namespaced_options(Some(&options.provider_options), NAMESPACE);

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
            if let Some(ar) = size_to_aspect_ratio(&size) {
                warnings.push(unsupported(
                    "size",
                    "Deriving aspect_ratio from size. Stability uses aspect_ratio, not \
                         explicit width/height, for these models.",
                ));
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
        if let Some(stab) = stability_opts {
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_to_api(
            exchange.request(
                exchange.url(&format!(
                    "/v2beta/stable-image/generate/{}",
                    model_id_to_subpath(&self.model_id)
                )),
                options,
            ),
            HttpBody::Bytes(body_bytes, content_type),
            stability_successful_response_handler(),
            stability_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let image_bytes = match resp.value {
            StabilitySuccess::Json(v) => {
                let b64 = v.get("image").and_then(|i| i.as_str()).ok_or_else(|| {
                    AiMuxError::invalid_response_data("Stability response missing `image` field")
                })?;
                aimux_provider_utils::convert_base64_to_uint8_array(b64).map_err(|e| {
                    AiMuxError::InvalidResponseData {
                        message: format!("invalid base64 image: {e}"),
                        cause: Some(ErrorCause::source(&e)),
                    }
                })?
            }
            StabilitySuccess::Binary(bytes) => bytes.to_vec(),
        };

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: None,
            response: image_response(aimux_core::Timestamp::now(), &self.model_id, rh),
            usage: None,
        })
    }
}
