//! Non-object input of a past tool call (RFC-0042 §6.2): each package
//! re-encodes it the way its upstream converter does.
//!
//! - OpenAI chat writes `"{}"` (`@ai-sdk/openai@4.0.83`
//!   `src/chat/convert-to-openai-chat-messages.ts:15-21`, used at `:287`;
//!   aimux `openai/convert.rs:479-483`).
//! - Groq and DeepSeek write `JSON.stringify(input)` (`@ai-sdk/groq@4.0.52`
//!   `src/convert-to-groq-chat-messages.ts:105`; `@ai-sdk/deepseek@3.0.56`
//!   `src/chat/convert-to-deepseek-chat-messages.ts:336`; aimux
//!   `groq/convert_to_chat_messages.rs:88`, `deepseek/convert.rs:286`).
//! - Anthropic wraps it as `{ rawInvalidInput: input }`
//!   (`@ai-sdk/anthropic@4.0.71` `src/convert-to-anthropic-prompt.ts:1668-1673`,
//!   used at `:1013`; aimux `anthropic/convert.rs:821-828`).

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, ToolCallPart};
use aimux_core::options::CallOptions;
use aimux_providers::anthropic::{AnthropicProviderSettings, create_anthropic};
use aimux_providers::deepseek::{DeepSeekProviderSettings, create_deepseek};
use aimux_providers::groq::{GroqProviderSettings, create_groq};
use aimux_providers::openai::{OpenAIProviderSettings, create_openai};
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

/// The inputs of the past tool calls: five non-objects, then an object as
/// the control.
fn inputs() -> Vec<Value> {
    vec![
        json!(null),
        json!("raw text"),
        json!([1, 2]),
        json!(42),
        json!(true),
        json!({"a": 1}),
    ]
}

/// A user turn, then one assistant message holding a tool call per input.
fn prompt() -> CallOptions {
    let calls = inputs()
        .into_iter()
        .enumerate()
        .map(|(i, input)| {
            AssistantPart::ToolCall(ToolCallPart {
                tool_call_id: format!("call_{i}"),
                tool_name: "lookup".into(),
                input,
                provider_executed: None,
                provider_options: None,
            })
        })
        .collect();
    CallOptions {
        max_output_tokens: Some(64),
        ..CallOptions::new(vec![
            LanguageModelMessage::user_text("Look it up."),
            LanguageModelMessage::Assistant {
                content: calls,
                provider_options: None,
            },
        ])
    }
}

fn chat_completion() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"},
                     "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn anthropic_message() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-test",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
}

/// The one request's assistant message.
fn assistant_message(fetch: &MockFetch) -> Value {
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    seen[0].json_body()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "assistant")
        .cloned()
        .expect("an assistant message")
}

/// The `function.arguments` strings of a chat-completions assistant message.
fn chat_arguments(fetch: &MockFetch) -> Vec<String> {
    assistant_message(fetch)["tool_calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call["function"]["arguments"].as_str().unwrap().to_owned())
        .collect()
}

/// What JSON.stringify writes for each input.
const STRINGIFIED: [&str; 6] = ["null", "\"raw text\"", "[1,2]", "42", "true", r#"{"a":1}"#];

#[tokio::test]
async fn openai_chat_sends_an_empty_object_for_non_object_input() {
    let fetch = MockFetch::new(vec![chat_completion()]);
    let model = create_openai(OpenAIProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://example.test/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gpt-4o");

    // Only the request is asserted.
    let _ = model.do_generate(&prompt()).await;

    assert_eq!(
        chat_arguments(&fetch),
        ["{}", "{}", "{}", "{}", "{}", r#"{"a":1}"#]
    );
}

#[tokio::test]
async fn groq_stringifies_non_object_input() {
    let fetch = MockFetch::new(vec![chat_completion()]);
    let model = create_groq(GroqProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://example.test/openai/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("llama-3.3-70b-versatile");

    let _ = model.do_generate(&prompt()).await;

    assert_eq!(chat_arguments(&fetch), STRINGIFIED);
}

#[tokio::test]
async fn deepseek_stringifies_non_object_input() {
    let fetch = MockFetch::new(vec![chat_completion()]);
    let model = create_deepseek(DeepSeekProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://example.test".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("deepseek-chat");

    let _ = model.do_generate(&prompt()).await;

    assert_eq!(chat_arguments(&fetch), STRINGIFIED);
}

#[tokio::test]
async fn anthropic_wraps_non_object_input_as_raw_invalid_input() {
    let fetch = MockFetch::new(vec![anthropic_message()]);
    let model = create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some("https://test.invalid/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-test");

    let _ = model.do_generate(&prompt()).await;

    let inputs: Vec<Value> = assistant_message(&fetch)["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| block["input"].clone())
        .collect();
    assert_eq!(
        inputs,
        [
            json!({"rawInvalidInput": null}),
            json!({"rawInvalidInput": "raw text"}),
            json!({"rawInvalidInput": [1, 2]}),
            json!({"rawInvalidInput": 42}),
            json!({"rawInvalidInput": true}),
            json!({"a": 1}),
        ]
    );
}
