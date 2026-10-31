//! xAI video model.
//!
//! Aligned with `XaiVideoModel`
//! (`reference/aisdk-pinned/xai/src/xai-video-model.ts`).
//!
//! `do_start` posts to `/videos/generations`, `/videos/edits` or
//! `/videos/extensions` (by mode) and returns the `request_id`; `do_status`
//! polls `GET /videos/{request_id}`.

use async_trait::async_trait;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::shared::{JsonObject, Warning};
use aimux_core::video_model::{
    VideoCallOptions, VideoData, VideoFile, VideoFrameType, VideoModel, VideoOperationStart,
    VideoOperationStatus, VideoResponse, VideoResult,
};
use aimux_provider_utils::get_top_level_media_type;

use super::options::{self, xai_metadata};
use crate::shared::EndpointConfig;

/// An xAI video model (`grok-imagine-video`, `grok-imagine-video-1.5`, ...).
pub struct XaiVideoModel {
    model_id: String,
    config: EndpointConfig,
}

impl XaiVideoModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

const RESOLUTION_MAP: &[(&str, &str)] = &[
    ("1920x1080", "1080p"),
    ("1280x720", "720p"),
    ("854x480", "480p"),
    ("640x480", "480p"),
];

/// `encodeURIComponent`'s unreserved set.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// Dot segments are normalized by URL parsing, so they are encoded twice.
fn encode_path_segment(value: &str) -> String {
    match utf8_percent_encode(value, URI_COMPONENT)
        .to_string()
        .as_str()
    {
        "." => "%252E".to_owned(),
        ".." => "%252E%252E".to_owned(),
        encoded => encoded.to_owned(),
    }
}

fn media_type(file: &VideoFile) -> Option<&str> {
    match file {
        VideoFile::File { media_type, .. } => Some(media_type),
        VideoFile::Url { media_type, .. } => media_type.as_deref(),
    }
}

fn top_level(file: &VideoFile) -> Option<&str> {
    media_type(file).map(get_top_level_media_type)
}

fn is_video(file: &VideoFile) -> bool {
    top_level(file) == Some("video")
}

/// References without a media type (only possible for URLs) count as images.
fn is_image_reference(file: &VideoFile) -> bool {
    matches!(top_level(file), None | Some("image"))
}

fn is_audio_reference(file: &VideoFile) -> bool {
    top_level(file) == Some("audio")
}

fn file_to_xai_url(file: &VideoFile) -> String {
    match file {
        VideoFile::Url { url, .. } => url.clone(),
        VideoFile::File { media_type, data } => {
            let base64 = aimux_provider_utils::convert_to_base64(data).into_owned();
            format!("data:{media_type};base64,{base64}")
        }
    }
}

fn unsupported(warnings: &mut Vec<Warning>, feature: &str, details: impl Into<String>) {
    warnings.push(Warning::Unsupported {
        feature: feature.to_owned(),
        details: Some(details.into()),
    });
}

/// `xaiVideoModelOptionsSchema` (`runtimeSchema`, a `z.looseObject`): the
/// keys it declares, checked; the rest is forwarded from the raw namespace.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoOptions {
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    mode: Option<VideoMode>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    video_url: Option<options::de::NonEmpty>,
    /// `z.array(nonEmptyString).min(1).max(7).optional()`.
    #[serde(default, deserialize_with = "reference_image_urls")]
    reference_image_urls: Option<Vec<String>>,
    /// `z.array(nonEmptyString).max(3).optional()`.
    #[serde(default, deserialize_with = "reference_voice_ids")]
    reference_voice_ids: Option<Vec<String>>,
    /// `z.array(keyframeSchema).max(4).optional()`.
    #[serde(default, deserialize_with = "keyframes")]
    keyframes: Option<Vec<Keyframe>>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    storage_options: Option<StorageOptions>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    user: Option<String>,
    #[serde(default, deserialize_with = "options::de::nullish_positive")]
    #[allow(
        dead_code,
        reason = "validated only; the poll settings read the raw value"
    )]
    poll_interval_ms: Option<f64>,
    #[serde(default, deserialize_with = "options::de::nullish_positive")]
    #[allow(
        dead_code,
        reason = "validated only; the poll settings read the raw value"
    )]
    poll_timeout_ms: Option<f64>,
    #[serde(default)]
    resolution: Option<Resolution>,
}

