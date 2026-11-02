//! The providerOptions namespace of Luma.
//!
//! Luma reads `providerOptions.luma`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "luma";

/// The Luma options in a providerOptions map.
pub(crate) fn luma_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `lumaImageModelOptionsSchema` (luma-image-model-options.ts), a
/// `z.looseObject` whose keys are all `.nullish()`. Only checked: the request
/// is built from the namespace as given, which also forwards the keys the
/// schema does not declare.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct LumaImageOptions {
    #[serde(default)]
    reference_type: Option<ReferenceType>,
    #[serde(default)]
    images: Option<Vec<ImageConfig>>,
    #[serde(default)]
    poll_interval_millis: Option<serde_json::Number>,
    #[serde(default)]
    max_poll_attempts: Option<serde_json::Number>,
}

/// `z.enum(['image', 'style', 'character', 'modify_image'])`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReferenceType {
    Image,
    Style,
    Character,
    ModifyImage,
}

/// One `images` entry: `{ weight?: number 0..1 | null, id?: string | null }`.
#[derive(Debug, serde::Deserialize)]
#[allow(
    dead_code,
    reason = "validated only; the request is built from the raw namespace"
)]
struct ImageConfig {
    #[serde(default, deserialize_with = "weight")]
    weight: Option<f64>,
    #[serde(default)]
    id: Option<String>,
}

/// `z.number().min(0).max(1).nullish()`.
fn weight<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    use serde::Deserialize as _;
    match Option::<f64>::deserialize(deserializer)? {
        Some(weight) if !(0.0..=1.0).contains(&weight) => {
            Err(serde::de::Error::custom("expected a number from 0 to 1"))
        }
        weight => Ok(weight),
    }
}

/// `parseProviderOptions({ provider: 'luma', providerOptions, schema:
/// lumaImageModelOptionsSchema })` (luma-image-model.ts doGenerate).
///
/// # Errors
///
/// `InvalidArgument` "invalid luma provider options" when the namespace does
/// not match the schema, with the serde error as its cause.
pub(crate) fn validate(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<(), aimux_core::AiMuxError> {
    aimux_provider_utils::parse_provider_options::<LumaImageOptions>(NAMESPACE, provider_options)?;
    Ok(())
}
