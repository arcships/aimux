//! The providerOptions namespace of the MiniMax media models and its schemas.
//!
//! The models read `providerOptions.minimax`, whatever the provider is named.
//! This module is the one place that spells the key. Unknown keys are
//! rejected: a misspelled option would otherwise be dropped silently from a
//! billable request.

use serde::Deserialize;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "minimax";

/// `providerOptions.minimax` of a video call.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct VideoOptions {
    /// `480P`, `768P` or `2K`, as the model allows.
    #[serde(default)]
    pub resolution: Option<String>,
    /// `adaptive` or one of the fixed ratios; wins over `aspectRatio` of the
    /// call, which cannot say `adaptive`.
    #[serde(default)]
    pub ratio: Option<String>,
    /// The last frame as an `http(s)` URL or a `data:` URI, for callers that
    /// do not pass it in `frameImages`.
    #[serde(default)]
    pub last_frame: Option<String>,
    /// `aigc_watermark`.
    #[serde(default)]
    pub aigc_watermark: Option<bool>,
    /// `extra.prompt_expansion_mode` (`MiniMax-H3-Max` only): `disabled`,
    /// `balanced` or `quality`.
    #[serde(default)]
    pub prompt_expansion_mode: Option<String>,
}

/// `providerOptions.minimax` of an image call.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ImageOptions {
    /// `auto` or `X:Y`; wins over `aspectRatio` of the call.
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    /// `1K`, `2K` or `4K`.
    #[serde(default)]
    pub area: Option<String>,
    /// `disable_prompt_enhanced`.
    #[serde(default)]
    pub disable_prompt_enhanced: Option<bool>,
    /// `output_compression`, 0 to 100.
    #[serde(default)]
    pub output_compression: Option<i64>,
    /// `aigc_watermark`.
    #[serde(default)]
    pub aigc_watermark: Option<bool>,
}

/// Parse the MiniMax namespace of `provider_options` as `T`; an absent
/// namespace is `T::default()`.
pub(super) fn parse<T: Default + for<'de> Deserialize<'de>>(
    provider_options: &SharedProviderOptions,
) -> Result<T, AiMuxError> {
    Ok(
        aimux_provider_utils::parse_provider_options::<T>(NAMESPACE, Some(provider_options))?
            .unwrap_or_default(),
    )
}
