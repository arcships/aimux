//! Replicate image provider.
//!
//! Aligned with Vercel AI SDK `ReplicateImageModel`
//! (`reference/ai/packages/replicate/src/replicate-image-model.ts`).
//!
//! [`create_replicate`] takes [`ReplicateProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ReplicateProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `REPLICATE_API_TOKEN`.
//! [`replicate()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult,
};
use aimux_core::shared::Warning;
use aimux_provider_utils::HttpRequest;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{
    AuthScheme, Credential, EndpointConfig, POLL_INTERVAL_MS_KEY, credential_headers,
    is_poll_control_key, poll_interval_ms, retry_download,
};

/// Replicate's `prefer: wait` holds the connection for at most this long when
/// no explicit duration is given (their documented default and maximum).
const DEFAULT_WAIT_SECONDS: u64 = 60;

/// Headroom added on top of the wait window for connect/TLS/body transport
/// when sizing the create exchange's `response_timeout`.
const WAIT_TRANSPORT_MARGIN_SECONDS: u64 = 5;

fn replicate_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("detail")
                .or_else(|| data.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown Replicate error")
                .to_string(),
            provider_code: None,
        }
    })
}

/// Milliseconds between two attempts to download a finished prediction's output
/// (`providerOptions.replicate.pollIntervalMs` overrides it for one call).
const DOWNLOAD_RETRY_INTERVAL_MS: u64 = 1_000;

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.replicate.com/v1";
const API_KEY_ENV_VAR: &str = "REPLICATE_API_TOKEN";
const DEFAULT_NAME: &str = "replicate";

/// Settings of [`create_replicate`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct ReplicateProviderSettings {
    /// Base URL for the API calls. Default `https://api.replicate.com/v1`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `REPLICATE_API_TOKEN` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.image"`, `"{name}.video"`).
    /// Default `"replicate"`. The providerOptions key stays `replicate`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for ReplicateProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplicateProviderSettings")
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

/// Create a Replicate provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_replicate(
    settings: ReplicateProviderSettings,
) -> Result<ReplicateProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(ReplicateProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        credential: Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Replicate"),
        user_headers: settings.headers,
        fetch: settings.fetch,
    })
}

/// The default provider: `create_replicate` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn replicate() -> &'static ReplicateProvider {
    static DEFAULT: OnceLock<ReplicateProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_replicate(ReplicateProviderSettings::default())
            .expect("default Replicate settings are always valid")
    })
}

/// A Replicate provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct ReplicateProvider {
    name: String,
    base_url: String,
    credential: Credential,
    user_headers: Option<HeaderMapOpt>,
    fetch: Option<FetchFunction>,
}

impl ReplicateProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            match method {
                "image" => credential_headers(
                    self.credential.clone(),
                    AuthScheme::Bearer,
                    Vec::new(),
                    self.user_headers.clone(),
                ),
                _ => credential_headers(
                    self.credential.clone(),
                    AuthScheme::Scheme("Token"),
                    Vec::new(),
                    self.user_headers.clone(),
                ),
            },
            self.fetch.clone(),
            None,
        )
    }

    /// An image model (e.g. `"black-forest-labs/flux-schnell"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> ReplicateImageModel {
        ReplicateImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }

    /// A video model; `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> ReplicateVideoModel {
        ReplicateVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

impl ::aimux_core::Provider for ReplicateProvider {
    fn language_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::LanguageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "languageModel"))
    }

    fn embedding_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(
        &self,
        model_id: &str,
    ) -> Result<::std::sync::Arc<dyn ::aimux_core::ImageModel>, AiMuxError> {
        Ok(::std::sync::Arc::new(self.image(model_id)))
    }

    fn video_model(
        &self,
        model_id: &str,
    ) -> Option<Result<::std::sync::Arc<dyn ::aimux_core::VideoModel>, AiMuxError>> {
        Some(Ok(::std::sync::Arc::new(self.video(model_id))))
    }
}

const FLUX_2_PATTERN: &str = "black-forest-labs/flux-2-";
const MAX_FLUX_2_INPUT_IMAGES: u32 = 8;

