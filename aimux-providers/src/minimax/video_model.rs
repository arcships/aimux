//! MiniMax H3 video generation model (`/v2/video_generation`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError, ApiCallErrorOrigin};
use aimux_core::shared::Warning;
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFrameType, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResult,
};

use crate::shared::{EndpointConfig, task_failed, unsupported, video_response};

use super::image_probe::image_dimensions;
use super::options::{self, NAMESPACE, VideoOptions};
use super::{BaseResp, encode_path_segment, not_retryable_unless_rate_limited};

/// The models and their rules.
struct ModelSpec {
    id: &'static str,
    min_duration: u32,
    default_duration: u32,
    resolutions: &'static [&'static str],
    default_resolution: &'static str,
    /// Whether `extra.prompt_expansion_mode` is accepted.
    prompt_expansion: bool,
}

const MODELS: [ModelSpec; 4] = [
    ModelSpec {
        id: "MiniMax-H3",
        min_duration: 4,
        default_duration: 4,
        resolutions: &["768P", "2K"],
        default_resolution: "768P",
        prompt_expansion: false,
    },
    ModelSpec {
        id: "MiniMax-H3-Highspeed",
        min_duration: 4,
        default_duration: 4,
        resolutions: &["768P", "480P"],
        default_resolution: "480P",
        prompt_expansion: false,
    },
    ModelSpec {
        id: "MiniMax-H3-Lite",
        min_duration: 4,
        default_duration: 4,
        resolutions: &["768P", "480P"],
        default_resolution: "480P",
        prompt_expansion: false,
    },
    ModelSpec {
        id: "MiniMax-H3-Max",
        min_duration: 5,
        default_duration: 5,
        resolutions: &["768P", "480P"],
        default_resolution: "480P",
        prompt_expansion: true,
    },
];

const MAX_DURATION: u32 = 15;
const MAX_PROMPT_CHARS: usize = 7000;
/// The ratios of a text-only request; one with frames is `adaptive`.
const TEXT_RATIOS: [&str; 6] = ["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];
const DEFAULT_TEXT_RATIO: &str = "16:9";
const ADAPTIVE: &str = "adaptive";
const PROMPT_EXPANSION_MODES: [&str; 3] = ["disabled", "balanced", "quality"];
const MAX_FRAME_BYTES: usize = 30 * 1024 * 1024;
const FRAME_SIDE: std::ops::RangeInclusive<u32> = 256..=5760;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

fn spec(model_id: &str) -> Result<&'static ModelSpec, AiMuxError> {
    MODELS.iter().find(|m| m.id == model_id).ok_or_else(|| {
        AiMuxError::invalid_argument(format!(
            "MiniMax video model `{model_id}` is not supported; supported models: {}",
            MODELS.map(|m| m.id).join(", ")
        ))
    })
}

/// The failed-response handler: `{"type":"error","error":{"type","message",
/// "http_code"}}`, or the older `{"base_resp":{"status_code","status_msg"}}`.
pub(super) fn minimax_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        let base = data.get("base_resp");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|e| e.get("message"))
                .or_else(|| base.and_then(|b| b.get("status_msg")))
                .and_then(Value::as_str)
                .unwrap_or("MiniMax request failed")
                .to_string(),
            provider_code: error
                .and_then(|e| e.get("type"))
                .or_else(|| base.and_then(|b| b.get("status_code")))
                .and_then(crate::shared::json_error_code),
        }
    })
}

/// A failure MiniMax reports in a 2xx `base_resp`: retryable only for its
/// rate-limit code `1002`, and then only when `retry_rate_limit` is set.
pub(super) fn base_resp_error(
    code: i64,
    message: String,
    url: String,
    response_body: Option<String>,
    retry_rate_limit: bool,
) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(200),
        provider_code: Some(code.to_string()),
        response_body,
        is_retryable: retry_rate_limit && code == 1002,
        origin: ApiCallErrorOrigin::HttpResponse,
        ..ApiCallError::new(message, url, json!({}))
    }))
}

#[derive(Debug, Deserialize)]
struct CreateResponse {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    base_resp: Option<BaseResp>,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    /// `response_format: "task"`; a `flat` task is the body itself.
    #[serde(default)]
    task: Option<Task>,
    #[serde(flatten)]
    flat: Task,
    #[serde(default)]
    base_resp: Option<BaseResp>,
}

