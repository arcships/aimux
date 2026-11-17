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
//!
//! [`create_klingai`] takes [`KlingAIProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`KlingAIProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `KLINGAI_API_KEY`.
//! [`klingai()`] is the default instance; it reads nothing and cannot fail.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus, VideoResult,
};
use aimux_provider_utils::AuthScheme;

use crate::shared::{
    EndpointConfig, define_vendor_provider, namespaced_options, task_failed,
    video_file_to_url_or_base64, video_response,
};

fn klingai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        aimux_provider_utils::ProviderErrorParts {
            message: data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Kling AI request failed")
                .to_string(),
            provider_code: data.get("code").and_then(crate::shared::json_error_code),
        }
    })
}

// ── Config ──────────────────────────────────────────────────────────────────

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "klingai";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.klingai.com";
pub(crate) const API_KEY_ENV_VAR: &str = "KLINGAI_API_KEY";
const DEFAULT_NAME: &str = "klingai";

define_vendor_provider! {
    settings: KlingAIProviderSettings,
    provider: KlingAIProvider,
    create: create_klingai,
    singleton: klingai,
    display: "KlingAI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl KlingAIProvider {
    /// A video model (e.g. `"kling-v2.6-t2v"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> KlingAIVideoModel {
        KlingAIVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_single_modality_provider!(KlingAIProvider, video_model, |p, id| p.video(id));

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
    config: EndpointConfig,
}

impl KlingAIVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// The API path of a generation mode.
fn mode_path(mode: &str) -> &'static str {
    match mode {
        "i2v" => "/v1/videos/image2video",
        "mi2v" => "/v1/videos/multi-image2video",
        "motion-control" => "/v1/videos/motion-control",
        _ => "/v1/videos/text2video",
    }
}

#[async_trait]
impl VideoModel for KlingAIVideoModel {
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
        let mode = detect_mode(&self.model_id).to_string();
        let model_name = get_api_model_name(&self.model_id, &mode);

        let mut body = Map::new();
        body.insert("model_name".to_string(), json!(model_name));

        if let Some(ref prompt) = options.prompt {
            body.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(negative_prompt) =
            namespaced_options(Some(&options.provider_options), NAMESPACE)
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
                    json!(video_file_to_url_or_base64(image)),
                );
            } else if let Some(frame_images) = &options.frame_images
                && let Some(first_frame) = frame_images
                    .iter()
                    .find(|f| f.frame_type == aimux_core::video_model::VideoFrameType::FirstFrame)
            {
                body.insert(
                    "image".to_string(),
                    json!(video_file_to_url_or_base64(&first_frame.image)),
                );
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit task.
        let submit_url = exchange.url(mode_path(&mode));
        let request_body = Value::Object(body);
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(submit_url, options),
            request_body,
            aimux_provider_utils::ResponseHandler::new_with_source(
                |input, body_source| async move {
                    let url = input.url.clone();
                    let out =
                        aimux_provider_utils::create_json_response_handler::<KlingAITaskResponse>()
                            .handle_with_source(input, Some(body_source.clone()))
                            .await?;
                    if out.value.code != 0 {
                        let response_body = out.raw_value.as_ref().map(ToString::to_string);
                        let code = out.value.code;
                        let msg = out
                            .value
                            .message
                            .unwrap_or_else(|| format!("KlingAI error code: {code}"));
                        return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                            status_code: Some(200),
                            provider_code: Some(code.to_string()),
                            response_body,
                            origin: aimux_core::ApiCallErrorOrigin::HttpResponse,
                            ..ApiCallError::new(
                                msg,
                                url,
                                // In-band failure inside a 2xx envelope: lazily materialize
                                // the redacted request body only when an error occurs.
                                body_source.materialize(),
                            )
                        })));
                    }
                    Ok(out)
                },
            ),
            klingai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let task: KlingAITaskResponse = resp.value;

        let task_data = task.data.ok_or_else(|| {
            AiMuxError::invalid_response_data(
                "KlingAI task submission did not return data".to_string(),
            )
        })?;

        let task_id = task_data.task_id;
        let id = task_data.id.ok_or_else(|| {
            AiMuxError::invalid_response_data(
                "KlingAI task submission did not return a job id".to_string(),
            )
        })?;

        Ok(VideoOperationStart {
            // `mode` and `id` are needed alongside `task_id` to rebuild the
            // poll endpoint in `do_status`.
            operation: json!({ "mode": mode, "id": id, "task_id": task_id }),
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
        let field = |name: &str| {
            operation.get(name).and_then(Value::as_str).ok_or_else(|| {
                AiMuxError::invalid_argument(format!(
                    "klingai operation reference is missing {name}"
                ))
            })
        };
        let mode = field("mode")?;
        let id = field("id")?;
        let task_id = field("task_id")?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let poll_url = exchange.url(&format!("{}/{id}/{task_id}", mode_path(mode)));
        let resp = aimux_provider_utils::get_from_api(
            exchange.request(poll_url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<KlingAITaskResult>(),
            klingai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let result: KlingAITaskResult = resp.value;

        if result.code != 0 {
            return Err(task_failed(
                result
                    .message
                    .unwrap_or_else(|| format!("KlingAI error code: {}", result.code)),
                poll_url.clone(),
                result.code.to_string(),
                response_body.clone(),
            ));
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
                    return Err(AiMuxError::invalid_response_data(format!(
                        "KlingAI task {task_id} succeeded without video URLs"
                    )));
                }
                return Ok(VideoOperationStatus::Completed(VideoResult {
                    videos,
                    warnings: Vec::new(),
                    provider_metadata: None,
                    response: video_response(
                        aimux_core::Timestamp::now(),
                        &self.model_id,
                        response_headers,
                    ),
                }));
            }
            if task_status == "failed" {
                return Err(task_failed(
                    "KlingAI video generation failed",
                    poll_url,
                    "failed",
                    response_body,
                ));
            }
        }

        Ok(VideoOperationStatus::Pending)
    }
}
