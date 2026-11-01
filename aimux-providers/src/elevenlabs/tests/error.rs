//! Port of `@ai-sdk/elevenlabs@3.0.54` `src/elevenlabs-error.test.ts`.
//!
//! Upstream parses the error body against `elevenlabsErrorDataSchema`
//! (`{ error: { message, code } }`); aimux reads it in
//! `elevenlabs_error_parts`, which keeps `error.message` as the message.

use serde_json::Value;

use crate::elevenlabs::elevenlabs_error_parts;

mod elevenlabs_error_data_schema {
    use super::*;

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-error.test.ts › elevenlabsErrorDataSchema › should parse ElevenLabs resource exhausted error
    #[test]
    fn should_parse_elevenlabs_resource_exhausted_error() {
        let error: Value = serde_json::from_str(
            r#"
{"error":{"message":"{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n","code":429}}
"#,
        )
        .unwrap();

        let parts = elevenlabs_error_parts(&error);

        assert_eq!(
            parts.message,
            "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n"
        );
        assert_eq!(parts.provider_code, None);
    }
}
