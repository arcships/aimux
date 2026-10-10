//! Luma image provider.
//!
//! Aligned with Vercel AI SDK `LumaImageModel`
//! (`reference/ai/packages/luma/src/luma-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to create generation, then GET
//! poll until state is "completed", then download the image.
//!
//! [`create_luma`] takes [`LumaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LumaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LUMA_API_KEY`.
//! [`luma()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{
    Credential, EndpointConfig, POLL_INTERVAL_MILLIS_KEY, PollStep, ProviderHeaders,
    is_poll_control_key, poll_interval_ms, poll_until, retry_download,
};

fn luma_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let message = data
            .get("detail")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown Luma error")
            .to_string();
        aimux_provider_utils::ProviderErrorParts {
            message,
            provider_code: None,
        }
    })
}

/// Milliseconds between two polls of a generation (`providerOptions.luma.pollIntervalMillis`
/// overrides it for one call).
const POLL_INTERVAL_MS: u64 = 500;
/// How many times a generation is polled before the call gives up.
const MAX_POLL_ATTEMPTS: u32 = 120;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.lumalabs.ai";
const API_KEY_ENV_VAR: &str = "LUMA_API_KEY";
const DEFAULT_NAME: &str = "luma";

/// Settings of [`create_luma`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct LumaProviderSettings {
    /// Base URL for the API calls. Default `https://api.lumalabs.ai`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `LUMA_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`).
    /// Default `"luma"`. The providerOptions key stays `luma`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for LumaProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LumaProviderSettings")
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

/// Create a Luma provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_luma(settings: LumaProviderSettings) -> Result<LumaProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(LumaProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Luma"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_luma` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn luma() -> &'static LumaProvider {
    static DEFAULT: OnceLock<LumaProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_luma(LumaProviderSettings::default())
            .expect("default Luma settings are always valid")
    })
}

/// A Luma provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct LumaProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl LumaProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

    /// An image model (e.g. `"photon-1"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> LumaImageModel {
        LumaImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(LumaProvider, image_model, |p, id| p.image(id));

/// A Luma image generation model.
pub struct LumaImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl LumaImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl ImageModel for LumaImageModel {
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

