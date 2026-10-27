//! Port of `@ai-sdk/open-responses@2.0.58` `src/open-responses-provider.test.ts`.
//!
//! Upstream's `url: 'https://example.com/responses'` is `base_url`
//! `https://example.com` here (the model posts to `{base_url}/responses`).

use serde_json::{Value, json};

use aimux_core::options::CallOptions;

use crate::open_responses::{OpenResponsesProviderSettings, create_open_responses};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// The upstream `prompt`.
fn prompt() -> Value {
    json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
}

/// The upstream `error`.
fn error() -> Value {
    json!({
        "code": "busy",
        "message": "Try again later.",
        "http_status": 503,
        "retry": false,
    })
}

mod endpoint_specific_response_error_metadata {
    use super::*;

    /// upstream: @ai-sdk/open-responses src/open-responses-provider.test.ts › endpoint-specific response error metadata › does not interpret vendor error fields unless configured
    #[tokio::test]
    async fn does_not_interpret_vendor_error_fields_unless_configured() {
        let mut error = error();
        error["status_code"] = json!(503);
        let mock = MockFetch::new(vec![Canned::json(&json!({
            "error": error,
            "status": "failed",
            "output": [],
        }))]);
        let mut settings = OpenResponsesProviderSettings::new("acme", "https://example.com");
        settings.fetch = Some(mock.transport());
        let provider = create_open_responses(settings).expect("valid settings");

        let options: CallOptions = serde_json::from_value(json!({ "prompt": prompt() })).unwrap();
        let Err(error) = provider.call("model").do_generate(&options).await else {
            panic!("doGenerate should reject");
        };
        assert_eq!(error.status_code(), Some(400), "{error:?}");
        assert!(!error.is_retryable(), "{error:?}");
    }
}
