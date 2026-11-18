//! Model limits and provider options of the OpenAI image model
//! (openai-image-model-options.ts).

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use aimux_provider_utils::{optional_field, parse_provider_options};

/// Models that default to `b64_json` response format and therefore do NOT need
/// the explicit `response_format` field. Identified by prefix.
const DEFAULT_RESPONSE_FORMAT_PREFIXES: &[&str] = &["chatgpt-image-", "gpt-image-"];

/// Returns `true` if the model ID has a default response format (and thus the
/// `response_format` field should be omitted from the request).
pub(crate) fn has_default_response_format(model_id: &str) -> bool {
    DEFAULT_RESPONSE_FORMAT_PREFIXES
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
}

/// Returns the maximum number of images per call for the given model ID.
///
/// Mirrors TS `getMaxImagesPerCall`:
/// - `dall-e-3` → 1
/// - `dall-e-2`, `gpt-image-*`, `chatgpt-image-*` → 10
/// - unknown models starting with `gpt-image-` → 10
/// - everything else → 1
pub(crate) fn get_max_images_per_call(model_id: &str) -> u32 {
    match model_id {
        "dall-e-3" => 1,
        "dall-e-2"
        | "gpt-image-1"
        | "gpt-image-1-mini"
        | "gpt-image-1.5"
        | "gpt-image-2"
        | "chatgpt-image-latest" => 10,
        _ => {
            if model_id.starts_with("gpt-image-") {
                10
            } else {
                1
            }
        }
    }
}

/// `baseImageModelOptionsObject`. Every key is `.optional()`: absent is
/// `None`, `null` fails.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BaseImageOptions {
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) quality: Option<Quality>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) background: Option<Background>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) output_format: Option<OutputFormat>,
    /// `z.number().int().min(0).max(100)`.
    #[serde(default, deserialize_with = "output_compression")]
    pub(crate) output_compression: Option<u8>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) user: Option<String>,
}

/// `openaiImageModelGenerationOptions`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GenerationOptions {
    #[serde(flatten)]
    pub(crate) base: BaseImageOptions,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) style: Option<Style>,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) moderation: Option<Moderation>,
}

/// `openaiImageModelEditOptions`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditOptions {
    #[serde(flatten)]
    pub(crate) base: BaseImageOptions,
    #[serde(default, deserialize_with = "optional_field")]
    pub(crate) input_fidelity: Option<InputFidelity>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Quality {
    Standard,
    Hd,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Auto,
}

impl Quality {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Hd => "hd",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Background {
    Transparent,
    Opaque,
    Auto,
}

impl Background {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::Opaque => "opaque",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OutputFormat {
    Png,
    Jpeg,
    Webp,
}

impl OutputFormat {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Style {
    Vivid,
    Natural,
}

impl Style {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Vivid => "vivid",
            Self::Natural => "natural",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Moderation {
    Auto,
    Low,
}

impl Moderation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum InputFidelity {
    High,
    Low,
}

impl InputFidelity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Low => "low",
        }
    }
}

/// `outputCompression`: an integer from 0 to 100 (`null` fails). Read as a
/// number, so `80.0` passes as zod's `int()` lets it.
fn output_compression<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u8>, D::Error> {
    use serde::Deserialize as _;
    let value = f64::deserialize(deserializer)?;
    if (0.0..=100.0).contains(&value) && value.fract() == 0.0 {
        // An integer from 0 to 100: the cast is exact.
        Ok(Some(value as u8))
    } else {
        Err(serde::de::Error::custom(
            "expected an integer from 0 to 100",
        ))
    }
}

/// `parseProviderOptions` with `openaiImageModelGenerationOptions`
/// (openai-image-model.ts:226-231); every key `None` without an `openai`
/// namespace.
///
/// # Errors
///
/// `InvalidArgument` "invalid openai provider options" when the namespace
/// does not match the schema, with the serde error as its cause.
pub(crate) fn parse_generation_provider_options(
    provider_options: &SharedProviderOptions,
) -> Result<GenerationOptions, AiMuxError> {
    Ok(parse_provider_options("openai", Some(provider_options))?.unwrap_or_default())
}

/// `parseProviderOptions` with `openaiImageModelEditOptions`
/// (openai-image-model.ts:135-141); every key `None` without an `openai`
/// namespace.
///
/// # Errors
///
/// `InvalidArgument` "invalid openai provider options" when the namespace
/// does not match the schema, with the serde error as its cause.
pub(crate) fn parse_edit_provider_options(
    provider_options: &SharedProviderOptions,
) -> Result<EditOptions, AiMuxError> {
    Ok(parse_provider_options("openai", Some(provider_options))?.unwrap_or_default())
}
