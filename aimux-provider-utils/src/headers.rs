//! HTTP header utilities.
//!
//! Provider headers are layered — provider settings, then per-call headers,
//! then authentication — and later layers win. A layer entry of `None` means
//! "remove this header" (the AI SDK's `undefined` header value).

use std::collections::HashMap;

use crate::resolvable::Resolvable;

/// The SDK version (injected at build time or hardcoded for now).
const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A header layer: `None` removes the header from earlier layers.
pub type HeaderMapOpt = HashMap<String, Option<String>>;

/// Headers supplied directly or computed per request (token refresh, …).
pub type HeadersFn = Resolvable<HeaderMapOpt>;

/// Add a `User-Agent` suffix to the headers.
///
/// Pattern: `ai-sdk/<provider-name>/<version>`
pub fn with_user_agent_suffix(headers: &mut HashMap<String, String>, provider_name: &str) {
    let ua = format!("ai-sdk/{provider_name}/{SDK_VERSION}");
    headers.insert("User-Agent".to_string(), ua);
}

/// Merge header layers, earliest first. Names are lowercased; a later layer
/// overrides an earlier one (never appends), including with `None`, which
/// removes the header. Entries of one layer whose names differ only by case
/// are applied in byte order of the original name, so the result does not
/// depend on hash-map iteration order.
#[must_use]
pub fn combine_headers(layers: &[&HeaderMapOpt]) -> HeaderMapOpt {
    let mut combined = HeaderMapOpt::new();
    for layer in layers {
        let mut entries: Vec<_> = layer.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, value) in entries {
            combined.insert(name.to_ascii_lowercase(), value.clone());
        }
    }
    combined
}

/// Turn a header layer into the list that goes on the wire: names lowercased,
/// `None` entries dropped, one entry per name (when names differ only by case
/// the one that sorts last wins), sorted by name.
#[must_use]
pub fn normalize_headers(headers: HeaderMapOpt) -> Vec<(String, String)> {
    let mut entries: Vec<_> = headers.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut unique: std::collections::BTreeMap<String, Option<String>> =
        std::collections::BTreeMap::new();
    for (name, value) in entries {
        unique.insert(name.to_ascii_lowercase(), value);
    }
    unique
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(entries: &[(&str, Option<&str>)]) -> HeaderMapOpt {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.map(str::to_string)))
            .collect()
    }

    #[test]
    fn names_are_lowercased_and_later_layers_override_regardless_of_case() {
        let provider = layer(&[("X-Org", Some("provider")), ("Content-Type", Some("a"))]);
        let call = layer(&[("x-org", Some("call"))]);
        let combined = combine_headers(&[&provider, &call]);
        assert_eq!(combined.len(), 2);
        assert_eq!(combined["x-org"].as_deref(), Some("call"));
        assert_eq!(combined["content-type"].as_deref(), Some("a"));
    }

    #[test]
    fn none_removes_a_header_set_by_an_earlier_layer() {
        let provider = layer(&[("x-remove", Some("1")), ("x-keep", Some("1"))]);
        let call = layer(&[("X-Remove", None)]);
        let wire = normalize_headers(combine_headers(&[&provider, &call]));
        assert_eq!(wire, vec![("x-keep".to_string(), "1".to_string())]);
    }

    #[test]
    fn a_later_value_restores_a_removed_header() {
        let removed = layer(&[("x-a", None)]);
        let restored = layer(&[("x-a", Some("back"))]);
        let wire = normalize_headers(combine_headers(&[
            &layer(&[("x-a", Some("1"))]),
            &removed,
            &restored,
        ]));
        assert_eq!(wire, vec![("x-a".to_string(), "back".to_string())]);
    }

    #[test]
    fn provider_then_call_then_auth_order_with_a_single_authorization() {
        let provider = layer(&[
            ("Authorization", Some("Bearer provider")),
            ("x-p", Some("1")),
        ]);
        let call = layer(&[("authorization", Some("Bearer call")), ("x-c", Some("1"))]);
        let auth = layer(&[("AUTHORIZATION", Some("Bearer auth"))]);
        let wire = normalize_headers(combine_headers(&[&provider, &call, &auth]));
        let authorizations: Vec<_> = wire
            .iter()
            .filter(|(name, _)| name == "authorization")
            .collect();
        assert_eq!(
            authorizations.len(),
            1,
            "same name must be sent once: {wire:?}"
        );
        assert_eq!(authorizations[0].1, "Bearer auth");
        assert!(wire.contains(&("x-p".to_string(), "1".to_string())));
        assert!(wire.contains(&("x-c".to_string(), "1".to_string())));
    }

    #[test]
    fn normalize_collapses_names_that_differ_only_by_case_deterministically() {
        let wire = normalize_headers(layer(&[("X-A", Some("upper")), ("x-a", Some("lower"))]));
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].0, "x-a");
        // "x-a" sorts after "X-A" in byte order, so it is the one kept.
        assert_eq!(wire[0].1, "lower");
    }
}
