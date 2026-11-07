//! The fal video model (`fal-video-model.ts`) and its options
//! (`fal-video-model-options.ts`, `fal-video-settings.ts`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoModel, VideoOperationStart, VideoOperationStatus,
    VideoResponse, VideoResult,
};
use aimux_provider_utils::HttpRequest;

use crate::shared::{EndpointConfig, is_poll_control_key};

use super::error::{fal_failed_response_handler, is_in_progress};
use super::image_model::number_in;
use super::options;

// ════════════════════════════════════════════════════════════════════════════
// Video model
// ════════════════════════════════════════════════════════════════════════════

/// The video queue's answer (fal-video-model.ts `falJobResponseSchema`):
/// both fields nullish, and only `response_url` is used.
#[derive(Debug, Deserialize)]
struct FalVideoJobResponse {
    #[serde(default)]
    response_url: Option<String>,
}

/// The status poll of a video (fal-video-model.ts fetchStatus). Its URL comes
/// from the queue response body, so it goes through the SSRF guard, and the
/// caller headers (the `Key` credential among them) are sent only while it
/// stays on the origin that was submitted to: `credentialedOrigin` and
/// `trustedOrigin` are both the submit URL.
pub(super) fn status_request(mut request: HttpRequest, submit_url: &str) -> HttpRequest {
    request.validate_url = true;
    request.credentialed_origin = Some(submit_url.to_string());
    request.trusted_origin = Some(submit_url.to_string());
    request
}

/// Fal video generation model — implements `VideoModel`.
///
/// Aligned with Vercel AI SDK `FalVideoModel`
/// (`reference/ai/packages/fal/src/fal-video-model.ts`).
///
/// Uses the same async queue pattern as the transcription model; the status
/// polling is driven by Core via `do_status`.
pub struct FalVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl FalVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }

    fn normalized_model_id(&self) -> String {
        self.model_id
            .strip_prefix("fal-ai/")
            .or_else(|| self.model_id.strip_prefix("fal/"))
            .unwrap_or(&self.model_id)
            .to_string()
    }

    fn submit_path(&self) -> String {
        format!("/fal-ai/{}", self.normalized_model_id())
    }
}

/// `falVideoModelOptionsSchema` (fal-video-model-options.ts), checked only:
/// a loose object whose five known keys are `.nullish()`. The body is built
/// from the raw namespace, which keeps every unknown key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the body is built from the raw namespace"
)]
struct FalVideoOptions {
    #[serde(default, rename = "loop")]
    loop_video: Option<bool>,
    #[serde(default, deserialize_with = "motion_strength")]
    motion_strength: Option<f64>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    negative_prompt: Option<String>,
    #[serde(default)]
    prompt_optimizer: Option<bool>,
}

/// `motionStrength`: `z.number().min(0).max(1)`.
fn motion_strength<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    number_in(deserializer, 0.0, 1.0)
}

/// The schema keys of [`FalVideoOptions`] and the body keys they are sent
/// as (fal-video-model.ts buildRequestBody).
const FAL_VIDEO_OPTION_KEYS: [(&str, &str); 5] = [
    ("loop", "loop"),
    ("motionStrength", "motion_strength"),
    ("resolution", "resolution"),
    ("negativePrompt", "negative_prompt"),
    ("promptOptimizer", "prompt_optimizer"),
];

fn video_file_to_data_uri(file: &VideoFile) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => Ok(url.clone()),
        VideoFile::File { media_type, data } => Ok(format!(
            "data:{media_type};base64,{}",
            aimux_provider_utils::convert_to_base64(data)
        )),
    }
}

#[async_trait]
impl VideoModel for FalVideoModel {
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
        aimux_provider_utils::parse_provider_options::<FalVideoOptions>(
            options::NAMESPACE,
            Some(&options.provider_options),
        )?;
        let mut body = Map::new();

        if let Some(ref prompt) = options.prompt {
            body.insert("prompt".to_string(), json!(prompt));
        }
        if let Some(ref image) = options.image {
            body.insert(
                "image_url".to_string(),
                json!(video_file_to_data_uri(image)?),
            );
        }
        if let Some(ar) = options.aspect_ratio {
            body.insert("aspect_ratio".to_string(), json!(ar.to_string()));
        }
        // Upstream tests `if (options.duration)` and `if (options.seed)`: a 0
        // is not sent.
        if let Some(duration) = options.duration.filter(|&duration| duration != 0) {
            body.insert("duration".to_string(), json!(format!("{duration}s")));
        }
        if let Some(seed) = options.seed.filter(|&seed| seed != 0) {
            body.insert("seed".to_string(), json!(seed));
        }

