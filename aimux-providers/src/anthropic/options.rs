//! The providerOptions namespace of the Anthropic Messages protocol.
//!
//! `@ai-sdk/anthropic` always reads `providerOptions.anthropic` and, when the
//! provider was created with a custom `name`, also `providerOptions[<first
//! segment of name>]`; the custom key wins where both set the same option.
//! What the model writes back (reasoning signatures, MCP server names, ...)
//! is keyed by that custom name too, so a transcript round-trips under the
//! name the caller chose. This module is the one place that knows the
//! canonical key and how the two are merged.

use aimux_core::shared::{JsonObject, SharedProviderOptions};

/// The canonical providerOptions key, read for every model of the protocol
/// whatever its name.
pub(crate) const CANONICAL: &str = "anthropic";

/// The providerOptions key of a provider name: its first dot-separated
/// segment, trimmed (`"proxy.messages"` -> `"proxy"`, `"proxy"` -> `"proxy"`).
pub(crate) fn options_name_of(provider: &str) -> String {
    provider
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Merge the canonical options with the custom-name options (custom wins,
/// shallowly). `None` when neither is set.
fn merge(canonical: Option<&JsonObject>, custom: Option<&JsonObject>) -> Option<JsonObject> {
    match (canonical, custom) {
        (None, None) => None,
        (Some(only), None) | (None, Some(only)) => Some(only.clone()),
        (Some(canonical), Some(custom)) => {
            let mut merged = canonical.clone();
            merged.extend(custom.iter().map(|(k, v)| (k.clone(), v.clone())));
            Some(merged)
        }
    }
}

/// The Anthropic options in a providerOptions object (a part's, a message's):
/// `anthropic` merged with `name` when `name` is a different key.
pub(crate) fn anthropic_options(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Option<JsonObject> {
    let canonical = provider_options.and_then(|options| options.get(CANONICAL));
    let custom = (name != CANONICAL)
        .then(|| provider_options.and_then(|options| options.get(name)))
        .flatten();
    merge(canonical, custom)
}

/// [`anthropic_options`] for `CallOptions::provider_options` or a tool's options.
pub(crate) fn anthropic_options_in(
    provider_options: Option<&SharedProviderOptions>,
    name: &str,
) -> Option<JsonObject> {
    anthropic_options(provider_options, name)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn name_is_the_first_segment() {
        assert_eq!(options_name_of("anthropic.messages"), CANONICAL);
        assert_eq!(options_name_of("proxy"), "proxy");
        assert_eq!(options_name_of(" proxy .messages"), "proxy");
    }

    #[test]
    fn custom_options_override_the_canonical_ones() {
        let options = serde_json::from_value(json!({
            CANONICAL: { "a": 1, "b": 1 },
            "proxy": { "b": 2, "c": 2 },
        }))
        .unwrap();
        assert_eq!(
            anthropic_options(Some(&options), "proxy"),
            Some(serde_json::from_value(json!({ "a": 1, "b": 2, "c": 2 })).unwrap())
        );
        // The canonical name reads only itself.
        assert_eq!(
            anthropic_options(Some(&options), CANONICAL),
            Some(serde_json::from_value(json!({ "a": 1, "b": 1 })).unwrap())
        );
    }

    #[test]
    fn either_namespace_alone_is_read() {
        let only_custom = aimux_core::shared::provider_namespace("proxy", json!({ "x": true }))
            .expect("provider metadata must be an object");
        assert_eq!(
            anthropic_options(Some(&only_custom), "proxy"),
            Some(serde_json::from_value(json!({ "x": true })).unwrap())
        );
        assert_eq!(anthropic_options(Some(&only_custom), CANONICAL), None);
        assert_eq!(anthropic_options(None, "proxy"), None);
    }
}
