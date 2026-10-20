//! Ports of `src/chat/deepseek-raw-usage.test.ts` from `@ai-sdk/deepseek@3.0.56`:
//! the complete provider usage object is kept as `usage.raw` through
//! `doGenerate` and through the final `doStream` usage event, and the known
//! usage fields are still validated.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - The `cacheWrite: undefined` of the TypeScript `toStrictEqual` is `None` in
//!   Rust, and the JSON form of the token counts omits it. The usage counts are
//!   therefore compared as JSON, where an absent `cacheWrite` is the `undefined`.
//! - The `it.each` rows are one function that checks each row in turn.
//! - Responses are served by [`MockFetch`].

use std::sync::Arc;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_providers::deepseek::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek};
use futures::StreamExt;
use serde_json::{Value, json};

use crate::mock_fetch::{Canned, MockFetch};

const MODEL: &str = "deepseek-v4-flash";

// ── helpers ─────────────────────────────────────────────────────────────────

/// `createDeepSeek({ apiKey })` with a mock transport.
fn provider(fetch: &Arc<MockFetch>) -> DeepSeekProvider {
    create_deepseek(DeepSeekProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `prompt: TEST_PROMPT`.
fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// `completeUsage`: every field the raw usage must keep.
fn complete_usage() -> Value {
    json!({
        "prompt_tokens": 100,
        "completion_tokens": 50,
        "total_tokens": 150,
        "prompt_cache_hit_tokens": 20,
        "prompt_cache_miss_tokens": 80,
        "prompt_tokens_details": {
            "cached_tokens": 20,
            "provider_prompt_metadata": { "preserved": true }
        },
        "completion_tokens_details": {
            "reasoning_tokens": 10,
            "provider_completion_metadata": ["preserved", 1]
        },
        "provider_usage_metadata": {
            "nested": { "preserved": true }
        }
    })
}

/// `createResponse(usage)`: a non-streaming completion carrying `usage`.
fn create_response(usage: Value) -> Value {
    json!({
        "id": "chatcmpl-raw-usage",
        "object": "chat.completion",
        "created": 1785880000,
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello" },
            "finish_reason": "stop"
        }],
        "usage": usage
    })
}

/// An event-stream mock: one `data:` frame per payload, then `[DONE]`.
fn stream_fetch(payloads: &[String]) -> Arc<MockFetch> {
    let mut body: String = payloads
        .iter()
        .map(|payload| format!("data: {payload}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![
            ("content-type".to_string(), "text/event-stream".to_string()),
            ("cache-control".to_string(), "no-cache".to_string()),
            ("connection".to_string(), "keep-alive".to_string()),
        ],
        body: body.into_bytes(),
    }])
}

mod deep_seek_chat_language_model_raw_usage {
    use super::*;

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-raw-usage.test.ts › DeepSeekChatLanguageModel raw usage › preserves complete raw usage through doGenerate parsing
    #[tokio::test]
    async fn preserves_complete_raw_usage_through_do_generate_parsing() {
        let fetch = MockFetch::new(vec![Canned::json(&create_response(complete_usage()))]);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_generate(&options())
            .await
            .unwrap();

        assert_eq!(result.usage.raw, complete_usage().as_object().cloned());
        assert_eq!(
            serde_json::to_value(&result.usage.input_tokens).unwrap(),
            json!({ "total": 100, "noCache": 80, "cacheRead": 20 })
        );
        assert_eq!(
            serde_json::to_value(&result.usage.output_tokens).unwrap(),
            json!({ "total": 50, "text": 40, "reasoning": 10 })
        );
    }

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-raw-usage.test.ts › DeepSeekChatLanguageModel raw usage › preserves complete raw usage from the final doStream usage event
    #[tokio::test]
    async fn preserves_complete_raw_usage_from_the_final_do_stream_usage_event() {
        let initial_usage = json!({
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
            "provider_usage_metadata": { "stage": "initial" }
        });
        let payloads = [
            json!({
                "id": "chatcmpl-raw-usage",
                "object": "chat.completion.chunk",
                "choices": [],
                "usage": initial_usage
            })
            .to_string(),
            json!({
                "id": "chatcmpl-raw-usage",
                "object": "chat.completion.chunk",
                "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                "usage": complete_usage()
            })
            .to_string(),
        ];
        let fetch = stream_fetch(&payloads);
        let result = provider(&fetch)
            .chat(MODEL)
            .do_stream(&options())
            .await
            .unwrap();
        let parts: Vec<StreamPart> = result
            .stream
            .map(|part| part.expect("stream part"))
            .collect()
            .await;
        let usage = parts
            .iter()
            .find_map(|part| match part {
                StreamPart::Finish { usage, .. } => Some(usage.clone()),
                _ => None,
            })
            .expect("a finish part");

        assert_eq!(usage.raw, complete_usage().as_object().cloned());
        assert_eq!(
            serde_json::to_value(&usage.input_tokens).unwrap(),
            json!({ "total": 100, "noCache": 80, "cacheRead": 20 })
        );
        assert_eq!(
            serde_json::to_value(&usage.output_tokens).unwrap(),
            json!({ "total": 50, "text": 40, "reasoning": 10 })
        );
    }

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-raw-usage.test.ts › DeepSeekChatLanguageModel raw usage › continues validating known usage field %s
    #[tokio::test]
    async fn continues_validating_known_usage_field() {
        let mut prompt_tokens = complete_usage();
        prompt_tokens["prompt_tokens"] = json!("100");

        let mut cached_tokens = complete_usage();
        cached_tokens["prompt_tokens_details"]["cached_tokens"] = json!("20");

        let mut reasoning_tokens = complete_usage();
        reasoning_tokens["completion_tokens_details"]["reasoning_tokens"] = json!("10");

        let rows = [
            ("prompt_tokens", prompt_tokens),
            ("prompt_tokens_details.cached_tokens", cached_tokens),
            (
                "completion_tokens_details.reasoning_tokens",
                reasoning_tokens,
            ),
        ];
        // Every row runs; the rows the provider accepts are reported together.
        let mut accepted = Vec::new();
        for (field, usage) in rows {
            let fetch = MockFetch::new(vec![Canned::json(&create_response(usage))]);
            let outcome = provider(&fetch).chat(MODEL).do_generate(&options()).await;
            if outcome.is_ok() {
                accepted.push(field);
            }
        }
        assert!(
            accepted.is_empty(),
            "a string count was accepted for: {accepted:?}"
        );
    }
}
