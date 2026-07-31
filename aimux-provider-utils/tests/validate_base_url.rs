//! Tests for [`aimux_provider_utils::validate_base_url`].
//!
//! The validator must reject non-HTTP(S) schemes, URLs without a host, and
//! unparseable input, while accepting valid `http`/`https` base URLs.

use aimux_core::AiMuxError;
use aimux_provider_utils::validate_base_url;

#[test]
fn rejects_file_scheme() {
    let err = validate_base_url("file:///etc/passwd").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn rejects_ftp_scheme() {
    let err = validate_base_url("ftp://example.com").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn rejects_unparseable_input() {
    let err = validate_base_url("not a url").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn rejects_empty_string() {
    let err = validate_base_url("").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
}

#[test]
fn accepts_https_url() {
    assert_eq!(
        validate_base_url("https://api.openai.com/v1").unwrap(),
        "https://api.openai.com/v1"
    );
}

#[test]
fn accepts_http_localhost() {
    assert_eq!(
        validate_base_url("http://localhost:8080").unwrap(),
        "http://localhost:8080"
    );
}

#[test]
fn strips_trailing_slash_on_valid_url() {
    assert_eq!(
        validate_base_url("https://api.openai.com/v1/").unwrap(),
        "https://api.openai.com/v1"
    );
}

#[test]
fn rejects_url_without_host() {
    // "https://" has no host and fails to parse into a usable URL.
    let err = validate_base_url("https:///").unwrap_err();
    assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err:?}");
}
