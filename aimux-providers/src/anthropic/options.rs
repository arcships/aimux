//! The providerOptions namespace of the Anthropic Messages protocol.
//!
//! `@ai-sdk/anthropic` always reads `providerOptions.anthropic` and, when the
//! provider was created with a custom `name`, also `providerOptions[<first
//! segment of name>]`; the custom key wins where both set the same option.
//! What the model writes back (reasoning signatures, MCP server names, ...)
//! is keyed by that custom name too, so a transcript round-trips under the
//! name the caller chose. This module is the one place that knows the
//! canonical key and how the two are merged.

use std::collections::HashMap;

use serde_json::{Map, Value};

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
fn merge(canonical: Option<&Value>, custom: Option<&Value>) -> Option<Value> {
    match (canonical, custom) {
        (None, None) => None,
        (Some(only), None) | (None, Some(only)) => Some(only.clone()),
        (Some(Value::Object(canonical)), Some(Value::Object(custom))) => {
            let mut merged: Map<String, Value> = canonical.clone();
            merged.extend(custom.iter().map(|(k, v)| (k.clone(), v.clone())));
            Some(Value::Object(merged))
        }
        (Some(_), Some(custom)) => Some(custom.clone()),
    }
}

/// The Anthropic options in a providerOptions object (a part's, a message's):
/// `anthropic` merged with `name` when `name` is a different key.
pub(crate) fn anthropic_options(provider_options: Option<&Value>, name: &str) -> Option<Value> {
    let canonical = provider_options.and_then(|options| options.get(CANONICAL));
    let custom = (name != CANONICAL)
        .then(|| provider_options.and_then(|options| options.get(name)))
        .flatten();
    merge(canonical, custom)
}

/// [`anthropic_options`] for the map form (`CallOptions::provider_options`, a
/// tool's).
pub(crate) fn anthropic_options_in(
    provider_options: Option<&HashMap<String, Value>>,
    name: &str,
) -> Option<Value> {
    let canonical = provider_options.and_then(|options| options.get(CANONICAL));
    let custom = (name != CANONICAL)
        .then(|| provider_options.and_then(|options| options.get(name)))
        .flatten();
    merge(canonical, custom)
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
        let options = json!({
            CANONICAL: { "a": 1, "b": 1 },
            "proxy": { "b": 2, "c": 2 },
        });
        assert_eq!(
            anthropic_options(Some(&options), "proxy"),
            Some(json!({ "a": 1, "b": 2, "c": 2 }))
        );
        // The canonical name reads only itself.
        assert_eq!(
            anthropic_options(Some(&options), CANONICAL),
            Some(json!({ "a": 1, "b": 1 }))
        );
    }

    #[test]
    fn either_namespace_alone_is_read() {
        let only_custom = json!({ "proxy": { "x": true } });
        assert_eq!(
            anthropic_options(Some(&only_custom), "proxy"),
            Some(json!({ "x": true }))
        );
        assert_eq!(anthropic_options(Some(&only_custom), CANONICAL), None);
        assert_eq!(anthropic_options(None, "proxy"), None);
    }

    #[test]
    fn map_form_matches_value_form() {
        let map: HashMap<String, Value> = [
            (CANONICAL.to_string(), json!({ "a": 1 })),
            ("proxy".to_string(), json!({ "a": 2 })),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            anthropic_options_in(Some(&map), "proxy"),
            Some(json!({ "a": 2 }))
        );
    }
}
