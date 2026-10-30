//! Port of `packages/provider-utils/src/is-url-supported.ts`
//! (`@ai-sdk/provider-utils@5.0.55`).

use std::collections::HashMap;

use regex::Regex;

/// Whether `url` is supported natively by the model for a file of
/// `media_type`, the AI SDK's `isUrlSupported`.
///
/// `media_type` is case-insensitive and may be a full `type/subtype`, a
/// wildcard `type/*`, or only the top-level segment (`image`). The keys of
/// `supported_urls` are media types or `*`; the values are URL patterns.
#[must_use]
pub fn is_url_supported(
    media_type: &str,
    url: &str,
    supported_urls: &HashMap<String, Vec<Regex>>,
) -> bool {
    let url = url.to_lowercase();
    let media_type = media_type.to_lowercase();

    supported_urls
        .iter()
        .map(|(key, regexes)| {
            let media_type_prefix = if key == "*" || key == "*/*" {
                String::new()
            } else {
                key.replacen('*', "", 1)
            };
            (media_type_prefix, regexes)
        })
        .filter(|(media_type_prefix, _)| {
            if media_type_prefix.is_empty() {
                return true;
            }
            if media_type_prefix.ends_with('/') {
                media_type.starts_with(media_type_prefix.as_str())
            } else {
                media_type == *media_type_prefix
            }
        })
        .flat_map(|(_, regexes)| regexes)
        .any(|pattern| pattern.is_match(&url))
}
