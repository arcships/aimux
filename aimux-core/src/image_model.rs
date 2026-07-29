//! The `ImageModel` trait — the provider-facing interface for image generation.
//!
//! Aligned with Vercel AI SDK `ImageModelV4`
//! (`reference/ai/packages/provider/src/image-model/v4/`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, AspectRatio, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Size,
    Warning,
};

/// An image file used for image editing or variation generation.
///
/// Aligned with V4 `ImageModelV4File`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ImageFile {
    /// Inline file data (base64 or binary) with an explicit media type.
    File {
        /// IANA media type, e.g. `"image/png"`.
        media_type: String,
        /// File data as base64 string or binary bytes.
        data: ImageFileData,
    },
    /// A URL pointing to the image.
    Url {
        /// The URL of the image file.
        url: String,
    },
}

/// File payload for an [`ImageFile::File`]: base64 string or raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ImageFileData {
    /// Base64-encoded string.
    Base64(String),
    /// Raw binary bytes.
    Binary(Vec<u8>),
}

/// The generated images, returned as a homogeneous collection.
///
/// The TS result type is `Array<string> | Array<Uint8Array>` — i.e. either all
/// base64 strings or all binary blobs, never mixed. This enum models that
/// union. Providers should return data without unnecessary conversion: if the
/// upstream API returns base64, return [`ImageOutputs::Base64`]; if binary,
/// return [`ImageOutputs::Binary`].
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ImageOutputs {
    /// Images as base64-encoded strings.
    Base64(Vec<String>),
    /// Images as raw binary bytes.
    Binary(Vec<Vec<u8>>),
}

/// Token usage for an image generation call (if the provider reports it).
///
/// Aligned with V4 `ImageModelV4Usage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImageUsage {
    /// Number of input (prompt) tokens used.
    pub input_tokens: Option<u32>,
    /// Number of output tokens used, if reported.
    pub output_tokens: Option<u32>,
    /// Total tokens as reported by the provider.
    pub total_tokens: Option<u32>,
}

/// Options passed to [`ImageModel::do_generate`].
///
/// Aligned with V4 `ImageModelV4CallOptions`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImageCallOptions {
    /// Prompt for the image generation. `None` for operations (e.g. upscaling)
    /// that do not require a prompt.
    pub prompt: Option<String>,

    /// Number of images to generate.
    pub n: u32,

    /// Size of the images, in `{width}x{height}` format.
    /// `None` uses the provider's default size.
    pub size: Option<Size>,

    /// Aspect ratio of the images, in `{width}:{height}` format.
    /// `None` uses the provider's default aspect ratio.
    pub aspect_ratio: Option<AspectRatio>,

    /// Seed for deterministic generation. `None` uses the provider's default.
    pub seed: Option<u64>,

    /// Images for image editing or variation generation.
    pub files: Option<Vec<ImageFile>>,

    /// Mask image for inpainting operations.
    pub mask: Option<ImageFile>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: SharedProviderOptions,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl ImageCallOptions {
    /// Create options with a prompt and `n = 1`, all other fields defaulted.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            n: 1,
            size: None,
            aspect_ratio: None,
            seed: None,
            files: None,
            mask: None,
            provider_options: SharedProviderOptions::new(),
            abort_signal: None,
            headers: None,
        }
    }
}

/// The result of [`ImageModel::do_generate`].
///
/// Aligned with V4 `ImageModelV4Result`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImageResult {
    /// Generated images (base64 strings or binary data).
    pub images: ImageOutputs,

    /// Warnings for the call, e.g. unsupported features.
    pub warnings: Vec<Warning>,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Response information for telemetry and debugging.
    pub response: ImageResponse,

    /// Optional token usage for the call (if the provider reports it).
    pub usage: Option<ImageUsage>,
}

/// Response information for an image generation call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImageResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
}

/// The unified image generation model trait (provider-facing).
///
/// Aligned with V4 `ImageModelV4`.
#[async_trait]
pub trait ImageModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"dall-e-3"`.
    fn model_id(&self) -> &str;

    /// Limit of how many images can be generated in a single API call.
    ///
    /// `None` means there is no fixed limit. The TS spec allows this to be a
    /// function of `modelId`; in Rust the provider resolves it to a plain
    /// `Option<u32>`.
    fn max_images_per_call(&self) -> Option<u32>;

    /// Generate an array of images.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError>;
}