/// `modeSchema`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum VideoMode {
    EditVideo,
    ExtendVideo,
    ReferenceToVideo,
}

impl VideoMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::EditVideo => "edit-video",
            Self::ExtendVideo => "extend-video",
            Self::ReferenceToVideo => "reference-to-video",
        }
    }
}

/// `resolutionSchema`.
#[derive(Debug, Clone, Copy, Deserialize)]
enum Resolution {
    #[serde(rename = "480p")]
    P480,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
}

impl Resolution {
    fn as_str(self) -> &'static str {
        match self {
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
        }
    }
}

/// `keyframeSchema`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Keyframe {
    image_url: options::de::NonEmpty,
    #[serde(deserialize_with = "positive")]
    timestamp_seconds: f64,
}

/// `storageOptionsSchema`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageOptions {
    filename: options::de::NonEmpty,
    #[serde(default, deserialize_with = "options::de::storage_expires_after")]
    expires_after: Option<i64>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    public_url: Option<PublicUrl>,
}

/// `z.union([z.boolean(), z.object({ expiresAfter })])`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PublicUrl {
    Enabled(bool),
    Expiring(PublicUrlOptions),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicUrlOptions {
    #[serde(default, deserialize_with = "options::de::public_url_expires_after")]
    expires_after: Option<i64>,
}

impl StorageOptions {
    /// The wire shape (`storage_options`).
    fn wire(self) -> Value {
        let mut out = Map::new();
        out.insert("filename".into(), json!(self.filename.0));
        if let Some(expires_after) = self.expires_after {
            out.insert("expires_after".into(), json!(expires_after));
        }
        match self.public_url {
            None => {}
            Some(PublicUrl::Enabled(enabled)) => {
                out.insert("public_url".into(), json!(enabled));
            }
            Some(PublicUrl::Expiring(public)) => {
                let mut wire = Map::new();
                if let Some(expires_after) = public.expires_after {
                    wire.insert("expires_after".into(), json!(expires_after));
                }
                out.insert("public_url".into(), Value::Object(wire));
            }
        }
        Value::Object(out)
    }
}

/// `z.number().positive()`.
fn positive<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(d)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("expected a number > 0"))
    }
}

fn reference_image_urls<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<String>>, D::Error> {
    options::de::non_empty_strings(d, 1, 7)
}

fn reference_voice_ids<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Vec<String>>, D::Error> {
    options::de::non_empty_strings(d, 0, 3)
}

fn keyframes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<Keyframe>>, D::Error> {
    let items = Vec::<Keyframe>::deserialize(d)?;
    if items.len() > 4 {
        return Err(serde::de::Error::custom(
            "too big: expected array to have <=4 items",
        ));
    }
    Ok(Some(items))
}

/// The parsed video options, in the shape the request builder reads.
#[derive(Default)]
struct Opts {
    mode: Option<String>,
    video_url: Option<String>,
    reference_image_urls: Option<Vec<String>>,
    reference_voice_ids: Option<Vec<String>>,
    keyframes: Vec<(String, f64)>,
    storage_options: Option<Value>,
    user: Option<String>,
    resolution: Option<String>,
}

impl Opts {
    /// `parseProviderOptions({ provider: 'xai', schema: xaiVideoModelOptionsSchema })`.
    fn parse(
        provider_options: &aimux_core::shared::SharedProviderOptions,
    ) -> Result<Self, AiMuxError> {
        let xai: VideoOptions = options::parse(Some(provider_options))?.unwrap_or_default();
        Ok(Self {
            mode: xai.mode.map(|mode| mode.as_str().to_owned()),
            video_url: xai.video_url.map(|url| url.0),
            reference_image_urls: xai.reference_image_urls,
            reference_voice_ids: xai.reference_voice_ids,
            keyframes: xai
                .keyframes
                .unwrap_or_default()
                .into_iter()
                .map(|frame| (frame.image_url.0, frame.timestamp_seconds))
                .collect(),
            storage_options: xai.storage_options.map(StorageOptions::wire),
            user: xai.user,
            resolution: xai
                .resolution
                .map(|resolution| resolution.as_str().to_owned()),
        })
    }
}

/// Keys of the `xai` options that are not forwarded as body fields.
const OWN_KEYS: &[&str] = &[
    "mode",
    "pollIntervalMs",
    "pollTimeoutMs",
    "resolution",
    "videoUrl",
    "referenceImageUrls",
    "referenceVoiceIds",
    "keyframes",
    "storageOptions",
    "user",
];

