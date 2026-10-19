//! A preset's `profile.structured_output_models` decides which chat models
//! get a `json_schema` response format, as the vendor package's
//! `supportsStructuredOutputs` does: Together AI only for
//! `deepseek-ai/DeepSeek-V4-Flash-0731` (@ai-sdk/togetherai 3.0.72
//! `togetherai-provider.ts:117-121,160`), Vercel never (@ai-sdk/vercel 3.0.30
//! `vercel-provider.ts`, no `supportsStructuredOutputs`), rows without the
//! key every model (Fireworks: `fireworks-provider.ts:149`).

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::types::Warning;
use aimux_provider_utils::Resolvable;
use aimux_providers::{PresetSettings, preset};
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn chat_ok() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "{}"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn schema() -> Value {
    json!({"type": "object", "properties": {"name": {"type": "string"}}})
}

/// The `response_format` a JSON-schema request to `model_id` of preset
/// `name` sends, and the warnings of the call.
async fn response_format(name: &str, model_id: &str) -> (Value, Vec<Warning>) {
    let fetch = MockFetch::new(vec![chat_ok()]);
    let provider = preset::create(
        name,
        PresetSettings {
            api_key: Some(Resolvable::Value("test-key".into())),
            fetch: Some(fetch.transport()),
            ..Default::default()
        },
    )
    .unwrap();
    let mut options = CallOptions::new(vec![LanguageModelMessage::user_text("hi")]);
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(schema()),
        name: None,
        description: None,
    });
    let result = provider.chat(model_id).do_generate(&options).await.unwrap();
    (
        fetch.seen()[0].json_body()["response_format"].clone(),
        result.warnings,
    )
}

fn json_schema() -> Value {
    json!({"type": "json_schema", "json_schema": {"schema": schema(), "strict": true, "name": "response"}})
}

#[tokio::test]
async fn together_ai_sends_a_schema_only_to_its_structured_output_model() {
    let (format, warnings) =
        response_format("togetherai", "deepseek-ai/DeepSeek-V4-Flash-0731").await;
    assert_eq!(format, json_schema());
    assert!(warnings.is_empty(), "{warnings:?}");

    let (format, warnings) =
        response_format("togetherai", "meta-llama/Llama-3.3-70B-Instruct-Turbo").await;
    assert_eq!(format, json!({"type": "json_object"}));
    assert!(
        matches!(&warnings[..], [Warning::Unsupported { feature, .. }] if feature == "responseFormat"),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn vercel_never_sends_a_schema() {
    let (format, warnings) = response_format("vercel", "v0-1.5-md").await;
    assert_eq!(format, json!({"type": "json_object"}));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[tokio::test]
async fn a_row_without_a_model_list_sends_a_schema_to_every_model() {
    let (format, warnings) = response_format(
        "fireworks",
        "accounts/fireworks/models/llama-v3p3-70b-instruct",
    )
    .await;
    assert_eq!(format, json_schema());
    assert!(warnings.is_empty(), "{warnings:?}");
}
