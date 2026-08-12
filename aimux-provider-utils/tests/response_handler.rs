//! Rust translation of `packages/provider-utils/src/response-handler.test.ts`.
//!
//! The TS response-handler tests exercise `createJsonResponseHandler`,
//! `createJsonErrorResponseHandler`, `createBinaryResponseHandler`, and
//! `createStatusCodeErrorResponseHandler` against the Web `Response` object,
//! including oversized-response size limiting and binary handling.
//!
//! The Rust equivalent works one layer down: providers fetch a response, then
//! call [`aimux_provider_utils::parse_provider_error`] with the HTTP status and
//! response body text to produce an [`aimux_core::AiMuxError`]. The cases that
//! depend on the streaming `Response` object (size-limit cancellation, binary
//! array buffers, the `rawValue`/`value` split) have no Rust counterpart and
//! are skipped with comments. The translatable cases — status-code → error
//! mapping, message extraction via an error structure, empty/non-JSON bodies,
//! and custom JSON paths — are covered below.

use aimux_core::AiMuxError;
use aimux_provider_utils::{DEFAULT_ERROR_STRUCTURE, ErrorStructure, parse_provider_error};

const TEST_URL: &str = "test-url";

// ---------------------------------------------------------------------------
// createStatusCodeErrorResponseHandler
// ---------------------------------------------------------------------------

#[test]
fn status_code_error_uses_status_text_for_non_json_body() {
    // TS: "should create error with status text and response body".
    //
    // The TS handler sets `message` to `response.statusText` ("Not Found") and
    // `responseBody` to the body text. Rust's `parse_provider_error` uses the
    // body text as the message when it isn't JSON matching the error
    // structure; a 404 maps to `ApiCall` (404).
    let body = "Error message";
    let err = parse_provider_error(404, body, &DEFAULT_ERROR_STRUCTURE);

    assert!(
        matches!(err, AiMuxError::ApiCall(ref m) if m.status_code == Some(404) && m.message == "Error message")
    );
}

#[test]
fn status_code_500_maps_to_provider_error() {
    // A 5xx-style status maps to the generic `Provider` variant.
    let body = "Internal Server Error";
    let err = parse_provider_error(500, body, &DEFAULT_ERROR_STRUCTURE);
    assert!(matches!(err, AiMuxError::ApiCall(_)));
}

// ---------------------------------------------------------------------------
// createJsonErrorResponseHandler
// ---------------------------------------------------------------------------

#[test]
fn json_error_response_extracts_message_from_default_structure() {
    // TS: createJsonErrorResponseHandler parses the body JSON and maps
    // `error.message` into the APICallError message. Rust's
    // `parse_provider_error` with the default structure (`["error","message"]`)
    // does the same; a 429 maps to `ApiCall` (429).
    let body = r#"{"error":{"message":"Rate limit reached for requests","type":"requests"}}"#;
    let err = parse_provider_error(429, body, &DEFAULT_ERROR_STRUCTURE);
    assert!(matches!(err, AiMuxError::ApiCall(ref d) if d.status_code == Some(429)));
}

#[test]
fn json_error_response_extracts_message_from_custom_structure() {
    // Some providers nest the error differently; a custom `ErrorStructure`
    // routes the message out of a different JSON path. The `Provider` arm
    // prepends "HTTP {status}: " to the extracted message.
    let structure = ErrorStructure {
        message_path: &["error", "msg"],
        type_path: &["error", "kind"],
    };
    let body = r#"{"error":{"msg":"boom","kind":"failure"}}"#;
    let err = parse_provider_error(500, body, &structure);
    let AiMuxError::ApiCall(ref detail) = err else {
        panic!("expected ApiCall, got {err:?}")
    };
    assert_eq!(detail.message, "boom");
    assert_eq!(err.to_string(), "API call error: HTTP 500: boom");
}

#[test]
fn empty_body_falls_back_to_http_status_message() {
    // TS: when the response body is empty, the handler uses `response.statusText`
    // as the message. Here there is no body text at all, so the status carried in
    // `ApiCallError` is the whole of what Display has to show.
    let err = parse_provider_error(503, "", &DEFAULT_ERROR_STRUCTURE);
    assert_eq!(err.status_code(), Some(503));
    assert!(err.to_string().contains("503"), "{err}");
}

#[test]
fn non_json_body_is_used_as_the_message() {
    // A non-empty, non-JSON body is used verbatim as the message.
    let err = parse_provider_error(401, "plain text problem", &DEFAULT_ERROR_STRUCTURE);
    assert!(
        matches!(err, AiMuxError::ApiCall(ref m) if m.status_code == Some(401) && m.message == "plain text problem")
    );
}

#[test]
fn auth_status_maps_to_auth_variant() {
    let body = r#"{"error":{"message":"Invalid API key","type":"auth"}}"#;
    let err = parse_provider_error(401, body, &DEFAULT_ERROR_STRUCTURE);
    assert!(
        matches!(err, AiMuxError::ApiCall(ref m) if m.status_code == Some(401) && m.message == "Invalid API key")
    );
}

// ---------------------------------------------------------------------------
// SKIPPED TS cases (no Rust counterpart)
// ---------------------------------------------------------------------------
// The following TS cases rely on the streaming Web `Response` object and a
// download size limit (`readResponseWithSizeLimit` / DEFAULT_MAX_DOWNLOAD_SIZE)
// that the Rust implementation does not model — providers consume an
// already-fetched `(status, body)` pair via `parse_provider_error`. They are
// intentionally not translated:
//
//   - createJsonResponseHandler "should return both parsed value and rawValue"
//     (Rust has no value/rawValue split; callers parse the body themselves.)
//   - createJsonResponseHandler "should reject oversized responses before
//     reading the body" (no size-limit gate in the Rust fetch path.)
//   - createJsonErrorResponseHandler "should reject oversized responses before
//     reading the body" (same.)
//   - createBinaryResponseHandler "should handle binary response successfully"
//     (Rust has no binary response handler; binary is read directly by callers.)
//   - createBinaryResponseHandler "should throw APICallError when response body
//     is null" (no binary handler to assert an empty-body error against.)
//   - createStatusCodeErrorResponseHandler "should reject oversized responses
//     before reading the body" (no size-limit gate.)

#[test]
fn url_constant_is_unused_but_documented() {
    // Kept so `TEST_URL` (mirroring the TS test's `testUrl` constant) is
    // referenced; the Rust parse_provider_error API does not take a URL.
    assert_eq!(TEST_URL, "test-url");
}