        if options.seed.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "seed".into(),
                details: Some("This model does not support the `seed` option.".into()),
            });
        }
        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".into(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .into(),
                ),
            });
        }

        let luma_opts = options::luma_options(Some(&options.provider_options));

        // Extract non-request options
        let poll_interval = Duration::from_millis(poll_interval_ms(
            luma_opts,
            POLL_INTERVAL_MILLIS_KEY,
            POLL_INTERVAL_MS,
        ));
        let reference_type = luma_opts
            .and_then(|o| o.get("referenceType"))
            .and_then(|v| v.as_str())
            .unwrap_or("image");
        let image_configs = luma_opts
            .and_then(|o| o.get("images"))
            .and_then(|v| v.as_array())
            .cloned();

        // Build editing options from files
        let mut editing_opts = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            // Validate all files are URL-based
            for file in files {
                if !matches!(file, ImageFile::Url { .. }) {
                    return Err(AiMuxError::InvalidArgument(
                        "Luma AI only supports URL-based images.".into(),
                    ));
                }
            }

            let default_weight = match reference_type {
                "style" => 0.8,
                "modify_image" => 1.0,
                _ => 0.85, // image, character
            };

            match reference_type {
                "style" => {
                    let arr: Vec<Value> = files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let url = if let ImageFile::Url { url } = f {
                                url.clone()
                            } else {
                                String::new()
                            };
                            let weight = image_configs
                                .as_ref()
                                .and_then(|c| c.get(i))
                                .and_then(|c| c.get("weight"))
                                .and_then(serde_json::Value::as_f64)
                                .unwrap_or(default_weight);
                            json!({ "url": url, "weight": weight })
                        })
                        .collect();
                    editing_opts.insert("style".into(), json!(arr));
                }
                "modify_image" => {
                    if files.len() > 1 {
                        return Err(AiMuxError::InvalidArgument(
                            "Luma AI modify_image only supports a single input image.".into(),
                        ));
                    }
                    let url = if let ImageFile::Url { url } = &files[0] {
                        url.clone()
                    } else {
                        String::new()
                    };
                    let weight = image_configs
                        .as_ref()
                        .and_then(|c| c.first())
                        .and_then(|c| c.get("weight"))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(default_weight);
                    editing_opts.insert(
                        "modify_image".into(),
                        json!({ "url": url, "weight": weight }),
                    );
                }
                _ => {
                    // image (default)
                    if files.len() > 4 {
                        return Err(AiMuxError::InvalidArgument(
                            "Luma AI image supports up to 4 reference images.".into(),
                        ));
                    }
                    let arr: Vec<Value> = files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let url = if let ImageFile::Url { url } = f {
                                url.clone()
                            } else {
                                String::new()
                            };
                            let weight = image_configs
                                .as_ref()
                                .and_then(|c| c.get(i))
                                .and_then(|c| c.get("weight"))
                                .and_then(serde_json::Value::as_f64)
                                .unwrap_or(default_weight);
                            json!({ "url": url, "weight": weight })
                        })
                        .collect();
                    editing_opts.insert("image".into(), json!(arr));
                }
            }
        }

        if options.mask.is_some() {
            return Err(AiMuxError::InvalidArgument(
                "Luma AI does not support mask-based image editing.".into(),
            ));
        }

        // Build request body
        let mut body = Map::new();
        if let Some(ref p) = options.prompt {
            body.insert("prompt".into(), json!(p));
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".into(), json!(ar.to_string()));
        }
        body.insert("model".into(), json!(self.model_id));
        for (k, v) in &editing_opts {
            body.insert(k.clone(), v.clone());
        }

        // Forward luma provider options (excluding non-request options)
        if let Some(luma) = luma_opts {
            for (k, v) in luma {
                if is_poll_control_key(k) || matches!(k.as_str(), "referenceType" | "images") {
                    continue;
                }
                body.insert(k.clone(), v.clone());
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let generations_url = |generation_id: Option<&str>| match generation_id {
            Some(id) => exchange.url(&format!("/dream-machine/v1/generations/{id}")),
            None => exchange.url("/dream-machine/v1/generations/image"),
        };

        // Submit. This is the only request that creates a generation: nothing
        // below sends it again.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(generations_url(None), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            luma_failed_response_handler(),
        )
        .await?;
        let rh = resp.response_headers;
        let submit_body: Value = resp.value;

        let generation_id = submit_body
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("missing id in Luma response".to_string())
            })?
            .to_string();

        // Poll for completion.
        let poll_url = generations_url(Some(&generation_id));
        let image_url = poll_until(
            &format!("luma generation {generation_id}"),
            options.abort_signal.as_ref(),
            poll_interval,
            MAX_POLL_ATTEMPTS,
            || async {
                let pr = aimux_provider_utils::get_from_api(
                    exchange.request(poll_url.clone(), options),
                    aimux_provider_utils::create_json_response_handler::<Value>(),
                    luma_failed_response_handler(),
                )
                .await?;
                let response_body = pr.raw_value.as_ref().map(ToString::to_string);
                let pv = pr.value;

                let state = pv.get("state").and_then(|v| v.as_str()).unwrap_or("");
                if state == "completed" {
                    return pv
                        .get("assets")
                        .and_then(|a| a.get("image"))
                        .and_then(|v| v.as_str())
                        .map(|url| PollStep::Ready(url.to_string()))
                        .ok_or_else(|| {
                            AiMuxError::InvalidResponseData(format!(
                                "Luma generation {generation_id} completed without assets.image"
                            ))
                        });
                }
                if state == "failed" {
                    return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                        status_code: Some(200),
                        provider_code: Some(state.to_string()),
                        message: "Image generation failed.".into(),
                        response_body,
                        ..ApiCallError::new(
                            "Image generation failed.",
                            poll_url.clone(),
                            serde_json::json!({}),
                        )
                    })));
                }
                Ok(PollStep::Pending)
            },
        )
        .await?;

        // Download image; assets.image is a URL from the poll response body,
        // so it goes through the SSRF download guard.
        let ir = retry_download(options.abort_signal.as_ref(), poll_interval, || {
            aimux_provider_utils::get_from_api(
                HttpRequest {
                    url: image_url.clone(),
                    abort_signal: options.abort_signal.clone(),
                    validate_url: true,
                    trusted_origin: Some(exchange.base_url().to_string()),
                    credentialed_origin: Some(exchange.base_url().to_string()),
                    ..Default::default()
                },
                aimux_provider_utils::create_binary_response_handler(),
                aimux_provider_utils::create_status_code_error_response_handler(),
            )
        })
        .await?;
        let image_bytes = ir.value.to_vec();

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
