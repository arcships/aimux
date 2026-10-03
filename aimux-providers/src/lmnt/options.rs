//! The providerOptions namespace of LMNT.
//!
//! LMNT reads `providerOptions.lmnt`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "lmnt";

/// The LMNT options in a providerOptions map.
pub(crate) fn lmnt_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
