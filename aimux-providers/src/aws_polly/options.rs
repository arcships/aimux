//! The providerOptions namespace of Amazon Polly.
//!
//! Amazon Polly reads `providerOptions.aws_polly`, whatever the provider is named. This
//! module is the one place that spells the key.

use serde_json::Value;

/// The providerOptions key.
pub(crate) const NAMESPACE: &str = "aws_polly";

/// The Amazon Polly options in a providerOptions map.
pub(crate) fn aws_polly_options<M: crate::shared::ProviderOptionsMap + ?Sized>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(NAMESPACE)
}
