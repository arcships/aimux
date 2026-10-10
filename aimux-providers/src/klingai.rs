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

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{Credential, EndpointConfig, ProviderHeaders};

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

pub(crate) mod options;

const DEFAULT_BASE_URL: &str = "https://api.klingai.com";
const API_KEY_ENV_VAR: &str = "KLINGAI_API_KEY";
const DEFAULT_NAME: &str = "klingai";

/// Settings of [`create_klingai`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct KlingAIProviderSettings {
    /// Base URL for the API calls. Default `https://api.klingai.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `KLINGAI_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included: it never falls back to
    /// the environment. A [`Resolvable::Future`] is awaited once, an
    /// [`Resolvable::AsyncFn`] on every request.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including the credential. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of the `provider()` strings (`"{name}.video"`).
    /// Default `"klingai"`. The providerOptions key stays `klingai`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
}

impl std::fmt::Debug for KlingAIProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KlingAIProviderSettings")
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

/// Create a KlingAI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_klingai(settings: KlingAIProviderSettings) -> Result<KlingAIProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(KlingAIProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: ProviderHeaders::bearer(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "KlingAI"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
    })
}

/// The default provider: `create_klingai` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn klingai() -> &'static KlingAIProvider {
    static DEFAULT: OnceLock<KlingAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_klingai(KlingAIProviderSettings::default())
            .expect("default KlingAI settings are always valid")
    })
}

/// A KlingAI provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct KlingAIProvider {
    name: String,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
}

impl KlingAIProvider {
    fn model_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
        )
    }

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
        if let Some(negative_prompt) = options::klingai_options(Some(&options.provider_options))
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

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit task.
        let submit_url = exchange.url(mode_path(&mode));
        let request_body = Value::Object(body);
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(submit_url.clone(), options),
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
                headers: Some(response_headers),
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
