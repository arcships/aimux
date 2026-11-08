//! Cases ported from `is-url-supported.test.ts` (`@ai-sdk/provider-utils@5.0.55`).

use std::collections::HashMap;

use is_url_supported::is_url_supported;
use regex::Regex;

fn urls(entries: &[(&str, &[&str])]) -> HashMap<String, Vec<Regex>> {
    entries
        .iter()
        .map(|(key, patterns)| {
            let regexes = patterns.iter().map(|p| Regex::new(p).unwrap()).collect();
            (key.to_string(), regexes)
        })
        .collect()
}

#[test]
fn no_supported_urls_returns_false() {
    assert!(!is_url_supported(
        "text/plain",
        "https://example.com",
        &HashMap::new()
    ));
}

#[test]
fn should_not_match_image_png_against_suffixed_media_types() {
    for media_type in [
        "image/png-not-supported",
        "image/png+invalid",
        "image/png;invalid",
    ] {
        let supported = urls(&[("image/png", &["^gs://"])]);
        assert!(
            !is_url_supported(media_type, "gs://example-bucket/image", &supported),
            "{media_type}"
        );
    }
}

#[test]
fn exact_media_type_and_exact_url_match() {
    let supported = urls(&[("text/plain", &[r"https://example\.com"])]);
    assert!(is_url_supported(
        "text/plain",
        "https://example.com",
        &supported
    ));
}

#[test]
fn exact_media_type_but_url_mismatch() {
    let supported = urls(&[("text/plain", &[r"https://example\.com"])]);
    assert!(!is_url_supported(
        "text/plain",
        "https://another.com",
        &supported
    ));
}

#[test]
fn url_match_but_media_type_mismatch() {
    let supported = urls(&[("text/plain", &[r"https://example\.com"])]);
    assert!(!is_url_supported(
        "image/png",
        "https://example.com",
        &supported
    ));
}

#[test]
fn wildcard_media_type_and_regex_url_match() {
    let supported = urls(&[("*", &[r"https://images\.example\.com/.+"])]);
    assert!(is_url_supported(
        "image/jpeg",
        "https://images.example.com/dog.jpg",
        &supported
    ));
}

#[test]
fn case_insensitive_for_media_types() {
    let supported = urls(&[("text/plain", &[r"https://example\.com"])]);
    assert!(is_url_supported(
        "TEXT/PLAIN",
        "https://example.com",
        &supported
    ));
}

#[test]
fn case_insensitive_for_url_paths_by_default_regex() {
    let supported = urls(&[("text/plain", &[r"https://example\.com/path"])]);
    assert!(is_url_supported(
        "text/plain",
        "https://example.com/PATH",
        &supported
    ));
}

#[test]
fn wildcard_subtype_match() {
    let supported = urls(&[("image/*", &[r"https://example\.com"])]);
    assert!(is_url_supported(
        "image/png",
        "https://example.com",
        &supported
    ));
}

#[test]
fn falls_back_to_full_wildcard() {
    let supported = urls(&[
        ("image/*", &[r"https://images\.com"]),
        ("*", &[r"https://any\.com"]),
    ]);
    assert!(is_url_supported("image/png", "https://any.com", &supported));
}

#[test]
fn empty_url_array_for_specific_media_type() {
    let supported = urls(&[("text/plain", &[])]);
    assert!(!is_url_supported(
        "text/plain",
        "https://example.com",
        &supported
    ));
}
