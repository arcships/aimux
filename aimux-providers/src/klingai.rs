//! KlingAI video generation provider.
//!
//! Aligned with Vercel AI SDK `createKlingAI` / `KlingAIVideoModel`
//! (`reference/ai/packages/klingai/src/klingai-video-model.ts`).
//!
//! KlingAI uses an async task pattern:
//! 1. POST to `/v1/videos/text2video` (or `image2video`) → returns task `id` + `task_id`
//! 2. GET `/v1/videos/text2video/{id}/{task_id}` to poll until `succeeded`
//! 3. Return the video URL from the result

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoResponse, VideoResult,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, load_api_key, send, without_trailing_slash,
};

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

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "KLINGAI_API_KEY", "KlingAI")?;
        Ok(Self::new(api_key))
    }
}

pub struct KlingAIProvider {
    config: KlingAIConfig,
}

impl KlingAIProvider {
    pub fn new(config: KlingAIConfig) -> Self {
        Self { config }
    }

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

    async fn do_generate(&self, options: &VideoCallOptions) -> Result<VideoResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();
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
        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.submit_endpoint(&mode),
                headers: header_list.clone(),
                body: HttpBody::Json(Value::Object(body)),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let task: KlingAITaskResponse =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        if task.code != 0 {
            return Err(AiMuxError::Provider(
                task.message
                    .unwrap_or_else(|| format!("KlingAI error code: {}", task.code)),
            ));
        }

        let task_data = task.data.ok_or_else(|| {
            AiMuxError::Provider("KlingAI task submission did not return data".to_string())
        })?;

        let task_id = task_data.task_id;
        let id = task_data.id.unwrap_or_default();

        // Poll for completion.
        let videos: Vec<VideoData>;
        let mut response_headers: HashMap<String, String>;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let resp = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url: self.poll_endpoint(&mode, &id, &task_id),
                    headers: header_list.clone(),
                    body: HttpBody::Empty,

                    abort_signal: options.abort_signal.clone(),
                    call_id: None,
                    recording_context: None,
                },
                RetryConfig::default(),
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            response_headers = resp.headers;

            let result: KlingAITaskResult =
                serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

            if result.code != 0 {
                return Err(AiMuxError::Provider(
                    result
                        .message
                        .unwrap_or_else(|| format!("KlingAI error code: {}", result.code)),
                ));
            }

            if let Some(data) = result.data
                && let Some(task_status) = data.task_status.as_deref()
            {
                if task_status == "succeeded" {
                    videos = data
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
                    break;
                }
                if task_status == "failed" {
                    return Err(AiMuxError::Provider(
                        "KlingAI video generation failed".to_string(),
                    ));
                }
            }
        }

        let timestamp = chrono::Utc::now().to_rfc3339();

        Ok(VideoResult {
            videos,
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(timestamp),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        })
    }
}
