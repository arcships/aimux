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

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFrameType, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResult,
};
use aimux_provider_utils::AuthScheme;

use crate::shared::{
    EndpointConfig, define_vendor_provider, task_failed, video_file_to_url_or_base64,
    video_response,
};

const PROVIDER_NAME: &str = "runwayml";
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

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.dev.runwayml.com";
pub(crate) const API_KEY_ENV_VAR: &str = "RUNWAYML_API_SECRET";
const DEFAULT_NAME: &str = "runwayml";

define_vendor_provider! {
    settings: RunwaymlProviderSettings,
    provider: RunwaymlProvider,
    create: create_runwayml,
    singleton: runwayml,
    display: "RunwayML",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: vec![("X-Runway-Version".to_string(), RUNWAY_VERSION.to_string())],
    package: None,
}

impl RunwaymlProvider {
    /// A video model (e.g. `"gen4_turbo"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> RunwaymlVideoModel {
        RunwaymlVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_single_modality_provider!(RunwaymlProvider, video_model, |p, id| p.video(id));

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

    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        // Image-to-video when an input image (or a first frame) is provided.
        let image_input: Option<String> = options
            .image
            .as_ref()
            .map(video_file_to_url_or_base64)
            .or_else(|| {
                options.frame_images.as_ref().and_then(|frames| {
                    frames
                        .iter()
                        .find(|f| f.frame_type == VideoFrameType::FirstFrame)
                        .map(|f| video_file_to_url_or_base64(&f.image))
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
            response: video_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                response_headers,
            ),
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
                AiMuxError::invalid_argument(format!(
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
                            AiMuxError::invalid_response_data(format!(
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
                    response: video_response(
                        aimux_core::Timestamp::now(),
                        &self.model_id,
                        response_headers,
                    ),
                }))
            }
            // A terminally failed task must be a non-retryable error, not
            // Pending, so the Core poll loop stops immediately.
            "FAILED" | "CANCELLED" => Err(task_failed(
                format!("{PROVIDER_NAME} task {task_id} failed with status {status_str}"),
                poll_url,
                status_str.clone(),
                response_body,
            )),
            // PENDING / THROTTLED / RUNNING / unknown — keep polling.
            _ => Ok(VideoOperationStatus::Pending),
        }
    }
}