/// `buildRequestBody`: the endpoint path, the body and the warnings.
fn build_request(
    model_id: &str,
    options: &VideoCallOptions,
) -> Result<(&'static str, Value, Vec<Warning>), AiMuxError> {
    let mut warnings = Vec::new();
    let empty = JsonObject::new();
    let xai_raw = options::xai_options(Some(&options.provider_options)).unwrap_or(&empty);
    let xai = Opts::parse(&options.provider_options)?;
    let references = options.input_references.as_deref().unwrap_or_default();
    let has_image_ref = references.iter().any(is_image_reference);
    let has_audio_ref = references.iter().any(is_audio_reference);

    let legacy_refs = xai
        .reference_image_urls
        .as_ref()
        .is_some_and(|u| !u.is_empty());
    let mode = match xai.mode.as_deref() {
        Some(mode) => Some(mode),
        None if xai.video_url.is_some() => Some("edit-video"),
        // Video-only references must not flip a standard generation into R2V.
        None if has_image_ref || has_audio_ref || legacy_refs => Some("reference-to-video"),
        None => None,
    };
    let is_edit = mode == Some("edit-video");
    let is_extension = mode == Some("extend-video");
    let has_reference_images = mode == Some("reference-to-video");

    if options.fps.is_some() {
        unsupported(
            &mut warnings,
            "fps",
            "xAI video models do not support custom FPS.",
        );
    }
    if options.seed.is_some() {
        unsupported(
            &mut warnings,
            "seed",
            "xAI video models do not support seed.",
        );
    }
    if options.n > 1 {
        unsupported(
            &mut warnings,
            "n",
            "xAI video models do not support generating multiple videos per call. \
             Only 1 video will be generated.",
        );
    }
    let has_resolution = xai.resolution.is_some() || options.resolution.is_some();
    if is_edit {
        if options.duration.is_some() {
            unsupported(
                &mut warnings,
                "duration",
                "xAI video editing does not support custom duration.",
            );
        }
        if options.aspect_ratio.is_some() {
            unsupported(
                &mut warnings,
                "aspectRatio",
                "xAI video editing does not support custom aspect ratio.",
            );
        }
        if has_resolution {
            unsupported(
                &mut warnings,
                "resolution",
                "xAI video editing does not support custom resolution.",
            );
        }
    }
    if is_extension {
        if options.aspect_ratio.is_some() {
            unsupported(
                &mut warnings,
                "aspectRatio",
                "xAI video extension does not support custom aspect ratio.",
            );
        }
        if has_resolution {
            unsupported(
                &mut warnings,
                "resolution",
                "xAI video extension does not support custom resolution.",
            );
        }
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    if let Some(prompt) = &options.prompt {
        body.insert("prompt".into(), json!(prompt));
    }
    let allow_resolution = !is_edit && !is_extension;
    if !is_edit && let Some(duration) = options.duration {
        body.insert("duration".into(), json!(duration));
    }
    if allow_resolution && let Some(ratio) = options.aspect_ratio {
        body.insert("aspect_ratio".into(), json!(ratio.to_string()));
    }
    if allow_resolution && let Some(resolution) = &xai.resolution {
        body.insert("resolution".into(), json!(resolution));
    } else if allow_resolution && let Some(size) = options.resolution {
        let size = size.to_string();
        match RESOLUTION_MAP.iter().find(|(from, _)| *from == size) {
            Some((_, mapped)) => {
                body.insert("resolution".into(), json!(mapped));
            }
            None => unsupported(
                &mut warnings,
                "resolution",
                format!(
                    "Unrecognized resolution \"{size}\". Use providerOptions.xai.resolution \
                     with \"480p\", \"720p\", or \"1080p\" instead."
                ),
            ),
        }
    }
    if let Some(generate_audio) = options.generate_audio {
        if allow_resolution {
            body.insert("generate_audio".into(), json!(generate_audio));
        } else {
            unsupported(
                &mut warnings,
                "generateAudio",
                format!(
                    "xAI {} does not support generateAudio.",
                    if is_edit {
                        "video editing"
                    } else {
                        "video extension"
                    }
                ),
            );
        }
    }
    if let Some(storage) = xai.storage_options {
        body.insert("storage_options".into(), storage);
    }

    let is_v15 = model_id == "grok-imagine-video-1.5";
    if !xai.keyframes.is_empty() {
        if !is_v15 || is_edit || is_extension {
            unsupported(
                &mut warnings,
                "keyframes",
                if !is_v15 {
                    "xAI only supports keyframes with \"grok-imagine-video-1.5\".".to_owned()
                } else {
                    format!(
                        "xAI {} does not support keyframes.",
                        if is_edit {
                            "video editing"
                        } else {
                            "video extension"
                        }
                    )
                },
            );
        } else {
            body.insert(
                "keyframes".into(),
                Value::Array(
                    xai.keyframes
                        .iter()
                        .map(|(url, at)| json!({ "image": { "url": url }, "timestamp_s": at }))
                        .collect(),
                ),
            );
        }
    }

    // Edit and extension pass the source video as a nested object.
    if is_edit || is_extension {
        body.insert(
            "video".into(),
            xai.video_url
                .as_ref()
                .map_or_else(|| json!({}), |url| json!({ "url": url })),
        );
    }

    let frame = |kind: VideoFrameType| {
        options
            .frame_images
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|f| f.frame_type == kind)
            .map(|f| &f.image)
    };
    let first_frame = frame(VideoFrameType::FirstFrame);
    if let Some(start) = first_frame.or(options.image.as_ref()) {
        if is_video(start) {
            unsupported(
                &mut warnings,
                if first_frame.is_some() {
                    "frameImages"
                } else {
                    "image"
                },
                "xAI does not accept a video as a start/frame image. The video was ignored. \
                 Use providerOptions.xai.mode \"extend-video\" to continue from a video instead.",
            );
        } else {
            body.insert("image".into(), json!({ "url": file_to_xai_url(start) }));
        }
    }
    // Only grok-imagine-video-1.5 supports a pinned last frame.
    if let Some(last) = frame(VideoFrameType::LastFrame) {
        if !is_v15 || is_edit || is_extension || is_video(last) {
            unsupported(
                &mut warnings,
                "frameImages",
                if !is_v15 {
                    "xAI only supports last_frame with \"grok-imagine-video-1.5\". The last frame was ignored."
                } else {
                    "xAI only accepts an image last_frame for video generation. The last frame was ignored."
                },
            );
        } else {
            body.insert("last_frame".into(), json!({ "url": file_to_xai_url(last) }));
        }
    }

    if has_reference_images {
        // First-class `inputReferences` win over the legacy `referenceImageUrls`.
        let reference_images: Option<Vec<Value>> = if !references.is_empty() {
            let mut images = Vec::new();
            for reference in references.iter().filter(|r| !is_audio_reference(r)) {
                if is_image_reference(reference) {
                    images.push(json!({ "url": file_to_xai_url(reference) }));
                } else {
                    unsupported(
                        &mut warnings,
                        "inputReferences",
                        "xAI reference-to-video does not accept video references. The video \
                         reference was ignored. Use providerOptions.xai.mode \"extend-video\" \
                         to continue from a video.",
                    );
                }
            }
            Some(images).filter(|images| !images.is_empty())
        } else {
            xai.reference_image_urls
                .as_ref()
                .filter(|urls| !urls.is_empty())
                .map(|urls| urls.iter().map(|url| json!({ "url": url })).collect())
        };
        let mut audio_inputs: Vec<Value> = references
            .iter()
            .filter(|r| is_audio_reference(r))
            .map(|r| json!({ "url": file_to_xai_url(r) }))
            .collect();
        let no_audio_refs = audio_inputs.is_empty();
        match reference_images {
            Some(images) => {
                body.insert("reference_images".into(), Value::Array(images));
            }
            // Explicit R2V with no usable image would silently send a plain request.
            None if no_audio_refs => unsupported(
                &mut warnings,
                "referenceImages",
                "xAI reference-to-video requires at least one image reference. \
                 The video will be generated without reference images.",
            ),
            None => {}
        }
        audio_inputs.extend(
            xai.reference_voice_ids
                .iter()
                .flatten()
                .map(|id| json!({ "voice_id": id })),
        );
        if !audio_inputs.is_empty() {
            if audio_inputs.len() > 3 {
                unsupported(
                    &mut warnings,
                    "inputReferences",
                    "xAI reference-to-video supports at most 3 audio references. Only the first 3 were used.",
                );
            }
            audio_inputs.truncate(3);
            body.insert("reference_audios".into(), Value::Array(audio_inputs));
        }
        // Reference-to-video is capped at 720p.
        if body.get("resolution").and_then(Value::as_str) == Some("1080p") {
            unsupported(
                &mut warnings,
                "resolution",
                "xAI reference-to-video is limited to 720p. The request was downgraded from 1080p to 720p.",
            );
            body.insert("resolution".into(), json!("720p"));
        }
    }

    // 1080p needs grok-imagine-video-1.5; warn, but send what the user asked.
    if body.get("resolution").and_then(Value::as_str) == Some("1080p")
        && model_id == "grok-imagine-video"
    {
        unsupported(
            &mut warnings,
            "resolution",
            "xAI model \"grok-imagine-video\" does not support 1080p. Use \"grok-imagine-video-1.5\" \
             for 1080p, or a lower resolution. The request was sent with 1080p.",
        );
    }
    if !references.is_empty() && !has_reference_images {
        unsupported(
            &mut warnings,
            "inputReferences",
            if has_image_ref || has_audio_ref {
                "xAI only supports inputReferences for reference-to-video generation. \
                 The references were ignored."
            } else {
                "xAI reference-to-video requires at least one image or audio reference. \
                 The references were ignored."
            },
        );
    }
    if xai
        .reference_voice_ids
        .as_ref()
        .is_some_and(|ids| !ids.is_empty())
        && !has_reference_images
    {
        unsupported(
            &mut warnings,
            "referenceVoiceIds",
            "xAI only supports reference voices for reference-to-video generation. \
             The reference voices were ignored.",
        );
    }
    if !is_extension && let Some(user) = xai.user {
        body.insert("user".into(), json!(user));
    }
    for (key, value) in xai_raw {
        if !OWN_KEYS.contains(&key.as_str()) {
            body.insert(key.clone(), value.clone());
        }
    }

    let path = if is_edit {
        "/videos/edits"
    } else if is_extension {
        "/videos/extensions"
    } else {
        "/videos/generations"
    };
    Ok((path, Value::Object(body), warnings))
}

