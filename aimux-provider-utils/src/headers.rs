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
