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

use std::sync::OnceLock;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};
use aimux_provider_utils::{HttpBody, MultipartForm};

use crate::shared::{Credential, EndpointConfig, provider_headers};

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

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.stability.ai";
const API_KEY_ENV_VAR: &str = "STABILITY_API_KEY";
const DEFAULT_NAME: &str = "stability";

/// Settings of [`create_stability`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct StabilityProviderSettings {
    /// Base URL for the API calls. Default `https://api.stability.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `STABILITY_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`).
    /// Default `"stability"`. The providerOptions key stays `stability`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for StabilityProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StabilityProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .finish()
    }
}

/// Create a Stability provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_stability(
    settings: StabilityProviderSettings,
) -> Result<StabilityProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(StabilityProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Stability"),
            vec![("Accept".to_string(), "image/*".to_string())],
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_stability` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn stability() -> &'static StabilityProvider {
    static DEFAULT: OnceLock<StabilityProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_stability(StabilityProviderSettings::default())
            .expect("default Stability settings are always valid")
    })
}

/// A Stability provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct StabilityProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl StabilityProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

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

        let stability_opts = options::stability_options(Some(&options.provider_options));

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