#[derive(Deserialize)]
struct CreateResponse {
    request_id: Option<String>,
}

#[derive(Default, Deserialize)]
struct StatusResponse {
    status: Option<String>,
    video: Option<StatusVideo>,
    usage: Option<StatusUsage>,
    progress: Option<f64>,
    error: Option<StatusError>,
}

#[derive(Deserialize)]
struct StatusVideo {
    url: Option<String>,
    duration: Option<f64>,
    respect_moderation: Option<bool>,
    file_output: Option<FileOutput>,
    storage_error: Option<String>,
}

#[derive(Deserialize)]
struct FileOutput {
    file_id: String,
    filename: String,
    expires_at: Option<f64>,
    public_url: Option<String>,
    public_url_error: Option<String>,
    public_url_expires_at: Option<f64>,
}

#[derive(Deserialize)]
struct StatusUsage {
    cost_in_usd_ticks: Option<f64>,
}

#[derive(Deserialize)]
struct StatusError {
    code: Option<String>,
    message: Option<String>,
}

/// Generous bound for a `{status, progress}` payload of ~50 bytes.
const MAX_PENDING_BODY_BYTES: usize = 1024 * 1024;

/// xAI answers 202 while a generation is still running, sometimes with an
/// empty body: that is `pending`, whatever the body holds.
fn status_response_handler() -> aimux_provider_utils::ResponseHandler<StatusResponse> {
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        if input.response.status().as_u16() != 202 {
            return aimux_provider_utils::create_json_response_handler::<StatusResponse>()
                .handle(input)
                .await;
        }
        let headers = aimux_provider_utils::extract_response_headers::extract_response_headers(
            input.response.headers(),
        );
        let body =
            aimux_provider_utils::read_response_with_size_limit::read_response_with_size_limit(
                input.response,
                &input.url,
                &input.request_body_values,
                MAX_PENDING_BODY_BYTES,
                input.abort_signal.as_ref(),
            )
            .await?;
        let value = serde_json::from_slice::<StatusResponse>(&body).unwrap_or(StatusResponse {
            status: Some("pending".into()),
            ..StatusResponse::default()
        });
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: headers,
        })
    })
}

