//! The shared helpers of the older ports (previously `tests/anthropic_remaining_test.rs`),
//! used by their parts across `src/anthropic/tests/`.

use crate::anthropic::{AnthropicMessagesModel, AnthropicProviderSettings, create_anthropic};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;

/// The TS `TEST_PROMPT`: a single user text message "Hello".
pub(super) fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

pub(super) fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

pub(super) fn make_model_with_config(
    settings: AnthropicProviderSettings,
) -> AnthropicMessagesModel {
    create_anthropic(settings)
        .unwrap()
        .messages("claude-3-haiku-20240307")
}

pub(super) fn text_response(text: &str) -> Value {
    json!({
        "id": "msg_123", "type": "message", "role": "assistant",
        "content": [{ "type": "text", "text": text }],
        "model": "claude-3-haiku-20240307", "stop_reason": "end_turn",
        "stop_sequence": null, "usage": { "input_tokens": 1, "output_tokens": 1 }
    })
}

pub(super) fn mock() -> Arc<MockFetch> {
    MockFetch::new(vec![Canned::json(&text_response("Hi"))])
}

pub(super) fn settings(fetch: &Arc<MockFetch>) -> AnthropicProviderSettings {
    AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

pub(super) async fn request_url(
    settings: AnthropicProviderSettings,
    fetch: &Arc<MockFetch>,
) -> String {
    make_model_with_config(settings)
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap();
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    requests[0].url.clone()
}

/// A `text/event-stream` response of one `data:` event per chunk.
pub(super) fn sse_response(chunks: &[Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}

pub(super) fn stream_message_start(id: &str) -> Value {
    json!({
        "type": "message_start",
        "message": {
            "id": id, "type": "message", "role": "assistant", "content": [],
            "model": "claude-3-haiku-20240307", "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 17, "output_tokens": 1 }
        }
    })
}

/// `doStream`, then every stream item.
pub(super) async fn stream_items(
    fetch: &Arc<MockFetch>,
    options: &CallOptions,
) -> Vec<Result<StreamPart, AiMuxError>> {
    make_model_with_config(settings(fetch))
        .do_stream(options)
        .await
        .unwrap()
        .stream
        .collect()
        .await
}