#[derive(Debug, Default, Deserialize)]
struct Task {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    content: Option<TaskContent>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    ratio: Option<Value>,
    #[serde(default)]
    resolution: Option<Value>,
    #[serde(default)]
    duration: Option<Value>,
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct TaskContent {
    #[serde(default)]
    url: Option<String>,
}

/// A MiniMax H3 video model — implements [`VideoModel`].
pub struct MiniMaxVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl MiniMaxVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// A frame as the `image_url.url` of a content item: an `http(s)` URL as
/// given, inline data as a `data:` URI, checked against the frame limits
/// when its bytes are at hand.
fn frame_url(file: &VideoFile, which: &str) -> Result<String, AiMuxError> {
    match file {
        VideoFile::Url { url, .. } => frame_url_string(url, which),
        VideoFile::File { media_type, data } => {
            let bytes = match data {
                aimux_core::video_model::VideoFileData::Binary(bytes) => bytes.clone(),
                aimux_core::video_model::VideoFileData::Base64(text) => decode_base64(text, which)?,
            };
            check_frame(media_type, &bytes, which)?;
            Ok(format!(
                "data:{media_type};base64,{}",
                aimux_provider_utils::convert_to_base64(data)
            ))
        }
    }
}

/// A frame given as a string: an `http(s)` URL, or a `data:` URI whose bytes
/// are checked.
fn frame_url_string(url: &str, which: &str) -> Result<String, AiMuxError> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (header, payload) = rest.split_once(',').ok_or_else(|| {
            AiMuxError::invalid_argument(format!("MiniMax {which}: malformed data URI"))
        })?;
        let media_type = header.strip_suffix(";base64").ok_or_else(|| {
            AiMuxError::invalid_argument(format!("MiniMax {which}: a data URI must be base64"))
        })?;
        check_frame(media_type, &decode_base64(payload, which)?, which)?;
        return Ok(url.to_string());
    }
    match url::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => Ok(url.to_string()),
        _ => Err(AiMuxError::invalid_argument(format!(
            "MiniMax {which} must be an http(s) URL or a data URI"
        ))),
    }
}

fn decode_base64(text: &str, which: &str) -> Result<Vec<u8>, AiMuxError> {
    aimux_provider_utils::convert_base64_to_uint8_array(text).map_err(|e| {
        AiMuxError::InvalidArgument {
            message: format!("MiniMax {which}: invalid base64: {e}"),
            cause: Some(aimux_core::ErrorCause::source(&e)),
        }
    })
}

/// 1 B to 30 MiB of an image type; sides 256–5760 px and an aspect ratio of
/// 0.4–2.5 when the header can be read.
fn check_frame(media_type: &str, bytes: &[u8], which: &str) -> Result<(), AiMuxError> {
    if !media_type.to_ascii_lowercase().starts_with("image/") {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax {which} must be an image, got `{media_type}`"
        )));
    }
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax {which} must be 1 byte to 30 MiB, got {} bytes",
            bytes.len()
        )));
    }
    if let Some((width, height)) = image_dimensions(bytes) {
        if !FRAME_SIDE.contains(&width) || !FRAME_SIDE.contains(&height) {
            return Err(AiMuxError::invalid_argument(format!(
                "MiniMax {which} must be 256 to 5760 px per side, got {width}x{height}"
            )));
        }
        // 0.4 <= width / height <= 2.5, in integers.
        let (w, h) = (u64::from(width), u64::from(height));
        if 5 * w < 2 * h || 2 * w > 5 * h {
            return Err(AiMuxError::invalid_argument(format!(
                "MiniMax {which} aspect ratio must be 0.4 to 2.5, got {width}x{height}"
            )));
        }
    }
    Ok(())
}