#[async_trait]
impl VideoModel for XaiVideoModel {
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
        let (path, body, warnings) = build_request(&self.model_id, options)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url(path), options),
            body,
            aimux_provider_utils::create_json_response_handler::<CreateResponse>(),
            super::error::xai_failed_response_handler(),
        )
        .await?;
        let request_id = resp
            .value
            .request_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AiMuxError::other("No request_id returned from xAI API."))?;
        Ok(VideoOperationStart {
            operation: json!({ "requestId": request_id }),
            warnings,
            provider_metadata: None,
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(resp.response_headers),
            },
        })
    }

    async fn do_status(
        &self,
        operation: &Value,
        options: &VideoCallOptions,
    ) -> Result<VideoOperationStatus, AiMuxError> {
        let request_id = operation
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiMuxError::invalid_argument("xai operation reference is missing requestId")
            })?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let url = exchange.url(&format!("/videos/{}", encode_path_segment(request_id)));
        let resp = aimux_provider_utils::get_from_api(
            exchange.request(url.clone(), options),
            status_response_handler(),
            super::error::xai_failed_response_handler(),
        )
        .await?;
        let headers = resp.response_headers;
        let status = resp.value;

        // A terminal failure is reported as a non-retryable error so the poll
        // loop stops (the AI SDK's `status: 'error'` result).
        let terminal = |message: String| {
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(200),
                is_retryable: false,
                ..ApiCallError::new(message, url.clone(), json!({}))
            }))
        };

        match status.status.as_deref() {
            Some("expired") => return Err(terminal("Video generation request expired.".into())),
            Some("failed") => {
                let details = status.error.and_then(|e| e.message.or(e.code));
                return Err(terminal(match details {
                    Some(details) => format!("Video generation failed: {details}"),
                    None => "Video generation failed.".into(),
                }));
            }
            _ => {}
        }

        let done = status.status.as_deref() == Some("done")
            || (status.status.is_none()
                && status
                    .video
                    .as_ref()
                    .and_then(|v| v.url.as_deref())
                    .is_some_and(|u| !u.is_empty()));
        if !done {
            return Ok(VideoOperationStatus::Pending);
        }

        let video = status.video;
        if video.as_ref().and_then(|v| v.respect_moderation) == Some(false) {
            return Err(terminal(
                "Video generation was blocked due to a content policy violation.".into(),
            ));
        }
        let video_url = video
            .as_ref()
            .and_then(|v| {
                v.url
                    .clone()
                    .or_else(|| v.file_output.as_ref()?.public_url.clone())
            })
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                terminal("Video generation completed but no video URL was returned.".into())
            })?;

        let mut metadata = Map::new();
        metadata.insert("requestId".into(), json!(request_id));
        metadata.insert("videoUrl".into(), json!(video_url));
        if let Some(duration) = video.as_ref().and_then(|v| v.duration) {
            metadata.insert("duration".into(), json!(duration));
        }
        if let Some(cost) = status.usage.and_then(|u| u.cost_in_usd_ticks) {
            metadata.insert("costInUsdTicks".into(), json!(cost));
        }
        if let Some(progress) = status.progress {
            metadata.insert("progress".into(), json!(progress));
        }
        if let Some(video) = &video {
            if let Some(file) = &video.file_output {
                let mut out = Map::new();
                out.insert("fileId".into(), json!(file.file_id));
                out.insert("filename".into(), json!(file.filename));
                for (key, value) in [
                    ("expiresAt", file.expires_at.map(|v| json!(v))),
                    ("publicUrl", file.public_url.as_ref().map(|v| json!(v))),
                    (
                        "publicUrlError",
                        file.public_url_error.as_ref().map(|v| json!(v)),
                    ),
                    (
                        "publicUrlExpiresAt",
                        file.public_url_expires_at.map(|v| json!(v)),
                    ),
                ] {
                    if let Some(value) = value {
                        out.insert(key.into(), value);
                    }
                }
                metadata.insert("fileOutput".into(), Value::Object(out));
            }
            if let Some(error) = &video.storage_error {
                metadata.insert("storageError".into(), json!(error));
            }
        }

        Ok(VideoOperationStatus::Completed(VideoResult {
            videos: vec![VideoData::Url {
                url: video_url,
                media_type: "video/mp4".into(),
            }],
            warnings: Vec::new(),
            provider_metadata: Some(xai_metadata(Value::Object(metadata))),
            response: VideoResponse {
                timestamp: Some(aimux_core::Timestamp::now()),
                model_id: Some(self.model_id.clone()),
                headers: Some(headers),
            },
        }))
    }
}
