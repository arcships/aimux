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

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageModel, ImageOutputs, ImageResponse, ImageResult,
};
use aimux_core::provider::Provider;
use aimux_core::shared::Warning;
use aimux_provider_utils::response::{DEFAULT_ERROR_STRUCTURE, api_call_to_provider_error};
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

const DEFAULT_BASE_URL: &str = "https://external.api.recraft.ai/v1";
const ENV_VAR: &str = "RECRAFT_API_TOKEN";
const PROVIDER_NAME: &str = "recraft";

/// Configuration for the Recraft provider.
#[derive(Debug, Clone)]
pub struct RecraftConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl RecraftConfig {
    /// Create from an API key, using the default Recraft base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            headers: None,
        }
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Set additional static HTTP headers sent with every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Create from the `RECRAFT_API_TOKEN` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, ENV_VAR, "Recraft")?;
        Ok(Self::new(api_key))
    }
}

/// Recraft provider — creates [`RecraftImageModel`] instances.
pub struct RecraftProvider {
    config: RecraftConfig,
}

impl RecraftProvider {
    pub fn new(config: RecraftConfig) -> Self {
        Self { config }
    }

    /// Create an image generation model instance (e.g. `"recraftv3"`).
    pub fn image(&self, model_id: &str) -> RecraftImageModel {
        RecraftImageModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for RecraftProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }
}

/// A Recraft image generation model.
pub struct RecraftImageModel {
    model_id: String,
    config: RecraftConfig,
}

impl RecraftImageModel {
    pub fn new(model_id: String, config: RecraftConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
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

    fn generations_endpoint(&self) -> String {
        format!("{}/images/generations", self.config.base_url)
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
    let recraft = provider_options.get("recraft");
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
    abort_signal: Option<aimux_core::shared::AbortSignal>,
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
        let resp = send(
            HttpRequest {
                method: HttpMethod::Get,
                url: url.clone(),
                headers: vec![],
                body: HttpBody::Empty,

                abort_signal: abort_signal.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;
        binaries.push(resp.body.to_vec());
    }
    Ok(ImageOutputs::Binary(binaries))
}

#[async_trait]
impl ImageModel for RecraftImageModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generations_endpoint(),
                headers: header_list,
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await
        .map_err(api_call_to_provider_error)?;

        let response_headers = resp.headers;
        let value: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| AiMuxError::Provider(format!("invalid JSON response: {e}")))?;

        let images = extract_images(&value, options.abort_signal.clone()).await?;

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