/// The request body of a create call, and the warnings for the call options
/// MiniMax does not take.
fn build_body(
    model_id: &str,
    options: &VideoCallOptions,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let spec = spec(model_id)?;
    let opts: VideoOptions = options::parse(&options.provider_options)?;
    let mut warnings = Vec::new();

    let prompt = options.prompt.as_deref().unwrap_or_default();
    let prompt_chars = prompt.chars().count();
    if prompt.trim().is_empty() || prompt_chars > MAX_PROMPT_CHARS {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax video prompt must be 1 to {MAX_PROMPT_CHARS} characters, got {prompt_chars}"
        )));
    }

    if options
        .input_references
        .as_ref()
        .is_some_and(|refs| !refs.is_empty())
    {
        return Err(AiMuxError::UnsupportedFunctionality(
            "MiniMax H3 reference inputs (inputReferences) are not supported; use a first or last frame"
                .to_string(),
        ));
    }

    // Frames: the first from `image` or `frameImages`, the last from
    // `frameImages` or `providerOptions.minimax.lastFrame`.
    let frame = |kind: VideoFrameType| {
        options
            .frame_images
            .iter()
            .flatten()
            .filter(move |f| f.frame_type == kind)
            .map(|f| &f.image)
    };
    let mut firsts: Vec<&VideoFile> = options
        .image
        .iter()
        .chain(frame(VideoFrameType::FirstFrame))
        .collect();
    let lasts: Vec<&VideoFile> = frame(VideoFrameType::LastFrame).collect();
    if firsts.len() > 1 {
        return Err(AiMuxError::invalid_argument(
            "MiniMax takes one first frame: pass it as `image` or one `first_frame` frame image",
        ));
    }
    if lasts.len() > 1 || (!lasts.is_empty() && opts.last_frame.is_some()) {
        return Err(AiMuxError::invalid_argument(
            "MiniMax takes one last frame: pass it as one `last_frame` frame image or providerOptions.minimax.lastFrame",
        ));
    }
    let first_url = firsts
        .pop()
        .map(|f| frame_url(f, "first frame"))
        .transpose()?;
    let last_url = match (lasts.first(), opts.last_frame.as_deref()) {
        (Some(file), _) => Some(frame_url(file, "last frame")?),
        (None, Some(url)) => Some(frame_url_string(url, "last frame")?),
        (None, None) => None,
    };
    let has_frames = first_url.is_some() || last_url.is_some();

    let duration = options.duration.unwrap_or(spec.default_duration);
    if !(spec.min_duration..=MAX_DURATION).contains(&duration) {
        return Err(AiMuxError::invalid_argument(format!(
            "{} duration must be {} to {MAX_DURATION} seconds, got {duration}",
            spec.id, spec.min_duration
        )));
    }

    if options.resolution.is_some() {
        warnings.push(unsupported(
            "resolution",
            "MiniMax takes a resolution tier; use providerOptions.minimax.resolution (480P, 768P or 2K).",
        ));
    }
    let resolution = opts
        .resolution
        .as_deref()
        .unwrap_or(spec.default_resolution);
    if !spec.resolutions.contains(&resolution) {
        return Err(AiMuxError::invalid_argument(format!(
            "{} resolution must be one of {}, got `{resolution}`",
            spec.id,
            spec.resolutions.join(", ")
        )));
    }

    let requested_ratio = opts
        .ratio
        .clone()
        .or_else(|| options.aspect_ratio.map(|ar| ar.to_string()));
    let ratio = if has_frames {
        match requested_ratio.as_deref() {
            None | Some(ADAPTIVE) => ADAPTIVE.to_string(),
            Some(other) => {
                return Err(AiMuxError::invalid_argument(format!(
                    "MiniMax ratio must be `adaptive` with a first or last frame, got `{other}`"
                )));
            }
        }
    } else {
        let ratio = requested_ratio.unwrap_or_else(|| DEFAULT_TEXT_RATIO.to_string());
        if !TEXT_RATIOS.contains(&ratio.as_str()) {
            return Err(AiMuxError::invalid_argument(format!(
                "MiniMax text-to-video ratio must be one of {}, got `{ratio}`",
                TEXT_RATIOS.join(", ")
            )));
        }
        ratio
    };

    if options.seed.is_some() {
        warnings.push(unsupported("seed", "MiniMax H3 does not take a seed."));
    }
    if options.fps.is_some() {
        warnings.push(unsupported("fps", "MiniMax H3 does not take a frame rate."));
    }

    let mut content = vec![json!({ "type": "text", "text": prompt })];
    for (url, role) in [(first_url, "first_frame"), (last_url, "last_frame")] {
        if let Some(url) = url {
            content.push(json!({
                "type": "image_url",
                "image_url": { "url": url },
                "role": role,
            }));
        }
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(spec.id));
    body.insert("content".into(), Value::Array(content));
    body.insert("resolution".into(), json!(resolution));
    body.insert("duration".into(), json!(duration));
    body.insert("ratio".into(), json!(ratio));
    body.insert("response_format".into(), json!("task"));
    if let Some(generate_audio) = options.generate_audio {
        body.insert("generate_audio".into(), json!(generate_audio));
    }
    if let Some(watermark) = opts.aigc_watermark {
        body.insert("aigc_watermark".into(), json!(watermark));
    }
    if let Some(mode) = opts.prompt_expansion_mode.as_deref() {
        if !spec.prompt_expansion {
            return Err(AiMuxError::invalid_argument(format!(
                "providerOptions.{NAMESPACE}.promptExpansionMode is only accepted by MiniMax-H3-Max"
            )));
        }
        if !PROMPT_EXPANSION_MODES.contains(&mode) {
            return Err(AiMuxError::invalid_argument(format!(
                "MiniMax promptExpansionMode must be one of {}, got `{mode}`",
                PROMPT_EXPANSION_MODES.join(", ")
            )));
        }
        body.insert("extra".into(), json!({ "prompt_expansion_mode": mode }));
    }

    let body = Value::Object(body);
    let size = serde_json::to_vec(&body).map_or(0, |bytes| bytes.len());
    if size > MAX_BODY_BYTES {
        return Err(AiMuxError::invalid_argument(format!(
            "MiniMax video request is {size} bytes, over the 64 MiB limit; pass large frames as URLs"
        )));
    }
    Ok((body, warnings))
}

