//! Recraft image generation model.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, ImageResult};
use aimux_provider_utils::HttpRequest;

use crate::shared::{
    EndpointConfig, POLL_INTERVAL_MS_KEY, image_response, namespaced_options, poll_interval_ms,
    retry_download, unsupported,
};

fn recraft_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::message_type_error_response_handler()
}

/// Milliseconds between two attempts to download a generated image
/// (`providerOptions.recraft.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "recraft";

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
fn parse_recraft_options(
    provider_options: &aimux_core::shared::SharedProviderOptions,
) -> RecraftOptions {
    let recraft = namespaced_options(Some(provider_options), NAMESPACE);
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
            warnings.push(unsupported(
                "aspectRatio",
                "Recraft does not support aspect ratio. Use `size` instead.",
            ));
        }

        let timestamp = aimux_core::Timestamp::now();
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
                namespaced_options(Some(&options.provider_options), NAMESPACE),
                POLL_INTERVAL_MS_KEY,
                DOWNLOAD_RETRY_INTERVAL_MS,
            )),
        )
        .await?;

        Ok(ImageResult {
            images,
            warnings,
            provider_metadata: None,
            response: image_response(timestamp, &self.model_id, response_headers),
            usage: None,
        })
    }
}
