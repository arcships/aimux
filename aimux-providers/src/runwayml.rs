//! RunwayML video generation provider (video modality only).
//!
//! RunwayML uses an async task pattern:
//! 1. POST to `/v1/text_to_video` (or `/v1/image_to_video` when an input image
//!    is supplied) → returns the task `id`.
//! 2. GET `/v1/tasks/{id}` polled until `SUCCEEDED` or `FAILED`.
//! 3. Return the generated video URL(s) from the task `output` array.
//!
//! Authentication is a `Bearer` token and every request must carry the
//! `X-Runway-Version: 2024-11-06` header.
//!
//! [`create_runwayml`] takes [`RunwaymlProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RunwaymlProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `RUNWAYML_API_SECRET`.
//! [`runwayml()`] is the default instance; it reads nothing and cannot fail.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameType, VideoModel,
    VideoOperationStart, VideoOperationStatus, VideoPollConfig, VideoResponse, VideoResult,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, provider_headers};

const PROVIDER_NAME: &str = "runwayml";
/// Milliseconds between two status checks of a task. The Core poll loop reads
/// it from [`VideoModel::poll_config`]; a call overrides it with
/// `VideoCallOptions::poll`.
const POLL_INTERVAL_MS: u64 = 2_000;
/// Milliseconds the Core poll loop waits for a task to finish.
const POLL_TIMEOUT_MS: u64 = 300_000;
const RUNWAY_VERSION: &str = "2024-11-06";

/// RunwayML returns errors as a flat `{"error": "<message>"}` object.
fn runwayml_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("RunwayML request failed")
                .to_string(),
            provider_code: None,
        }
    })
}

// ── Config ──────────────────────────────────────────────────────────────────

const DEFAULT_BASE_URL: &str = "https://api.dev.runwayml.com";
const API_KEY_ENV_VAR: &str = "RUNWAYML_API_SECRET";
const DEFAULT_NAME: &str = "runwayml";

/// Settings of [`create_runwayml`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct RunwaymlProviderSettings {
    /// Base URL for the API calls. Default `https://api.dev.runwayml.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `RUNWAYML_API_SECRET` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.video"`).
    /// Default `"runwayml"`. The providerOptions key stays `runwayml`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for RunwaymlProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunwaymlProviderSettings")
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

/// Create a RunwayML provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_runwayml(settings: RunwaymlProviderSettings) -> Result<RunwaymlProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(RunwaymlProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "RunwayML"),
            vec![("X-Runway-Version".to_string(), RUNWAY_VERSION.to_string())],
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_runwayml` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn runwayml() -> &'static RunwaymlProvider {
    static DEFAULT: OnceLock<RunwaymlProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_runwayml(RunwaymlProviderSettings::default())
            .expect("default RunwayML settings are always valid")
    })
}

/// A RunwayML provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct RunwaymlProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

impl RunwaymlProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            None,
        )
    }

    /// A video model (e.g. `"gen4_turbo"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> RunwaymlVideoModel {
        RunwaymlVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_single_modality_provider!(RunwaymlProvider, video_model, |p, id| p.video(id));

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a [`VideoFile`] into the string RunwayML accepts as `promptImage`
/// (a URL, or base64-encoded data for inline files).
fn video_file_to_prompt_image(file: &VideoFile) -> String {
    match file {
        VideoFile::Url { url, .. } => url.clone(),
        VideoFile::File { data, .. } => match data {
            VideoFileData::Base64(s) => s.clone(),
            VideoFileData::Binary(bytes) => {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
            }
        },
    }
}

// ── Response schema (private wire types) ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RunwaymlTaskCreationResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RunwaymlTaskDetailsResponse {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    output: Option<Vec<String>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

/// A RunwayML video generation model — implements [`VideoModel`].
pub struct RunwaymlVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl RunwaymlVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl VideoModel for RunwaymlVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    fn poll_config(&self) -> VideoPollConfig {
        VideoPollConfig {
            interval: Duration::from_millis(POLL_INTERVAL_MS),
            timeout: Duration::from_millis(POLL_TIMEOUT_MS),
        }
    }

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        // Image-to-video when an input image (or a first frame) is provided.
        let image_input: Option<String> = options
            .image
            .as_ref()
            .map(video_file_to_prompt_image)
            .or_else(|| {
                options.frame_images.as_ref().and_then(|frames| {
                    frames
                        .iter()
                        .find(|f| f.frame_type == VideoFrameType::FirstFrame)
                        .map(|f| video_file_to_prompt_image(&f.image))
                })
            });
        let is_image_to_video = image_input.is_some();

        // Build the request body.
        let mut body = Map::new();
        body.insert("model".to_string(), json!(self.model_id));
        if let Some(ref prompt) = options.prompt {
            body.insert("promptText".to_string(), json!(prompt));
        }
        if let Some(image) = image_input.as_ref() {
            body.insert("promptImage".to_string(), json!(image));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(duration) = options.duration {
            body.insert("duration".to_string(), json!(duration));
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("ratio".to_string(), json!(ar.to_string()));
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let submit_path = if is_image_to_video {
            "/v1/image_to_video"
        } else {
            "/v1/text_to_video"
        };
        let submit_url = exchange.url(submit_path);

        // Submit the task.
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(submit_url, options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            runwayml_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let task: RunwaymlTaskCreationResponse = resp.value;

        Ok(VideoOperationStart {
            operation: json!({ "task_id": task.id }),
            warnings: Vec::new(),
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
        let task_id = operation
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(format!(
                    "{PROVIDER_NAME} operation reference is missing task_id"
                ))
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let poll_url = exchange.url(&format!("/v1/tasks/{task_id}"));

        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<RunwaymlTaskDetailsResponse>(),
            runwayml_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.clone();
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let task_details: RunwaymlTaskDetailsResponse = resp.value;

        let status_str = task_details.status.clone().unwrap_or_default();
        match status_str.as_str() {
            "SUCCEEDED" => {
                let final_output =
                    task_details
                        .output
                        .filter(|o| !o.is_empty())
                        .ok_or_else(|| {
                            AiMuxError::InvalidResponseData(format!(
                                "{PROVIDER_NAME} task {task_id} succeeded without output"
                            ))
                        })?;

                let videos: Vec<VideoData> = final_output
                    .into_iter()
                    .map(|url| VideoData::Url {
                        url,
                        media_type: "video/mp4".to_string(),
                    })
                    .collect();

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
            // A terminally failed task must be a non-retryable error, not
            // Pending, so the Core poll loop stops immediately.
            "FAILED" | "CANCELLED" => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: Some(status_str.clone()),
                response_body,
                ..ApiCallError::new(
                    format!("{PROVIDER_NAME} task {task_id} failed with status {status_str}"),
                    poll_url,
                    serde_json::json!({}),
                )
            }))),
            // PENDING / THROTTLED / RUNNING / unknown — keep polling.
            _ => Ok(VideoOperationStatus::Pending),
        }
    }
}