/// A Replicate image generation model.
pub struct ReplicateImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl ReplicateImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn is_flux2(&self) -> bool {
        self.model_id.starts_with(FLUX_2_PATTERN)
    }

    fn file_to_data_uri(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { url } => Ok(url.clone()),
            ImageFile::File { media_type, data } => {
                let b64 = match data {
                    ImageFileData::Base64(s) => s.clone(),
                    ImageFileData::Binary(b) => {
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b)
                    }
                };
                Ok(format!("data:{media_type};base64,{b64}"))
            }
        }
    }

    /// The path of the prediction endpoint of this model; a `:version` suffix
    /// of the model id is sent in the body, not in the path.
    fn endpoint_path(&self) -> String {
        let model = self
            .model_id
            .split_once(':')
            .map_or(self.model_id.as_str(), |(model, _version)| model);
        format!("/models/{model}/predictions")
    }
}

#[async_trait]
impl ImageModel for ReplicateImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        if self.is_flux2() {
            Some(MAX_FLUX_2_INPUT_IMAGES)
        } else {
            Some(1)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let mut warnings: Vec<Warning> = Vec::new();
        let (_model_id, version) = self
            .model_id
            .split_once(':')
            .unwrap_or((&self.model_id, ""));

        let replicate_opts = options::replicate_options(Some(&options.provider_options));
        let max_wait = replicate_opts
            .and_then(|o| o.get("maxWaitTimeInSeconds"))
            .and_then(serde_json::Value::as_u64);

        // Build image inputs
        let mut image_inputs = Map::new();
        if let Some(ref files) = options.files
            && !files.is_empty()
        {
            if self.is_flux2() {
                for (i, file) in files
                    .iter()
                    .enumerate()
                    .take(MAX_FLUX_2_INPUT_IMAGES as usize)
                {
                    let key = if i == 0 {
                        "input_image".to_string()
                    } else {
                        format!("input_image_{}", i + 1)
                    };
                    image_inputs.insert(key, json!(Self::file_to_data_uri(file)?));
                }
                if files.len() > MAX_FLUX_2_INPUT_IMAGES as usize {
                    warnings.push(Warning::Other { message: format!("Flux-2 models support up to {MAX_FLUX_2_INPUT_IMAGES} input images. Additional images are ignored.") });
                }
            } else {
                image_inputs.insert("image".into(), json!(Self::file_to_data_uri(&files[0])?));
                if files.len() > 1 {
                    warnings.push(Warning::Other { message: "This Replicate model only supports a single input image. Additional images are ignored.".into() });
                }
            }
        }

        // Handle mask
        let mask_input = if let Some(ref mask) = options.mask {
            if self.is_flux2() {
                warnings.push(Warning::Other {
                    message: "Flux-2 models do not support mask input. The mask will be ignored."
                        .into(),
                });
                None
            } else {
                Some(Self::file_to_data_uri(mask)?)
            }
        } else {
            None
        };

        // Build input object
        let mut input = Map::new();
        if let Some(ref p) = options.prompt {
            input.insert("prompt".into(), json!(p));
        }
        if let Some(ar) = options.aspect_ratio {
            input.insert("aspect_ratio".into(), json!(ar.to_string()));
        }
        if let Some(s) = options.size {
            input.insert("size".into(), json!(s.to_string()));
        }
        if let Some(seed) = options.seed {
            input.insert("seed".into(), json!(seed));
        }
        input.insert("num_outputs".into(), json!(options.n));
        for (k, v) in &image_inputs {
            input.insert(k.clone(), v.clone());
        }
        if let Some(m) = mask_input {
            input.insert("mask".into(), json!(m));
        }

        // Forward replicate provider options (excluding maxWaitTimeInSeconds and
        // the pacing keys, which are read here, never sent)
        if let Some(ro) = replicate_opts.and_then(|v| v.as_object()) {
            for (k, v) in ro {
                if k != "maxWaitTimeInSeconds" && !is_poll_control_key(k) {
                    input.insert(k.clone(), v.clone());
                }
            }
        }

        let mut body = Map::new();
        body.insert("input".into(), Value::Object(input));
        if !version.is_empty() {
            body.insert("version".into(), json!(version));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let prefer = if let Some(mw) = max_wait {
            format!("wait={mw}")
        } else {
            "wait".to_string()
        };
        let mut request = exchange.request(exchange.url(&self.endpoint_path()), options);
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("prefer"));
        request.headers.push(("prefer".to_string(), prefer));
        // This exchange legitimately holds the connection for the whole wait
        // window; widen the hang guard past it so a slow-but-alive generation
        // is not misread as a dead transport (retryable -> re-create -> double
        // billing).
        request.response_timeout = Some(Duration::from_secs(
            max_wait.unwrap_or(DEFAULT_WAIT_SECONDS) + WAIT_TRANSPORT_MARGIN_SECONDS,
        ));

        let resp = aimux_provider_utils::post_json_to_api(
            request,
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            replicate_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let rb: Value = resp.value;

        // Extract output (string or array of strings)
        let urls: Vec<String> = match &rb["output"] {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        };

        // Download images; output URLs come from the prediction response
        // body, so they go through the SSRF download guard. A transient failure
        // repeats the download, never the prediction.
        let mut downloaded: Vec<Vec<u8>> = Vec::new();
        for url in &urls {
            let ir = retry_download(
                options.abort_signal.as_ref(),
                Duration::from_millis(poll_interval_ms(
                    replicate_opts,
                    POLL_INTERVAL_MS_KEY,
                    DOWNLOAD_RETRY_INTERVAL_MS,
                )),
                || {
                    aimux_provider_utils::get_from_api(
                        HttpRequest {
                            url: url.clone(),
                            abort_signal: options.abort_signal.clone(),
                            validate_url: true,
                            trusted_origin: Some(exchange.base_url().to_string()),
                            credentialed_origin: Some(exchange.base_url().to_string()),
                            ..Default::default()
                        },
                        aimux_provider_utils::create_binary_response_handler(),
                        replicate_failed_response_handler(),
                    )
                },
            )
            .await?;
            downloaded.push(ir.value.to_vec());
        }

        Ok(ImageResult {
            images: ImageOutputs::Binary(downloaded),
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

// ════════════════════════════════════════════════════════════════════════════
// Video model
// ════════════════════════════════════════════════════════════════════════════

use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResponse, VideoResult,
};

/// Replicate video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `ReplicateVideoModel`
/// (`reference/ai/packages/replicate/src/replicate-video-model.ts`).
///
/// Uses the predictions API: POST to create (`do_start`); Core drives the
/// status polling via `do_status`.
pub struct ReplicateVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl ReplicateVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

fn video_file_to_url(file: &VideoFile) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => Ok(url.clone()),
        VideoFile::File { media_type, data } => match data {
            VideoFileData::Base64(s) => Ok(format!("data:{media_type};base64,{s}")),
            VideoFileData::Binary(bytes) => {
                let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
                Ok(format!("data:{media_type};base64,{b64}"))
            }
        },
    }
}

#[async_trait]
impl VideoModel for ReplicateVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let mut input = Map::new();
        if let Some(ref prompt) = options.prompt {
            input.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(ref image) = options.image {
            input.insert("image".to_string(), json!(video_file_to_url(image)?));
        }
        if let Some(seed) = options.seed {
            input.insert("seed".to_string(), json!(seed));
        }

        let body = json!({
            "model": self.model_id,
            "input": Value::Object(input),
        });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit prediction.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/predictions"), options),
            body,
            aimux_provider_utils::create_json_response_handler(),
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let prediction: Value = resp.value;
        let prediction_id = prediction
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiMuxError::InvalidResponseData("Replicate prediction missing id".to_string())
            })?
            .to_string();

        Ok(VideoOperationStart {
            operation: json!({ "prediction_id": prediction_id }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let prediction_id = operation
            .get("prediction_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(
                    "replicate operation reference is missing prediction_id".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/predictions/{prediction_id}"));

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            replicate_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let raw_body = resp.value;
        let status_str = raw_body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match status_str {
            "succeeded" => {}
            // A terminally failed prediction must be a non-retryable error,
            // not Pending, so the Core poll loop stops immediately.
            "failed" | "canceled" => {
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some(status_str.to_string()),
                    response_body,
                    ..ApiCallError::new(
                        format!("Replicate prediction {status_str}"),
                        poll_url,
                        serde_json::json!({}),
                    )
                })));
            }
            // starting / processing / unknown — keep polling.
            _ => return Ok(VideoOperationStatus::Pending),
        }

        // Extract video from output.
        let videos: Vec<VideoData> =
            if let Some(url) = raw_body.get("output").and_then(|v| v.as_str()) {
                vec![VideoData::Url {
                    url: url.to_string(),
                    media_type: "video/mp4".to_string(),
                }]
            } else if let Some(arr) = raw_body.get("output").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| {
                        v.as_str().map(|url| VideoData::Url {
                            url: url.to_string(),
                            media_type: "video/mp4".to_string(),
                        })
                    })
                    .collect()
            } else {
                vec![]
            };
        if videos.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "Replicate prediction succeeded without a usable output URL".to_string(),
            ));
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos,
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}
