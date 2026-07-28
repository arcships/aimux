//! The `VideoModel` trait — the provider-facing interface for video generation.
//!
//! Aligned with Vercel AI SDK `VideoModelV4`
//! (`reference/ai/packages/provider/src/video-model/v4/`).

use async_trait::async_trait;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, AspectRatio, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Size,
    Warning,
};

/// A video or image file used for video editing or image-to-video generation.
///
/// Aligned with V4 `VideoModelV4File`.
#[derive(Debug, Clone)]
pub enum VideoFile {
    /// Inline file data (base64 or binary) with an explicit media type.
    File {
        /// IANA media type, e.g. `"video/mp4"` or `"image/png"`.
        media_type: String,
        /// File data as a base64 string or binary bytes.
        data: VideoFileData,
    },
    /// A URL pointing to the file.
    Url {
        /// The URL of the video or image file.
        url: String,
        /// The media type of the referenced file, when known.
        media_type: Option<String>,
    },
}

/// File payload for a [`VideoFile::File`]: base64 string or raw bytes.
#[derive(Debug, Clone)]
pub enum VideoFileData {
    /// Base64-encoded string.
    Base64(String),
    /// Raw binary bytes.
    Binary(Vec<u8>),
}

/// The role a frame image plays in video generation.
///
/// Aligned with V4 `VideoModelV4FrameType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFrameType {
    /// The starting frame the model animates from.
    FirstFrame,
    /// The ending frame the model animates towards.
    LastFrame,
}

/// A role-tagged image input for image-to-video and first-last-frame generation.
///
/// Aligned with V4 `VideoModelV4FrameImage`.
#[derive(Debug, Clone)]
pub struct VideoFrameImage {
    /// The image file used for this frame.
    pub image: VideoFile,
    /// Which frame this image represents.
    pub frame_type: VideoFrameType,
}

/// Generated video data: a URL, base64-encoded string, or binary data.
///
/// Aligned with V4 `VideoModelV4VideoData`. Most providers return URLs due to
/// large file sizes.
#[derive(Debug, Clone)]
pub enum VideoData {
    /// Video available as a URL (most common).
    Url { url: String, media_type: String },
    /// Video as a base64-encoded string.
    Base64 { data: String, media_type: String },
    /// Video as binary data.
    Binary { data: Vec<u8>, media_type: String },
}

/// Options passed to [`VideoModel::do_generate`].
///
/// Aligned with V4 `VideoModelV4CallOptions`.
#[derive(Debug, Clone)]
pub struct VideoCallOptions {
    /// Text prompt for the video generation. `None` when not required.
    pub prompt: Option<String>,

    /// Number of videos to generate. Default `1`; most models only support
    /// `n = 1` due to computational cost.
    pub n: u32,

    /// Aspect ratio, in `{width}:{height}` format (e.g. `"16:9"`).
    pub aspect_ratio: Option<AspectRatio>,

    /// Resolution, in `{width}x{height}` format (e.g. `"1280x720"`).
    pub resolution: Option<Size>,

    /// Duration of the video in seconds. Typically 3–10 seconds.
    pub duration: Option<u32>,

    /// Frames per second. Common values: 24, 30, 60.
    pub fps: Option<u32>,

    /// Seed for deterministic generation. `None` uses a random seed.
    pub seed: Option<u64>,

    /// Input image for image-to-video generation (the starting frame).
    pub image: Option<VideoFile>,

    /// Role-tagged image inputs for first-last-frame generation.
    pub frame_images: Option<Vec<VideoFrameImage>>,

    /// Reference inputs for reference-to-video generation (images or videos).
    pub input_references: Option<Vec<VideoFile>>,

    /// Whether the model should generate audio alongside the video.
    pub generate_audio: Option<bool>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: SharedProviderOptions,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl VideoCallOptions {
    /// Create options with a prompt and `n = 1`, all other fields defaulted.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            n: 1,
            aspect_ratio: None,
            resolution: None,
            duration: None,
            fps: None,
            seed: None,
            image: None,
            frame_images: None,
            input_references: None,
            generate_audio: None,
            provider_options: SharedProviderOptions::new(),
            abort_signal: None,
            headers: None,
        }
    }
}

/// The result of [`VideoModel::do_generate`].
///
/// Aligned with V4 `VideoModelV4Result`.
#[derive(Debug, Clone)]
pub struct VideoResult {
    /// Generated videos as URLs, base64 strings, or binary data.
    pub videos: Vec<VideoData>,

    /// Warnings for the call, e.g. unsupported features.
    pub warnings: Vec<Warning>,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Response information for telemetry and debugging.
    pub response: VideoResponse,
}

/// Response information for a video generation call.
#[derive(Debug, Clone, Default)]
pub struct VideoResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
}

/// The unified video generation model trait (provider-facing).
///
/// Aligned with V4 `VideoModelV4`.
#[async_trait]
pub trait VideoModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"fal"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"kling-video"`.
    fn model_id(&self) -> &str;

    /// Limit of how many videos can be generated in a single API call.
    ///
    /// `None` means no fixed limit. Most video models only support `1`.
    fn max_videos_per_call(&self) -> Option<u32>;

    /// Generate an array of videos.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_generate(&self, options: &VideoCallOptions) -> Result<VideoResult, AiMuxError>;
}
