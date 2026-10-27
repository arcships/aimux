use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use serde_json::json;

use crate::openai::{OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};

mod openai_error_data_schema {
    use super::*;

    /// upstream: @ai-sdk/openai src/openai-error.test.ts › openaiErrorDataSchema › should parse OpenRouter resource exhausted error
    #[tokio::test]
    async fn should_parse_open_router_resource_exhausted_error() {
        // `openaiErrorDataSchema` is parsed by the OpenAI failed-response
        // handler; the error body reaches it through a failing call.
        let error = "\n{\"error\":{\"message\":\"{\\n  \\\"error\\\": {\\n    \\\"code\\\": 429,\\n    \\\"message\\\": \\\"Resource has been exhausted (e.g. check quota).\\\",\\n    \\\"status\\\": \\\"RESOURCE_EXHAUSTED\\\"\\n  }\\n}\\n\",\"code\":429}}\n";
        let fetch = MockFetch::new(vec![Canned {
            status: 429,
            headers: vec![("content-type".into(), "application/json".into())],
            body: error.as_bytes().to_vec(),
        }]);
        let provider = create_openai(OpenAIProviderSettings {
            base_url: Some("https://api.openai.com/v1".into()),
            api_key: Some("test-api-key".into()),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap();

        let result = provider
            .chat("gpt-4o")
            .do_generate(&CallOptions::new(vec![LanguageModelMessage::user_text(
                "Hello",
            )]))
            .await;

        let Err(AiMuxError::ApiCall(details)) = result else {
            panic!("expected an API call error, got {result:?}");
        };
        let value = json!({
            "error": {
                "message": "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n",
                "code": 429,
            },
        });
        assert_eq!(details.data, Some(value.clone()));
        assert_eq!(details.message, value["error"]["message"].as_str().unwrap());
        assert_eq!(details.provider_code.as_deref(), Some("429"));
        assert_eq!(details.response_body.as_deref(), Some(error));
    }
}
