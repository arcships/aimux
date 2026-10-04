//! The providerOptions namespace of KlingAI.
//!
//! KlingAI reads `providerOptions.klingai`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "klingai";

/// The KlingAI options in a providerOptions map.
pub(crate) fn klingai_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
