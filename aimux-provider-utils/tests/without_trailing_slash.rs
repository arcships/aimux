//! Rust translation of
//! `packages/provider-utils/src/without-trailing-slash.test.ts`.
//!
//! The TS `withoutTrailingSlash(url: string | undefined)` returns `undefined`
//! for `undefined` input. The Rust [`aimux_provider_utils::without_trailing_slash`]
//! operates on `&str`; an optional URL is mapped over it, which covers the
//! `None` case.

use aimux_provider_utils::without_trailing_slash;

#[test]
fn removes_a_trailing_slash() {
    // TS: "removes a trailing slash"
    assert_eq!(
        without_trailing_slash("https://example.com/"),
        "https://example.com"
    );
}

#[test]
fn returns_none_when_url_is_none() {
    // TS: "returns undefined when the URL is undefined"
    let url: Option<&str> = None;
    assert_eq!(url.map(without_trailing_slash), None);
}

#[test]
fn preserves_an_empty_string() {
    // TS: "preserves an empty string"
    assert_eq!(without_trailing_slash(""), "");
    let url: Option<&str> = Some("");
    assert_eq!(url.map(without_trailing_slash), Some(String::new()));
}

#[test]
fn leaves_url_without_trailing_slash_unchanged() {
    // Extra coverage: no trailing slash -> unchanged.
    assert_eq!(
        without_trailing_slash("https://example.com/v1"),
        "https://example.com/v1"
    );
}

#[test]
fn removes_only_the_final_slash() {
    // Extra coverage: only the trailing slash is removed; interior slashes stay.
    assert_eq!(
        without_trailing_slash("https://example.com/v1/"),
        "https://example.com/v1"
    );
}
