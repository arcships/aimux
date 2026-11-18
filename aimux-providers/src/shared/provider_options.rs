//! The providerOptions namespace of a vendor package.
//!
//! A vendor reads its own key (its `NAMESPACE` constant), whatever the
//! provider is named. Packages that parse their options with a schema use
//! `aimux_provider_utils::parse_provider_options` instead.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The options under `namespace` in a providerOptions map.
pub(crate) fn namespaced_options<'a>(
    provider_options: Option<&'a SharedProviderOptions>,
    namespace: &str,
) -> Option<&'a JsonObject> {
    provider_options?.get(namespace)
}
