//! KlingAI video generation provider.
//!
//! Aligned with Vercel AI SDK `createKlingAI` / `KlingAIVideoModel`
//! (`reference/ai/packages/klingai/src/klingai-video-model.ts`).
//!
//! KlingAI uses an async task pattern:
//! 1. POST to `/v1/videos/text2video` (or `image2video`) → returns task `id` + `task_id`
//! 2. GET `/v1/videos/text2video/{id}/{task_id}` — polled by Core via `do_status`
//!    until `succeeded`
//! 3. Return the video URL from the result

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};

use aimux_provider_utils::{HttpRequest, load_api_key, without_trailing_slash};

fn klingai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Kling AI request failed")
                .to_string(),
            provider_code: data.get("code").and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
        }
    })
}

// ── Config ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KlingAIConfig {
    pub api_key: String,
    pub base_url: String,
    pub headers: Option<HashMap<String, String>>,
}

impl KlingAIConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.klingai.com".to_string(),
            headers: None,
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

    /// Create from the `KLINGAI_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when the environment variable is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "KLINGAI_API_KEY", "KlingAI")?;
        Ok(Self::new(api_key))
    }
}

pub struct KlingAIProvider {
    config: KlingAIConfig,
}

impl KlingAIProvider {
    #[must_use]
    pub fn new(config: KlingAIConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn video(&self, model_id: &str) -> KlingAIVideoModel {
        KlingAIVideoModel::new(model_id.to_string(), self.config.clone())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Derive the KlingAI video mode from the model ID suffix.
fn detect_mode(model_id: &str) -> &str {
    if model_id.ends_with("-i2v") {
        "i2v"
    } else {
        "t2v"
    }
}

/// Derive the API model_name from the SDK model ID.
fn get_api_model_name(model_id: &str, mode: &str) -> String {
    let suffix = format!("-{mode}");
    let base = model_id.strip_suffix(&suffix).unwrap_or(model_id);
    base.replace(".0", "").replace('.', "-")
}

// ── Response schema ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct KlingAITaskResponse {
    code: i64,
    #[serde(default)]
    data: Option<KlingAITaskData>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KlingAITaskData {
    task_id: String,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KlingAITaskResult {
    code: i64,
    #[serde(default)]
    data: Option<KlingAITaskResultData>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KlingAITaskResultData {
    #[serde(default)]
    task_status: Option<String>,
    #[serde(default)]
    task_result: Option<KlingAITaskVideos>,
}

#[derive(Debug, Deserialize)]
struct KlingAITaskVideos {
    #[serde(default)]
    videos: Option<Vec<Value>>,
}

// ── Model ───────────────────────────────────────────────────────────────────

pub struct KlingAIVideoModel {
    model_id: String,
    config: KlingAIConfig,
}

impl KlingAIVideoModel {
    #[must_use]
    pub fn new(model_id: String, config: KlingAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
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

    fn submit_endpoint(&self, mode: &str) -> String {
        let path = match mode {
            "i2v" => "/v1/videos/image2video",
            "mi2v" => "/v1/videos/multi-image2video",
            "motion-control" => "/v1/videos/motion-control",
            _ => "/v1/videos/text2video",
        };
        format!("{}{path}", self.config.base_url)
    }

    fn poll_endpoint(&self, mode: &str, id: &str, task_id: &str) -> String {
        let path = match mode {
            "i2v" => "/v1/videos/image2video",
            "mi2v" => "/v1/videos/multi-image2video",
            "motion-control" => "/v1/videos/motion-control",
            _ => "/v1/videos/text2video",
        };
        format!("{}{path}/{id}/{task_id}", self.config.base_url)
    }
}

fn video_file_to_image_string(file: &aimux_core::video_model::VideoFile) -> String {
    match file {
        aimux_core::video_model::VideoFile::Url { url, .. } => url.clone(),
        aimux_core::video_model::VideoFile::File { data, .. } => match data {
            aimux_core::video_model::VideoFileData::Base64(s) => s.clone(),
            aimux_core::video_model::VideoFileData::Binary(bytes) => {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
            }
        },
    }
}

#[async_trait]
impl VideoModel for KlingAIVideoModel {
    fn provider(&self) -> &str {
        "klingai"
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
        let mode = detect_mode(&self.model_id).to_string();
        let model_name = get_api_model_name(&self.model_id, &mode);

        let mut body = Map::new();
        body.insert("model_name".to_string(), json!(model_name));

        if let Some(ref prompt) = options.prompt {
            body.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(negative_prompt) = options
            .provider_options
            .get("klingai")
            .and_then(|v| v.get("negativePrompt"))
            .and_then(|v| v.as_str())
        {
            body.insert("negative_prompt".to_string(), json!(negative_prompt));
        }
        if let Some(seed) = options.seed {
            body.insert("seed".to_string(), json!(seed));
        }
        if let Some(duration) = options.duration {
            body.insert("duration".to_string(), json!(duration));
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".to_string(), json!(ar.to_string()));
        }

        // Image-to-video: add image field
        if mode == "i2v" {
            if let Some(ref image) = options.image {
                body.insert(
                    "image".to_string(),
                    json!(video_file_to_image_string(image)),
                );
            } else if let Some(frame_images) = &options.frame_images
                && let Some(first_frame) = frame_images
                    .iter()
                    .find(|f| f.frame_type == aimux_core::video_model::VideoFrameType::FirstFrame)
            {
                body.insert(
                    "image".to_string(),
                    json!(video_file_to_image_string(&first_frame.image)),
                );
            }
        }

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        // Submit task.
        let submit_url = self.submit_endpoint(&mode);
        let request_body = Value::Object(body);
        let resp = aimux_provider_utils::post_json_to_api(
            HttpRequest {
                url: submit_url.clone(),
                headers: header_list,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            request_body.clone(),
            aimux_provider_utils::create_json_response_handler::<KlingAITaskResponse>(),
            klingai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let task: KlingAITaskResponse = resp.value;

        if task.code != 0 {
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: Some(task.code.to_string()),
                response_body,
                ..ApiCallError::new(
                    task.message
                        .unwrap_or_else(|| format!("KlingAI error code: {}", task.code)),
                    submit_url,
                    // In-band failure inside a 2xx envelope: the body is still
                    // raw here (i2v mode embeds base64 image data), so redact
                    // before it lands in the public error.
                    aimux_provider_utils::redact_error_context(request_body),
                )
            })));
        }

        let task_data = task.data.ok_or_else(|| {
            AiMuxError::InvalidResponseData(
                "KlingAI task submission did not return data".to_string(),
            )
        })?;

        let task_id = task_data.task_id;
        let id = task_data.id.ok_or_else(|| {
            AiMuxError::InvalidResponseData(
                "KlingAI task submission did not return a job id".to_string(),
            )
        })?;

        Ok(VideoOperationStart {
            // `mode` and `id` are needed alongside `task_id` to rebuild the
            // poll endpoint in `do_status`.
            operation: json!({ "mode": mode, "id": id, "task_id": task_id }),
            warnings: Vec::new(),
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: response_headers,
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let field = |name: &str| {
            operation.get(name).and_then(Value::as_str).ok_or_else(|| {
                AiMuxError::InvalidArgument(format!(
                    "klingai operation reference is missing {name}"
                ))
            })
        };
        let mode = field("mode")?;
        let id = field("id")?;
        let task_id = field("task_id")?;

        let headers = self.build_headers(options.headers.as_ref());
        let header_list: Vec<(String, String)> = headers.into_iter().collect();

        let poll_url = self.poll_endpoint(mode, id, task_id);
        let resp = aimux_provider_utils::get_from_api(
            HttpRequest {
                url: poll_url.clone(),
                headers: header_list,

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
                response_timeout: None,
                validate_url: false,
                trusted_origin: None,
                credentialed_origin: None,
            },
            aimux_provider_utils::create_json_response_handler::<KlingAITaskResult>(),
            klingai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers.unwrap_or_default();
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let result: KlingAITaskResult = resp.value;

        if result.code != 0 {
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                provider_code: Some(result.code.to_string()),
                response_body: response_body.clone(),
                ..ApiCallError::new(
                    result
                        .message
                        .unwrap_or_else(|| format!("KlingAI error code: {}", result.code)),
                    poll_url.clone(),
                    serde_json::json!({}),
                )
            })));
        }

        if let Some(data) = result.data
            && let Some(task_status) = data.task_status.as_deref()
        {
            if task_status == "succeeded" {
                let videos: Vec<VideoData> = data
                    .task_result
                    .and_then(|r| r.videos)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|v| {
                        v.get("url")
                            .and_then(|u| u.as_str())
                            .map(|url| VideoData::Url {
                                url: url.to_string(),
                                media_type: "video/mp4".to_string(),
                            })
                    })
                    .collect();
                if videos.is_empty() {
                    return Err(AiMuxError::InvalidResponseData(format!(
                        "KlingAI task {task_id} succeeded without video URLs"
                    )));
                }
                return Ok(VideoOperationStatus::Completed(VideoResult {
                    videos,
                    warnings: Vec::new(),
                    provider_metadata: None,
                    response: VideoResponse {
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                        model_id: Some(self.model_id.clone()),
                        headers: Some(response_headers),
                    },
                }));
            }
            if task_status == "failed" {
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(200),
                    provider_code: Some("failed".to_string()),
                    response_body,
                    ..ApiCallError::new(
                        "KlingAI video generation failed",
                        poll_url,
                        serde_json::json!({}),
                    )
                })));
            }
        }

        Ok(VideoOperationStatus::Pending)
    }
}
