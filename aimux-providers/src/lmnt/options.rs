//! The providerOptions namespace of LMNT.
//!
//! LMNT reads `providerOptions.lmnt`, whatever the provider is named. This
//! module is the one place that spells the key.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "lmnt";

/// The LMNT options in a providerOptions map.
pub(crate) fn lmnt_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}
