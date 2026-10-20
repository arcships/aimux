//! Upstream `groq-chat-language-model-options.test.ts`.
//!
//! The zod `safeParse` calls are `serde_json::from_value` on
//! [`GroqLanguageModelChatOptions`]: success is `Ok`, a rejected value is `Err`.
//! JavaScript `undefined` has no JSON form and is the absent key on the wire,
//! so the "undefined" and "omitted explicitly" cases both send `{}`. The
//! `type inference` case checks only TypeScript types and is not ported.

use serde_json::{Value, json};

use crate::groq::options::GroqLanguageModelChatOptions;

/// The schema check of one `groqLanguageModelChatOptions` input.
fn safe_parse(input: Value) -> Result<GroqLanguageModelChatOptions, serde_json::Error> {
    serde_json::from_value(input)
}

mod groq_language_model_chat_options {
    use super::*;

    mod reasoning_effort {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › reasoningEffort › accepts valid reasoningEffort values
        #[test]
        fn accepts_valid_reasoning_effort_values() {
            for value in ["none", "default", "low", "medium", "high"] {
                let options = safe_parse(json!({ "reasoningEffort": value }))
                    .expect("a valid reasoningEffort");

                assert_eq!(
                    serde_json::to_value(options.reasoning_effort).expect("enum serializes"),
                    json!(value)
                );
            }
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › reasoningEffort › rejects invalid reasoningEffort values
        #[test]
        fn rejects_invalid_reasoning_effort_values() {
            for value in ["invalid", "high-effort", "minimal", "maximum", ""] {
                assert!(
                    safe_parse(json!({ "reasoningEffort": value })).is_err(),
                    "{value:?} must be rejected"
                );
            }
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › reasoningEffort › allows reasoningEffort to be undefined
        #[test]
        fn allows_reasoning_effort_to_be_undefined() {
            let options = safe_parse(json!({})).expect("no options is valid");

            assert_eq!(
                serde_json::to_value(options.reasoning_effort).expect("enum serializes"),
                Value::Null
            );
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › reasoningEffort › allows reasoningEffort to be omitted explicitly
        #[test]
        fn allows_reasoning_effort_to_be_omitted_explicitly() {
            // `{ reasoningEffort: undefined }` is `{}` on the wire.
            let options = safe_parse(json!({})).expect("an omitted reasoningEffort is valid");

            assert_eq!(
                serde_json::to_value(options.reasoning_effort).expect("enum serializes"),
                Value::Null
            );
        }
    }

    mod combined_options_with_reasoning_effort {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › combined options with reasoningEffort › accepts reasoningEffort with other valid options
        #[test]
        fn accepts_reasoning_effort_with_other_valid_options() {
            let options = safe_parse(json!({
                "reasoningEffort": "high",
                "parallelToolCalls": true,
                "user": "test-user",
                "structuredOutputs": false,
                "serviceTier": "flex",
            }))
            .expect("valid combined options");

            assert_eq!(
                serde_json::to_value(options.reasoning_effort).expect("enum serializes"),
                json!("high")
            );
            assert_eq!(options.parallel_tool_calls, Some(true));
            assert_eq!(options.user.as_deref(), Some("test-user"));
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › combined options with reasoningEffort › rejects when reasoningEffort is invalid among valid options
        #[test]
        fn rejects_when_reasoning_effort_is_invalid_among_valid_options() {
            let result = safe_parse(json!({
                "reasoningEffort": "ultra-high",
                "parallelToolCalls": true,
                "user": "test-user",
            }));

            assert!(result.is_err());
        }
    }

    mod service_tier {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › serviceTier › accepts valid serviceTier values
        #[test]
        fn accepts_valid_service_tier_values() {
            for value in ["on_demand", "performance", "flex", "auto"] {
                let options =
                    safe_parse(json!({ "serviceTier": value })).expect("a valid serviceTier");

                assert_eq!(
                    serde_json::to_value(options.service_tier).expect("enum serializes"),
                    json!(value)
                );
            }
        }

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › serviceTier › rejects invalid serviceTier values
        #[test]
        fn rejects_invalid_service_tier_values() {
            for value in ["priority", "default", "turbo", ""] {
                assert!(
                    safe_parse(json!({ "serviceTier": value })).is_err(),
                    "{value:?} must be rejected"
                );
            }
        }
    }

    mod all_reasoning_effort_enum_variants {
        use super::*;

        /// upstream: @ai-sdk/groq src/groq-chat-language-model-options.test.ts › groqLanguageModelChatOptions › all reasoningEffort enum variants › validates all reasoningEffort variants individually
        #[test]
        fn validates_all_reasoning_effort_variants_individually() {
            for variant in ["none", "default", "low", "medium", "high"] {
                let options = safe_parse(json!({ "reasoningEffort": variant }))
                    .expect("each variant is valid");

                assert_eq!(
                    serde_json::to_value(options.reasoning_effort).expect("enum serializes"),
                    json!(variant)
                );
            }
        }
    }
}
