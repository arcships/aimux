//! The image generation provider tool and its arguments (tool/image-generation.ts).

use aimux_core::AiMuxError;
use aimux_core::tool::ProviderTool;
use serde::{Deserialize, Serialize};

use crate::openai::tools::provider_tool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageAction {
    Generate,
    Edit,
    Auto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackground {
    Auto,
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFidelity {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageModeration {
    Auto,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageQuality {
    Auto,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputImageMask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<ImageAction>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ImageBackground>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_fidelity: Option<InputFidelity>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_image_mask: Option<InputImageMask>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<ImageModeration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u8>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<ImageOutputFormat>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_images: Option<u8>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ImageQuality>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

/// Configure the `image_generation` provider tool.
///
/// # Errors
/// Returns an error when arguments do not match the provider tool schema.
pub fn image_generation(args: ImageGenerationArgs) -> Result<ProviderTool, AiMuxError> {
    provider_tool("image_generation", args)
}
