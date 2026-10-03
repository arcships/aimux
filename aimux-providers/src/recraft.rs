//! Recraft image provider.
//!
//! OpenAI Images-compatible API with Recraft extension fields
//! (`style`, `style_id`, `negative_prompt`, `random_seed`).
//!
//! - Endpoint: `POST {base_url}/images/generations` (JSON body)
//! - Auth: Bearer token (`RECRAFT_API_TOKEN`)
//! - Base URL: `https://external.api.recraft.ai/v1`
//! - Response: `{ "data": [{ "url" | "b64_json" }] }` (OpenAI Images shape)
//! - No streaming.
//!
//! [`create_recraft`] takes [`RecraftProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RecraftProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `RECRAFT_API_TOKEN`.
//! [`recraft()`] is the default instance; it reads nothing and cannot fail.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{
    Credential, EndpointConfig, POLL_INTERVAL_MS_KEY, poll_interval_ms, provider_headers,
    retry_download,
};

fn recraft_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

/// Milliseconds between two attempts to download a generated image
/// (`providerOptions.recraft.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://external.api.recraft.ai/v1";
const API_KEY_ENV_VAR: &str = "RECRAFT_API_TOKEN";
const DEFAULT_NAME: &str = "recraft";

/// Settings of [`create_recraft`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct RecraftProviderSettings {
    /// Base URL for the API calls. Default `https://external.api.recraft.ai/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `RECRAFT_API_TOKEN` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`).
    /// Default `"recraft"`. The providerOptions key stays `recraft`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for RecraftProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecraftProviderSettings")
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

/// Create a Recraft provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_recraft(settings: RecraftProviderSettings) -> Result<RecraftProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(RecraftProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Recraft"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_recraft` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn recraft() -> &'static RecraftProvider {
    static DEFAULT: OnceLock<RecraftProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_recraft(RecraftProviderSettings::default())
            .expect("default Recraft settings are always valid")
    })
}

/// A Recraft provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct RecraftProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl RecraftProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// An image model (e.g. `"recraftv3"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> RecraftImageModel {
        RecraftImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(RecraftProvider, image_model, |p, id| p.image(id));

/// A Recraft image generation model.
pub struct RecraftImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl RecraftImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// Recraft generation provider options (camelCase → snake_case mapping).
struct RecraftOptions {
    style: Option<Value>,
    style_id: Option<Value>,
    negative_prompt: Option<Value>,
    random_seed: Option<Value>,
    response_format: Option<Value>,
}

/// Parse recraft provider options from the `"recraft"` key.
///
/// Unknown fields are ignored (safe degradation); values are passed through as
/// raw JSON so that unrecognised-but-valid enum values do not cause errors.
fn parse_recraft_options(provider_options: &HashMap<String, Value>) -> RecraftOptions {
    let recraft = options::recraft_options(Some(provider_options));
    RecraftOptions {
        style: recraft.and_then(|o| o.get("style")).cloned(),
        style_id: recraft.and_then(|o| o.get("styleId")).cloned(),
        negative_prompt: recraft.and_then(|o| o.get("negativePrompt")).cloned(),
        random_seed: recraft.and_then(|o| o.get("randomSeed")).cloned(),
        response_format: recraft.and_then(|o| o.get("responseFormat")).cloned(),
    }
}

/// Build the JSON body for `POST /images/generations`.
///
/// Fields with `None` values are omitted. The Recraft extension fields
/// (`style`, `style_id`, `negative_prompt`, `random_seed`) are forwarded from
/// `provider_options.recraft`. `random_seed` falls back to `options.seed` when
/// no explicit `randomSeed` option is supplied. `response_format` defaults to
/// `b64_json` unless overridden.
fn build_generation_body(
    model_id: &str,
    options: &ImageCallOptions,
    recraft: &RecraftOptions,
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
    if let Some(ref v) = recraft.style {
        body.insert("style".to_string(), v.clone());
    }
    if let Some(ref v) = recraft.style_id {
        body.insert("style_id".to_string(), v.clone());
    }
    if let Some(ref v) = recraft.negative_prompt {
        body.insert("negative_prompt".to_string(), v.clone());
    }
    // random_seed: explicit option takes precedence over options.seed.
    if let Some(ref v) = recraft.random_seed {
        body.insert("random_seed".to_string(), v.clone());
    } else if let Some(seed) = options.seed {
        body.insert("random_seed".to_string(), json!(seed));
    }
    // response_format: default to b64_json, allow override via provider options.
    let response_format = recraft
        .response_format
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or("b64_json");
    body.insert("response_format".to_string(), json!(response_format));
    body
}

/// Extract images from the OpenAI Images-shaped response.
///
/// - If `data[*].b64_json` is present, returns [`ImageOutputs::Base64`].
/// - Otherwise, if `data[*].url` is present, each URL is downloaded and
///   returned as [`ImageOutputs::Binary`].
/// - If neither is present, returns an empty [`ImageOutputs::Base64`].
async fn extract_images(
    response: &Value,
    abort_signal: Option<aimux_core::AbortSignal>,
    base_url: &str,
    retry_interval: Duration,
) -> Result<ImageOutputs, AiMuxError> {
    let items = response.get("data").and_then(|d| d.as_array());

    let Some(items) = items else {
        return Ok(ImageOutputs::Base64(Vec::new()));
    };

    let b64_images: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("b64_json")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    if !b64_images.is_empty() {
        return Ok(ImageOutputs::Base64(b64_images));
    }

    let urls: Vec<String> = items
        .iter()
        .filter_map(|item| item.get("url").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if urls.is_empty() {
        return Ok(ImageOutputs::Base64(Vec::new()));
    }

    let mut binaries = Vec::with_capacity(urls.len());
    for url in &urls {
        // data[].url is a generated-image URL from the response body, so it
        // goes through the SSRF download guard.
        let resp = retry_download(abort_signal.as_ref(), retry_interval, || {
            aimux_provider_utils::get_from_api(
                HttpRequest {
                    url: url.clone(),
                    abort_signal: abort_signal.clone(),
                    validate_url: true,
                    trusted_origin: Some(base_url.to_string()),
                    credentialed_origin: Some(base_url.to_string()),
                    ..Default::default()
                },
                aimux_provider_utils::create_binary_response_handler(),
                recraft_failed_response_handler(),
            )
        })
        .await?;
        binaries.push(resp.value.to_vec());
    }
    Ok(ImageOutputs::Binary(binaries))
}

#[async_trait]
impl ImageModel for RecraftImageModel {
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
        let mut warnings = Vec::new();

        if options.aspect_ratio.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "aspectRatio".to_string(),
                details: Some(
                    "Recraft does not support aspect ratio. Use `size` instead.".to_string(),
                ),
            });
        }

        let timestamp = chrono::Utc::now().to_rfc3339();
        let recraft_opts = parse_recraft_options(&options.provider_options);
        let body = build_generation_body(&self.model_id, options, &recraft_opts);

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/images/generations"), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            recraft_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let value: Value = resp.value;

        let images = extract_images(
            &value,
            options.abort_signal.clone(),
            exchange.base_url(),
            Duration::from_millis(poll_interval_ms(
                options::recraft_options(Some(&options.provider_options)),
                POLL_INTERVAL_MS_KEY,
                DOWNLOAD_RETRY_INTERVAL_MS,
            )),
        )
        .await?;

        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: None,
            response: ImageResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
            usage: None,
        })
    }
}
