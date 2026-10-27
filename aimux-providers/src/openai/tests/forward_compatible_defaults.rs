use std::sync::Arc;

use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use serde_json::{Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

const CHAT_PARAMETER_ERROR: &str =
    include_str!("fixtures/reasoning-model-legacy-parameter-error.json");
const RESPONSES_TEMPERATURE_ERROR: &str =
    include_str!("fixtures/reasoning-model-temperature-error.json");
const IMAGE_RESPONSE_FORMAT_ERROR: &str =
    include_str!("fixtures/gpt-image-response-format-error.json");

fn prompt() -> Value {
    json!([
        {"role": "system", "content": "Follow the instructions."},
        {"role": "user", "content": [{"type": "text", "text": "Say ok."}]},
    ])
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn error_response(body: &Value) -> Canned {
    Canned {
        status: 400,
        ..Canned::json(body)
    }
}

fn successful_chat_response(model: &Value) -> Value {
    json!({
        "id": "chatcmpl_test",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop",
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
}

fn successful_responses_response(model: &Value) -> Value {
    json!({
        "id": "resp_test",
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "model": model,
        "output": [],
        "usage": {
            "input_tokens": 1,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": 1,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": 2,
        },
    })
}

fn provider(handler: impl Fn(&Seen) -> Canned + Send + Sync + 'static) -> OpenAIProvider {
    let fetch: Arc<RouteFetch> = RouteFetch::new(handler);
    create_openai(OpenAIProviderSettings {
        base_url: Some("https://api.openai.com/v1".into()),
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `== null` in JavaScript: absent or null.
fn is_nullish(body: &Value, key: &str) -> bool {
    body.get(key).is_none_or(Value::is_null)
}

mod open_ai_forward_compatible_model_family_defaults {
    use super::*;

    /// upstream: @ai-sdk/openai src/openai-forward-compatible-defaults.test.ts › OpenAI forward-compatible model-family defaults › uses reasoning-safe Chat Completions request defaults for gpt-99
    #[tokio::test]
    async fn uses_reasoning_safe_chat_completions_request_defaults_for_gpt_99() {
        let provider = provider(|seen| {
            let body = seen.json_body();
            let is_reasoning_safe = body["messages"][0]["role"] == "developer"
                && body["max_completion_tokens"] == 64
                && is_nullish(&body, "max_tokens")
                && is_nullish(&body, "temperature")
                && is_nullish(&body, "top_p")
                && is_nullish(&body, "frequency_penalty")
                && is_nullish(&body, "presence_penalty")
                && is_nullish(&body, "logit_bias")
                && is_nullish(&body, "logprobs");
            if is_reasoning_safe {
                Canned::json(&successful_chat_response(&body["model"]))
            } else {
                error_response(&fixture(CHAT_PARAMETER_ERROR))
            }
        });

        let options: CallOptions = serde_json::from_value(json!({
            "prompt": prompt(),
            "maxOutputTokens": 64,
            "temperature": 0.2,
            "topP": 0.8,
            "frequencyPenalty": 0.1,
            "presencePenalty": 0.1,
            "providerOptions": {
                "openai": {"logitBias": {"1": 1}, "logprobs": 2},
            },
        }))
        .unwrap();

        let result = provider.chat("gpt-99").do_generate(&options).await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/openai src/openai-forward-compatible-defaults.test.ts › OpenAI forward-compatible model-family defaults › uses reasoning-safe Responses API request defaults for gpt-99
    #[tokio::test]
    async fn uses_reasoning_safe_responses_api_request_defaults_for_gpt_99() {
        let provider = provider(|seen| {
            let body = seen.json_body();
            let is_reasoning_safe = body["input"][0]["role"] == "developer"
                && is_nullish(&body, "temperature")
                && is_nullish(&body, "top_p");
            if is_reasoning_safe {
                Canned::json(&successful_responses_response(&body["model"]))
            } else {
                error_response(&fixture(RESPONSES_TEMPERATURE_ERROR))
            }
        });

        let options: CallOptions = serde_json::from_value(json!({
            "prompt": prompt(),
            "temperature": 0.2,
            "topP": 0.8,
            "providerOptions": {"openai": {"reasoningEffort": "medium"}},
        }))
        .unwrap();

        let result = provider.responses("gpt-99").do_generate(&options).await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// upstream: @ai-sdk/openai src/openai-forward-compatible-defaults.test.ts › OpenAI forward-compatible model-family defaults › uses GPT Image family defaults for gpt-image-99
    #[tokio::test]
    async fn uses_gpt_image_family_defaults_for_gpt_image_99() {
        let provider = provider(|seen| {
            let body = seen.json_body();
            if is_nullish(&body, "response_format") {
                Canned::json(&json!({
                    "created": 1,
                    "data": [{"b64_json": "image"}, {"b64_json": "image"}],
                }))
            } else {
                error_response(&fixture(IMAGE_RESPONSE_FORMAT_ERROR))
            }
        });
        let model = provider.image("gpt-image-99");

        assert_eq!(model.max_images_per_call(), Some(10));
        let options: ImageCallOptions = serde_json::from_value(json!({
            "prompt": "A black square.",
            "n": 2,
            "providerOptions": {},
        }))
        .unwrap();
        let result = model.do_generate(&options).await;
        assert!(result.is_ok(), "{result:?}");
    }
}
