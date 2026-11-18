//! Port of `@ai-sdk/fal@3.0.54` `src/fal-error.test.ts`.
//!
//! Upstream parses the error body against `falErrorDataSchema`
//! (`{ error: { message, code } }`); aimux reads it in `fal_error_parts`,
//! which keeps `error.message` as the message.

use serde_json::Value;

use crate::fal::fal_error_parts;

mod fal_error_data_schema {
    use super::*;

    /// upstream: @ai-sdk/fal src/fal-error.test.ts › falErrorDataSchema › should parse Fal resource exhausted error
    #[test]
    fn should_parse_fal_resource_exhausted_error() {
        let error: Value = serde_json::from_str(
            r#"
{"error":{"message":"{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n","code":429}}
"#,
        )
        .unwrap();

        let parts = fal_error_parts(&error);

        assert_eq!(
            parts.message,
            "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n"
        );
        assert_eq!(parts.provider_code, None);
    }
}
