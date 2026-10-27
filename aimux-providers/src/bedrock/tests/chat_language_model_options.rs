//! `amazonBedrockLanguageModelChatOptions` is a zod schema upstream. aimux
//! validates the same options where the request body is built
//! (`build_request_body_checked`), so `safeParse(options)` is that call with
//! `{ amazonBedrock: options }` as the provider options: `Ok` is
//! `success: true`, an `Err` is `success: false`.

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;

use crate::bedrock::chat_language_model::build_request_body_checked;

/// `amazonBedrockLanguageModelChatOptions.safeParse(options)`: the request
/// body built with `options`, or the validation error.
fn safe_parse(options: Value) -> Result<Value, AiMuxError> {
    let call: CallOptions = serde_json::from_value(json!({
        "prompt": [{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }],
        "providerOptions": { "amazonBedrock": options },
    }))
    .expect("valid call options");
    build_request_body_checked("anthropic.claude-3-haiku-20240307-v1:0", &call, None)
        .map(|(body, ..)| body)
}

mod amazon_bedrock_language_model_chat_options {
    use super::*;

    mod structured_output_mode {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-options.test.ts › amazonBedrockLanguageModelChatOptions › structuredOutputMode › accepts %s
        ///
        /// The parsed value (`result.data`) has no Rust form: the mode only
        /// steers how the body is built, so only acceptance is asserted.
        #[test]
        fn accepts() {
            for structured_output_mode in ["outputFormat", "jsonTool", "auto"] {
                let result = safe_parse(json!({ "structuredOutputMode": structured_output_mode }));

                assert!(result.is_ok(), "{structured_output_mode}: {result:?}");
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-options.test.ts › amazonBedrockLanguageModelChatOptions › structuredOutputMode › rejects invalid values
        #[test]
        fn rejects_invalid_values() {
            let result = safe_parse(json!({ "structuredOutputMode": "native" }));

            assert!(result.is_err(), "{result:?}");
        }
    }

    mod service_tier {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-options.test.ts › amazonBedrockLanguageModelChatOptions › serviceTier › accepts valid service tier values
        #[test]
        fn accepts_valid_service_tier_values() {
            let valid_values = ["reserved", "priority", "default", "flex"];

            for value in valid_values {
                let result = safe_parse(json!({ "serviceTier": value }));

                let body = result.unwrap_or_else(|error| panic!("{value}: {error}"));
                // `result.data.serviceTier` is what the body carries.
                assert_eq!(body["serviceTier"]["type"], json!(value));
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-chat-language-model-options.test.ts › amazonBedrockLanguageModelChatOptions › serviceTier › rejects invalid service tier values
        #[test]
        fn rejects_invalid_service_tier_values() {
            let invalid_values = ["on-demand", "auto", "standard", "", "PRIORITY"];

            for value in invalid_values {
                let result = safe_parse(json!({ "serviceTier": value }));

                assert!(result.is_err(), "{value:?}: {result:?}");
            }
        }
    }
}
