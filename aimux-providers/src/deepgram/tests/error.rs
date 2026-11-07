//! Port of `@ai-sdk/deepgram@3.1.24` `src/deepgram-error.test.ts`.
//!
//! Upstream parses the error body against `deepgramErrorDataSchema`; aimux
//! reads it in `deepgram_error_parts`, which keeps `err_msg` as the message
//! and `err_code` as the provider code.

use serde_json::Value;

use crate::deepgram::deepgram_error_parts;

mod deepgram_error_data_schema {
    use super::*;

    /// upstream: @ai-sdk/deepgram src/deepgram-error.test.ts › deepgramErrorDataSchema › should parse the err_code/err_msg error shape
    #[test]
    fn should_parse_the_err_code_err_msg_error_shape() {
        let error: Value = serde_json::from_str(
            r#"{"err_code":"INVALID_QUERY_PARAMETER","err_msg":"Invalid 'model' value of 'aura-2-not-a-real-voice-en'.","request_id":"01a00450-5a52-70f0-9253-2fc492123595"}"#,
        )
        .unwrap();

        let parts = deepgram_error_parts(&error);

        assert_eq!(
            parts.message,
            "Invalid 'model' value of 'aura-2-not-a-real-voice-en'."
        );
        assert_eq!(
            parts.provider_code.as_deref(),
            Some("INVALID_QUERY_PARAMETER")
        );
    }

    /// upstream: @ai-sdk/deepgram src/deepgram-error.test.ts › deepgramErrorDataSchema › should parse an auth error without relying on optional fields
    #[test]
    fn should_parse_an_auth_error_without_relying_on_optional_fields() {
        let error: Value =
            serde_json::from_str(r#"{"err_code":"INVALID_AUTH","err_msg":"Invalid credentials."}"#)
                .unwrap();

        let parts = deepgram_error_parts(&error);

        assert_eq!(parts.message, "Invalid credentials.");
        assert_eq!(parts.provider_code.as_deref(), Some("INVALID_AUTH"));
    }
}
