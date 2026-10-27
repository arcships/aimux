use aimux_core::error::{AiMuxError, ApiCallError};
use serde_json::{Value, json};

use crate::openai::openai_stream_error;

/// `createOpenAIProviderStreamError(frame)`. The Rust error is an
/// `ApiCallError`; it has no `type` field, so upstream's `type` is not
/// asserted.
fn create_stream_error(frame: &Value) -> ApiCallError {
    match openai_stream_error(
        frame,
        "https://api.test.com/v1/responses",
        json!({}),
        std::collections::HashMap::new(),
    ) {
        AiMuxError::ApiCall(details) => *details,
        other => panic!("expected an API call error, got {other:?}"),
    }
}

mod create_open_ai_provider_stream_error {
    use super::*;

    /// upstream: @ai-sdk/openai src/openai-stream-error.test.ts › createOpenAIProviderStreamError › classifies a documented top-level rate-limit event
    #[test]
    fn classifies_a_documented_top_level_rate_limit_event() {
        let data = json!({
            "type": "error",
            "code": "rate_limit_exceeded",
            "message": "Rate limit reached",
            "param": null,
        });

        let error = create_stream_error(&data);
        assert_eq!(error.message, "Rate limit reached");
        assert_eq!(error.provider_code.as_deref(), Some("rate_limit_exceeded"));
        assert_eq!(error.status_code, Some(429));
        assert!(error.is_retryable);
        assert_eq!(error.data, Some(data));
    }

    /// upstream: @ai-sdk/openai src/openai-stream-error.test.ts › createOpenAIProviderStreamError › classifies insufficient quota as non-retryable
    #[test]
    fn classifies_insufficient_quota_as_non_retryable() {
        let data = json!({
            "type": "error",
            "code": "insufficient_quota",
            "message": "You exceeded your current quota.",
            "param": null,
        });

        let error = create_stream_error(&data);
        assert_eq!(error.message, "You exceeded your current quota.");
        assert_eq!(error.provider_code.as_deref(), Some("insufficient_quota"));
        assert_eq!(error.status_code, Some(429));
        assert!(!error.is_retryable);
        assert_eq!(error.data, Some(data));
    }

    /// upstream: @ai-sdk/openai src/openai-stream-error.test.ts › createOpenAIProviderStreamError › preserves the provider type when code is an HTTP status
    #[test]
    fn preserves_the_provider_type_when_code_is_an_http_status() {
        let data = json!({
            "type": "rate_limit_error",
            "code": "429",
            "message": "Rate limit reached",
        });

        let error = create_stream_error(&data);
        assert_eq!(error.message, "Rate limit reached");
        assert_eq!(error.provider_code.as_deref(), Some("429"));
        assert_eq!(error.status_code, Some(429));
        assert!(error.is_retryable);
        assert_eq!(error.data, Some(data));
    }

    /// upstream: @ai-sdk/openai src/openai-stream-error.test.ts › createOpenAIProviderStreamError › classifies a response.failed server error by its provider code
    #[test]
    fn classifies_a_response_failed_server_error_by_its_provider_code() {
        let data = json!({
            "type": "response.failed",
            "response": {
                "error": {"code": "server_error", "message": "Response failed"},
            },
        });

        let error = create_stream_error(&data);
        assert_eq!(error.message, "Response failed");
        assert_eq!(error.provider_code.as_deref(), Some("server_error"));
        assert_eq!(error.status_code, Some(500));
        assert!(error.is_retryable);
        assert_eq!(error.data, Some(data));
    }
}
