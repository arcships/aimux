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

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::provider::Provider;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFileData, VideoFrameType, VideoModel,
    VideoOperationStart, VideoOperationStatus, VideoPollConfig, VideoResponse, VideoResult,
};
use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

const PROVIDER_NAME: &str = "runwayml";
const DEFAULT_BASE_URL: &str = "https://api.dev.runwayml.com";
const ENV_VAR: &str = "RUNWAYML_API_SECRET";
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

/// Configuration for the RunwayML provider.
#[derive(Debug, Clone)]
pub struct RunwaymlConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
    /// Interval between task-status polls. Defaults to 2 seconds.
    pub poll_interval: Duration,
    /// Maximum total time to wait for a task to finish. Defaults to 300 seconds.
    pub timeout: Duration,
}

impl RunwaymlConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            headers: None,
            poll_interval: Duration::from_secs(2),
            timeout: Duration::from_secs(300),
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

    /// Override the interval between status polls.
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Override the maximum time to wait for a task to finish.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Create from the `RUNWAYML_API_SECRET` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, ENV_VAR, "RunwayML")?;
        Ok(Self::new(api_key))
    }
}

// ── Provider ────────────────────────────────────────────────────────────────

pub struct RunwaymlProvider {
    config: RunwaymlConfig,
}

impl RunwaymlProvider {
    #[must_use]
    pub fn new(config: RunwaymlConfig) -> Self {
        Self { config }
    }

    /// Create a video generation model instance for the given model ID.
    #[must_use]
    pub fn video(&self, model_id: &str) -> RunwaymlVideoModel {
        RunwaymlVideoModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for RunwaymlProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }
}

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
    config: RunwaymlConfig,
}

impl RunwaymlVideoModel {
    #[must_use]
    pub fn new(model_id: String, config: RunwaymlConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        headers.insert("X-Runway-Version".to_string(), RUNWAY_VERSION.to_string());
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }
}

#[async_trait]
impl VideoModel for RunwaymlVideoModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    fn poll_config(&self) -> VideoPollConfig {
        VideoPollConfig {
            interval: self.config.poll_interval,
            timeout: self.config.timeout,
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let submit_path = if is_image_to_video {
            "/v1/image_to_video"
        } else {
            "/v1/text_to_video"
        };
        let submit_url = format!("{}{submit_path}", self.config.base_url);

        // Submit the task.
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest::new(submit_url, header_list, options),
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

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();
        let poll_url = format!("{}/v1/tasks/{}", self.config.base_url, task_id);

        let resp = aimux_provider_utils::get_from_api(
            HttpRequest::new(poll_url.clone(), header_list, options),
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