        if let Some(fal) = options::fal_options(Some(&options.provider_options)) {
            // The schema keys first, a non-null value renamed; then every
            // other key as given, so an unknown key wins over a renamed one.
            for (key, wire_key) in FAL_VIDEO_OPTION_KEYS {
                if let Some(value) = fal.get(key).filter(|value| !value.is_null()) {
                    body.insert(wire_key.to_string(), value.clone());
                }
            }
            for (key, value) in fal {
                let known = FAL_VIDEO_OPTION_KEYS.iter().any(|(k, _)| k == key);
                if !known && !is_poll_control_key(key) {
                    body.insert(key.clone(), value.clone());
                }
            }
        }

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        // Submit.
        let submit_url = exchange.url(&self.submit_path());
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(submit_url.clone(), options),
            Value::Object(body),
            aimux_provider_utils::create_json_response_handler(),
            fal_failed_response_handler(),
        )
        .await?;

        // fal-video-model.ts submitToQueue: `if (!responseUrl)`, so an empty
        // URL fails like a missing one.
        let job: FalVideoJobResponse = resp.value;
        let response_url = job
            .response_url
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data("No response URL returned from queue endpoint")
            })?;

        Ok(VideoOperationStart {
            operation: json!({ "responseUrl": response_url, "submitUrl": submit_url }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                // fal-video-model.ts doStart reports `headers: undefined`:
                // the submission's headers are not carried.
                headers: None,
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let response_url = operation
            .get("responseUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "fal operation reference is missing responseUrl".to_string(),
                )
            })?;
        let submit_url = operation
            .get("submitUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument(
                    "fal operation reference is missing submitUrl".to_string(),
                )
            })?;

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::get_from_api(
            status_request(
                exchange.request(response_url.to_string(), options),
                submit_url,
            ),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            fal_failed_response_handler(),
        )
        .await;

        let resp = match resp {
            Ok(response) => response,
            Err(error) if is_in_progress(&error) => return Ok(VideoOperationStatus::Pending),
            Err(error) => return Err(error),
        };

        let response_headers = resp.response_headers;
        let raw_body = resp.value;

        // fal-video-model.ts buildResult reads `response.video.url` only: an
        // empty string counts as missing, and there is no top-level fallback.
        let video_url = raw_body
            .get("video")
            .and_then(|video| video.get("url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty());
        let (Some(video), Some(video_url)) = (raw_body.get("video"), video_url) else {
            return Err(AiMuxError::invalid_response_data(
                "No video URL in response".to_string(),
            ));
        };

        // `mediaType` is the response's `content_type`, else `video/mp4`.
        let media_type = video
            .get("content_type")
            .and_then(Value::as_str)
            .filter(|content_type| !content_type.is_empty())
            .unwrap_or("video/mp4")
            .to_string();

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos: vec![VideoData::Url {
                url: video_url.to_string(),
                media_type,
            }],
            warnings: Vec::new(),
            provider_metadata: Some(fal_video_metadata(&raw_body, video, video_url)),
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
            },
        }))
    }
}

/// `providerMetadata.fal` of a completed video (`fal-video-model.ts`
/// buildResult): the video's fields under `videos`, then the job's `seed`,
/// `timings`, `has_nsfw_concepts` when present, and `prompt` when truthy.
fn fal_video_metadata(
    raw_body: &Value,
    video: &Value,
    video_url: &str,
) -> aimux_core::shared::SharedProviderMetadata {
    let field = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
    let mut fal = Map::new();
    fal.insert(
        "videos".into(),
        json!([{
            "url": video_url,
            "width": field("width"),
            "height": field("height"),
            "duration": field("duration"),
            "fps": field("fps"),
            "contentType": field("content_type"),
        }]),
    );
    if let Some(seed) = raw_body.get("seed").filter(|seed| !seed.is_null()) {
        fal.insert("seed".into(), seed.clone());
    }
    if let Some(timings) = raw_body.get("timings").filter(|timings| !timings.is_null()) {
        fal.insert("timings".into(), timings.clone());
    }
    if let Some(nsfw) = raw_body
        .get("has_nsfw_concepts")
        .filter(|nsfw| !nsfw.is_null())
    {
        fal.insert("has_nsfw_concepts".into(), nsfw.clone());
    }
    if let Some(prompt) = raw_body
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.is_empty())
    {
        fal.insert("prompt".into(), json!(prompt));
    }

    let mut metadata = aimux_core::shared::NamespaceMap::new();
    metadata.insert(options::NAMESPACE.into(), fal);
    metadata
}
