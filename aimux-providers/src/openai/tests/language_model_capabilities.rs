use crate::openai::language_model_capabilities::{SystemMessageMode, get_model_capabilities};

mod get_open_ai_language_model_capabilities {
    use super::*;

    mod is_reasoning_model {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › isReasoningModel › %s reasoning model: %s
        #[test]
        fn reasoning_model() {
            for (model_id, expected) in [
                ("gpt-4.1", false),
                ("gpt-4.1-2025-04-14", false),
                ("gpt-4.1-mini", false),
                ("gpt-4.1-mini-2025-04-14", false),
                ("gpt-4.1-nano", false),
                ("gpt-4.1-nano-2025-04-14", false),
                ("gpt-4o", false),
                ("gpt-4o-2024-05-13", false),
                ("gpt-4o-2024-08-06", false),
                ("gpt-4o-2024-11-20", false),
                ("gpt-4o-audio-preview", false),
                ("gpt-4o-audio-preview-2024-12-17", false),
                ("gpt-4o-search-preview", false),
                ("gpt-4o-search-preview-2025-03-11", false),
                ("gpt-4o-mini-search-preview", false),
                ("gpt-4o-mini-search-preview-2025-03-11", false),
                ("gpt-4o-mini", false),
                ("gpt-4o-mini-2024-07-18", false),
                ("gpt-3.5-turbo-0125", false),
                ("gpt-3.5-turbo", false),
                ("gpt-3.5-turbo-1106", false),
                ("gpt-5-chat-latest", false),
                ("gpt-5.99-chat-latest", true),
                ("o1", true),
                ("o1-2024-12-17", true),
                ("o3-mini", true),
                ("o3-mini-2025-01-31", true),
                ("o3", true),
                ("o3-2025-04-16", true),
                ("o4", true),
                ("o4-mini", true),
                ("o4-mini-2025-04-16", true),
                ("o99", true),
                ("o99-2099-01-01", true),
                ("gpt-5", true),
                ("gpt-5-2025-08-07", true),
                ("gpt-5-codex", true),
                ("gpt-5-mini", true),
                ("gpt-5-mini-2025-08-07", true),
                ("gpt-5-nano", true),
                ("gpt-5-nano-2025-08-07", true),
                ("gpt-5-pro", true),
                ("gpt-5-pro-2025-10-06", true),
                ("gpt-5.4-mini", true),
                ("gpt-5.4-mini-2026-03-17", true),
                ("gpt-5.4-nano", true),
                ("gpt-5.4-nano-2026-03-17", true),
                ("gpt-5.5", true),
                ("gpt-5.5-2026-04-23", true),
                ("gpt-5.6", true),
                ("gpt-5.6-luna", true),
                ("gpt-5.6-sol", true),
                ("gpt-5.6-terra", true),
                ("gpt-5.99", true),
                ("gpt-6-astra", true),
                ("gpt-99", true),
                ("gpt-99-mini", true),
                ("new-unknown-model", false),
                ("ft:gpt-4o-2024-08-06:org:custom:abc123", false),
                ("ft:gpt-99:org:custom:abc123", false),
                ("acme-gpt-99-proxy", false),
                ("custom-model", false),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).is_reasoning_model,
                    expected,
                    "{model_id}"
                );
            }
        }
    }

    mod supports_non_reasoning_parameters {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › supportsNonReasoningParameters › %s supports non-reasoning parameters: %s
        #[test]
        fn supports_non_reasoning_parameters() {
            for (model_id, expected) in [
                ("gpt-5.1", true),
                ("gpt-5.1-chat-latest", true),
                ("gpt-5.1-codex-mini", true),
                ("gpt-5.1-codex", true),
                ("gpt-5.2", true),
                ("gpt-5.2-pro", true),
                ("gpt-5.2-chat-latest", true),
                ("gpt-5.3-chat-latest", true),
                ("gpt-5.4", true),
                ("gpt-5.4-mini", true),
                ("gpt-5.4-nano", true),
                ("gpt-5.4-pro", true),
                ("gpt-5.4-2026-03-05", true),
                ("gpt-5.4-mini-2026-03-17", true),
                ("gpt-5.4-nano-2026-03-17", true),
                ("gpt-5.5", true),
                ("gpt-5.5-2026-04-23", true),
                ("gpt-5.6", true),
                ("gpt-5.6-luna", true),
                ("gpt-5.6-sol", true),
                ("gpt-5.6-terra", true),
                ("gpt-5.99", true),
                ("gpt-5.100", true),
                ("gpt-6-astra", false),
                ("gpt-99", false),
                ("gpt-5", false),
                ("gpt-5.0", false),
                ("gpt-5-mini", false),
                ("gpt-5-nano", false),
                ("gpt-5-pro", false),
                ("gpt-5-chat-latest", false),
                ("ft:gpt-99:org:custom:abc123", false),
                ("acme-gpt-99-proxy", false),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).supports_non_reasoning_parameters,
                    expected,
                    "{model_id}"
                );
            }
        }
    }

    mod gpt_6_and_later_reasoning_capabilities {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › GPT-6 and later reasoning capabilities › supports the documented GPT-6.1 Sol capabilities
        #[test]
        fn supports_the_documented_gpt_6_1_sol_capabilities() {
            // `supportsAsyncToolCalling: true` has no field on the Rust
            // capabilities (see the report); every other field is compared.
            let caps = get_model_capabilities("gpt-6.1-sol");
            assert!(caps.is_reasoning_model);
            assert_eq!(caps.system_message_mode, SystemMessageMode::Developer);
            assert!(caps.supports_flex_processing);
            assert!(caps.supports_priority_processing);
            assert!(caps.supports_configuration_update);
            assert_eq!(
                caps.supported_reasoning_efforts,
                Some(&["low", "medium", "high", "xhigh", "max"][..])
            );
            assert!(!caps.supports_non_reasoning_parameters);
        }

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › GPT-6 and later reasoning capabilities › %s supports async tool calling: %s
        #[test]
        fn supports_async_tool_calling() {
            // The Rust capabilities carry no `supportsAsyncToolCalling` field;
            // the Responses request builder decides it per model id, so the
            // capability is read from whether an async tool keeps `async`.
            for (model_id, expected) in [
                ("gpt-5.6", false),
                ("gpt-6-astra", true),
                ("gpt-6.1", true),
                ("gpt-7", true),
                ("gpt-99", true),
                ("custom-model", false),
            ] {
                let options: aimux_core::options::CallOptions =
                    serde_json::from_value(serde_json::json!({
                        "prompt": [{"role": "user", "content": [{"type": "text", "text": "Hi"}]}],
                        "tools": [{
                            "type": "function",
                            "name": "get_weather",
                            "inputSchema": {"type": "object", "properties": {}},
                            "providerOptions": {"openai": {"async": true}},
                        }],
                    }))
                    .unwrap();
                let result = crate::openai::responses::build_responses_request_body(
                    model_id, &options, false,
                )
                .unwrap();
                assert_eq!(
                    result.body["tools"][0].get("async") == Some(&serde_json::json!(true)),
                    expected,
                    "{model_id}"
                );
            }
        }

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › GPT-6 and later reasoning capabilities › %s supports configuration updates: %s
        #[test]
        fn supports_configuration_updates() {
            for (model_id, expected) in [
                ("gpt-5.6", false),
                ("gpt-6-astra", true),
                ("gpt-6.1", true),
                ("gpt-99", true),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).supports_configuration_update,
                    expected,
                    "{model_id}"
                );
            }
        }

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › GPT-6 and later reasoning capabilities › %s supports the expected reasoning efforts
        #[test]
        fn supports_the_expected_reasoning_efforts() {
            let gpt6: &[&str] = &["low", "medium", "high", "xhigh", "max"];
            for (model_id, expected) in [
                ("gpt-5.6", None),
                ("gpt-6-astra", Some(gpt6)),
                ("gpt-99", Some(gpt6)),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).supported_reasoning_efforts,
                    expected,
                    "{model_id}"
                );
            }
        }
    }

    mod supports_flex_processing {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › supportsFlexProcessing › %s supports flex processing: %s
        #[test]
        fn supports_flex_processing() {
            for (model_id, expected) in [
                ("o1", false),
                ("o3", true),
                ("o4", true),
                ("o99", true),
                ("gpt-4.1", false),
                ("gpt-5", true),
                ("gpt-5.99", true),
                ("gpt-6-astra", true),
                ("gpt-99", true),
                ("gpt-99-chat-latest", false),
                ("ft:gpt-99:org:custom:abc123", false),
                ("acme-gpt-99-proxy", false),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).supports_flex_processing,
                    expected,
                    "{model_id}"
                );
            }
        }
    }

    mod supports_priority_processing {
        use super::*;

        /// upstream: @ai-sdk/openai src/openai-language-model-capabilities.test.ts › getOpenAILanguageModelCapabilities › supportsPriorityProcessing › %s supports priority processing: %s
        #[test]
        fn supports_priority_processing() {
            for (model_id, expected) in [
                ("o1", false),
                ("o3", true),
                ("o4", true),
                ("o99", true),
                ("gpt-4.1", true),
                ("gpt-5", true),
                ("gpt-5.4-nano", false),
                ("gpt-5.99", true),
                ("gpt-6-astra", true),
                ("gpt-99", true),
                ("gpt-99-nano", false),
                ("gpt-99-chat-latest", false),
                ("ft:gpt-99:org:custom:abc123", false),
                ("acme-gpt-99-proxy", false),
            ] {
                assert_eq!(
                    get_model_capabilities(model_id).supports_priority_processing,
                    expected,
                    "{model_id}"
                );
            }
        }
    }
}