/// The message of a failed task's `error` (an object with `message`, or a
/// string).
fn task_error_message(error: Option<&Value>) -> Option<String> {
    match error? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(obj) => obj
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

#[async_trait]
impl VideoModel for MiniMaxVideoModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_videos_per_call(&self) -> Option<u32> {
        Some(1)
    }

    /// Create the task. Its failures are not retryable except an HTTP 429:
    /// MiniMax may have created and billed the task before a transport error,
    /// a timeout or a 5xx reached us.
    async fn do_start(
        &self,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStart, AiMuxError> {
        let (body, warnings) = build_body(&self.model_id, options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url("/v2/video_generation");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(url.clone(), options),
            body,
            aimux_provider_utils::create_json_response_handler::<CreateResponse>(),
            minimax_failed_response_handler(),
        )
        .await
        .map_err(not_retryable_unless_rate_limited)?;

        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        if let Some((code, message)) = resp.value.base_resp.as_ref().and_then(BaseResp::failure) {
            return Err(base_resp_error(code, message, url, response_body, false));
        }
        let task_id = resp
            .value
            .task_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                AiMuxError::invalid_response_data(
                    "MiniMax video creation did not return a task_id; the task may still have been created",
                )
            })?;

        Ok(VideoOperationStart {
            operation: json!({ "taskId": task_id }),
            warnings,
            provider_metadata: None,
            response: video_response(
                aimux_core::Timestamp::now(),
                &self.model_id,
                resp.response_headers,
            ),
        })
    }

    /// Query the task. A transient failure (429, 5xx, transport) stays
    /// retryable: querying creates nothing.
    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let task_id = operation
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument("minimax operation reference is missing taskId")
            })?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!(
            "/v2/query/video_generation/{}",
            encode_path_segment(task_id)
        ));
        let resp = aimux_provider_utils::get_from_api(
            exchange.request(url.clone(), options),
            aimux_provider_utils::create_json_response_handler::<QueryResponse>(),
            minimax_failed_response_handler(),
        )
        .await?;
        let headers = resp.response_headers;
        let response_body = resp.raw_value.as_ref().map(ToString::to_string);
        let QueryResponse {
            task,
            flat,
            base_resp,
        } = resp.value;
        if let Some((code, message)) = base_resp.as_ref().and_then(BaseResp::failure) {
            return Err(base_resp_error(code, message, url, response_body, true));
        }
        let task = task.unwrap_or(flat);
        if task.id.as_deref() != Some(task_id) {
            return Err(AiMuxError::invalid_response_data(format!(
                "MiniMax task query for {task_id} returned task {:?}",
                task.id
            )));
        }

        let status = task.status.as_deref().unwrap_or_default();
        match status {
            "queued" | "running" => Ok(VideoOperationStatus::Pending),
            "succeeded" => {
                let video_url = task
                    .content
                    .as_ref()
                    .and_then(|c| c.url.as_deref())
                    .filter(|u| {
                        url::Url::parse(u).is_ok_and(|p| matches!(p.scheme(), "http" | "https"))
                    })
                    .ok_or_else(|| {
                        AiMuxError::invalid_response_data(format!(
                            "MiniMax task {task_id} succeeded without an http(s) video URL"
                        ))
                    })?
                    .to_string();
                let mut metadata = Map::new();
                metadata.insert("taskId".into(), json!(task_id));
                for (key, value) in [
                    ("ratio", task.ratio),
                    ("resolution", task.resolution),
                    ("duration", task.duration),
                    ("usage", task.usage),
                ] {
                    if let Some(value) = value {
                        metadata.insert(key.into(), value);
                    }
                }
                let mut provider_metadata = aimux_core::shared::SharedProviderMetadata::new();
                provider_metadata.insert(NAMESPACE.to_string(), metadata);
                Ok(VideoOperationStatus::Completed(VideoResult {
                    // Core downloads the URL without the provider headers.
                    videos: vec![VideoData::Url {
                        url: video_url,
                        media_type: "video/mp4".to_string(),
                    }],
                    warnings: Vec::new(),
                    provider_metadata: Some(provider_metadata),
                    response: video_response(aimux_core::Timestamp::now(), &self.model_id, headers),
                }))
            }
            "failed" | "cancelled" | "expired" => {
                let detail = task_error_message(task.error.as_ref());
                Err(task_failed(
                    match detail {
                        Some(detail) => format!("MiniMax video task {task_id} {status}: {detail}"),
                        None => format!("MiniMax video task {task_id} {status}"),
                    },
                    url,
                    status,
                    response_body,
                ))
            }
            other => Err(AiMuxError::invalid_response_data(format!(
                "MiniMax task {task_id} has an unknown status `{other}`"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::video_model::{VideoFileData, VideoFrameImage};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0]);
        data
    }

    fn file(bytes: Vec<u8>) -> VideoFile {
        VideoFile::File {
            media_type: "image/png".into(),
            data: VideoFileData::Binary(bytes),
        }
    }

    fn with_minimax(mut options: VideoCallOptions, value: Value) -> VideoCallOptions {
        let Value::Object(obj) = value else { panic!() };
        options.provider_options.insert(NAMESPACE.into(), obj);
        options
    }

    fn err(model: &str, options: &VideoCallOptions) -> String {
        build_body(model, options).unwrap_err().to_string()
    }

    #[test]
    fn text_to_video_uses_the_model_defaults() {
        let (body, warnings) = build_body("MiniMax-H3", &VideoCallOptions::new("a cat")).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "MiniMax-H3",
                "content": [{"type": "text", "text": "a cat"}],
                "resolution": "768P",
                "duration": 4,
                "ratio": "16:9",
                "response_format": "task",
            })
        );
        assert!(warnings.is_empty());
        let (body, _) = build_body("MiniMax-H3-Max", &VideoCallOptions::new("a cat")).unwrap();
        assert_eq!(body["duration"], 5);
        assert_eq!(body["resolution"], "480P");
    }

    #[test]
    fn unknown_models_and_bad_prompts_are_rejected() {
        assert!(err("MiniMax-H2", &VideoCallOptions::new("x")).contains("not supported"));
        assert!(err("MiniMax-H3", &VideoCallOptions::new("   ")).contains("1 to 7000"));
        let long = "猫".repeat(7001);
        assert!(err("MiniMax-H3", &VideoCallOptions::new(long)).contains("got 7001"));
        // 7000 multi-byte characters are within the limit.
        assert!(build_body("MiniMax-H3", &VideoCallOptions::new("猫".repeat(7000))).is_ok());
    }

    #[test]
    fn duration_and_resolution_follow_the_model() {
        let mut o = VideoCallOptions::new("x");
        o.duration = Some(4);
        assert!(err("MiniMax-H3-Max", &o).contains("5 to 15"));
        o.duration = Some(16);
        assert!(err("MiniMax-H3", &o).contains("4 to 15"));
        o.duration = Some(15);
        assert!(build_body("MiniMax-H3-Lite", &o).is_ok());

        let o = with_minimax(VideoCallOptions::new("x"), json!({"resolution": "2K"}));
        assert!(build_body("MiniMax-H3", &o).is_ok());
        assert!(err("MiniMax-H3-Highspeed", &o).contains("768P, 480P"));
        let o = with_minimax(VideoCallOptions::new("x"), json!({"resolution": "480P"}));
        assert!(err("MiniMax-H3", &o).contains("768P, 2K"));
    }

    #[test]
    fn text_to_video_needs_a_fixed_ratio() {
        let mut o = VideoCallOptions::new("x");
        o.aspect_ratio = Some("9:16".parse().unwrap());
        assert_eq!(build_body("MiniMax-H3", &o).unwrap().0["ratio"], "9:16");
        o.aspect_ratio = Some("2:1".parse().unwrap());
        assert!(err("MiniMax-H3", &o).contains("ratio must be one of"));
        let o = with_minimax(VideoCallOptions::new("x"), json!({"ratio": "adaptive"}));
        assert!(err("MiniMax-H3", &o).contains("ratio must be one of"));
    }

    #[test]
    fn frames_become_role_tagged_content_with_an_adaptive_ratio() {
        let mut o = VideoCallOptions::new("x");
        o.image = Some(file(png(1280, 720)));
        o.frame_images = Some(vec![VideoFrameImage {
            image: VideoFile::Url {
                url: "https://example.com/last.png".into(),
                media_type: None,
            },
            frame_type: VideoFrameType::LastFrame,
        }]);
        let (body, _) = build_body("MiniMax-H3", &o).unwrap();
        assert_eq!(body["ratio"], "adaptive");
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[1]["role"], "first_frame");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(content[2]["role"], "last_frame");
        assert_eq!(
            content[2]["image_url"]["url"],
            "https://example.com/last.png"
        );

        o.aspect_ratio = Some("16:9".parse().unwrap());
        assert!(err("MiniMax-H3", &o).contains("must be `adaptive`"));
    }

    #[test]
    fn the_last_frame_can_come_from_provider_options() {
        let data = aimux_provider_utils::convert_uint8_array_to_base64(png(720, 1280));
        let o = with_minimax(
            VideoCallOptions::new("x"),
            json!({"lastFrame": format!("data:image/png;base64,{data}")}),
        );
        let (body, _) = build_body("MiniMax-H3", &o).unwrap();
        assert_eq!(body["content"][1]["role"], "last_frame");

        let o = with_minimax(
            VideoCallOptions::new("x"),
            json!({"lastFrame": "file:///x.png"}),
        );
        assert!(err("MiniMax-H3", &o).contains("http(s) URL or a data URI"));
        let o = with_minimax(VideoCallOptions::new("x"), json!({"lastFrames": "x"}));
        assert!(err("MiniMax-H3", &o).contains("invalid minimax provider options"));
    }

    #[test]
    fn frame_limits_are_checked_when_the_bytes_are_known() {
        let mut o = VideoCallOptions::new("x");
        o.image = Some(file(png(200, 720)));
        assert!(err("MiniMax-H3", &o).contains("256 to 5760"));
        o.image = Some(file(png(3000, 1000)));
        assert!(err("MiniMax-H3", &o).contains("0.4 to 2.5"));
        o.image = Some(file(Vec::new()));
        assert!(err("MiniMax-H3", &o).contains("1 byte to 30 MiB"));
        o.image = Some(VideoFile::File {
            media_type: "video/mp4".into(),
            data: VideoFileData::Binary(vec![1]),
        });
        assert!(err("MiniMax-H3", &o).contains("must be an image"));
        // An unreadable header (HEIC, say) is left to MiniMax.
        o.image = Some(VideoFile::File {
            media_type: "image/heic".into(),
            data: VideoFileData::Binary(vec![0; 64]),
        });
        assert!(build_body("MiniMax-H3", &o).is_ok());
    }

    #[test]
    fn prompt_expansion_is_max_only() {
        let o = with_minimax(
            VideoCallOptions::new("x"),
            json!({"promptExpansionMode": "quality", "aigcWatermark": true}),
        );
        let (body, _) = build_body("MiniMax-H3-Max", &o).unwrap();
        assert_eq!(body["extra"], json!({"prompt_expansion_mode": "quality"}));
        assert_eq!(body["aigc_watermark"], true);
        assert!(err("MiniMax-H3", &o).contains("only accepted by MiniMax-H3-Max"));
        let o = with_minimax(
            VideoCallOptions::new("x"),
            json!({"promptExpansionMode": "balance"}),
        );
        assert!(err("MiniMax-H3-Max", &o).contains("disabled, balanced, quality"));
    }

    #[test]
    fn reference_inputs_are_refused_and_seed_warns() {
        let mut o = VideoCallOptions::new("x");
        o.seed = Some(1);
        o.fps = Some(24);
        let (_, warnings) = build_body("MiniMax-H3", &o).unwrap();
        assert_eq!(warnings.len(), 2);
        o.input_references = Some(vec![file(png(512, 512))]);
        assert!(matches!(
            build_body("MiniMax-H3", &o),
            Err(AiMuxError::UnsupportedFunctionality(_))
        ));
    }
}
