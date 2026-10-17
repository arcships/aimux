//! The providerOptions namespace of Fal.
//!
//! Fal reads `providerOptions.fal`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "fal";

/// The Fal options in a providerOptions map.
pub(crate) fn fal_options(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
