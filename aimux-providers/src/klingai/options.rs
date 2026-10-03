//! The providerOptions namespace of KlingAI.
//!
//! KlingAI reads `providerOptions.klingai`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "klingai";

/// The KlingAI options in a providerOptions map.
pub(crate) fn klingai_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
