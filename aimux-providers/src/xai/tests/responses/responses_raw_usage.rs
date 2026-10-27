//! Port of `src/responses/xai-responses-raw-usage.test.ts` from
//! `@ai-sdk/xai@5.0.12`: the raw usage cases.
//!
//! The helpers are copied from `responses_language_model.rs` (the same
//! harness: [`MockFetch`] for `createTestServer`, `undefined` keys absent).

use std::sync::Arc;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::StreamResult;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};

use crate::test_support::mock_fetch::{Canned, MockFetch};
use crate::xai::{XAIProviderSettings, XaiResponsesModel, create_xai};

/// The TS `createModel()` default model id.
const DEFAULT_MODEL_ID: &str = "grok-4-fast-non-reasoning";

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: one user text message "hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("hello")]
}

/// The TS `createModel(modelId)`: the xAI provider at its default base URL,
/// `Authorization: Bearer test-key` and the mock transport.
fn create_model(fetch: &Arc<MockFetch>, model_id: &str) -> XaiResponsesModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-key".to_string()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .expect("valid settings")
    .responses(model_id)
}

/// `prepareJsonResponse(body)`: one JSON response, for the first request.
fn json_fetch(body: &Value) -> Arc<MockFetch> {
    MockFetch::new(vec![Canned::json(body)])
}

/// `prepareStreamChunks(chunks)`: each chunk as `data: <chunk>`, then
/// `data: [DONE]`.
fn sse_fetch(frames: &[String]) -> Arc<MockFetch> {
    let mut body = String::new();
    for frame in frames {
        body.push_str(&format!("data: {frame}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }])
}

/// `prepareStreamChunks([...])` with `JSON.stringify`-ed events: one event per chunk.
fn stream_fetch(events: Vec<Value>) -> Arc<MockFetch> {
    let chunks: Vec<String> = events.iter().map(Value::to_string).collect();
    sse_fetch(&chunks)
}

fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("value serializes")
}

/// `convertReadableStreamToArray(stream)` as JSON.
async fn stream_parts(result: StreamResult) -> Value {
    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(item) = stream.next().await {
        parts.push(match item {
            Ok(part) => to_json(&part),
            Err(error) => json!({ "type": "error", "error": to_json(&error) }),
        });
    }
    Value::Array(parts)
}

/// `doStream(options)` on a model of `model_id` answering from `fetch`, and the
/// parts of the stream.
async fn run_stream(fetch: &Arc<MockFetch>, model_id: &str, options: &CallOptions) -> Value {
    let result = create_model(fetch, model_id)
        .do_stream(options)
        .await
        .expect("should succeed");
    stream_parts(result).await
}

/// `parts.find(part => part.type === kind)`, or `null`.
fn first_part(parts: &Value, kind: &str) -> Value {
    parts
        .as_array()
        .expect("an array")
        .iter()
        .find(|part| part["type"] == kind)
        .cloned()
        .unwrap_or(Value::Null)
}

fn js_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => number.as_f64().map_or(Value::Number(number), |n| json!(n)),
        Value::Array(items) => Value::Array(items.into_iter().map(js_numbers).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, field)| (key, js_numbers(field)))
                .collect(),
        ),
        other => other,
    }
}

/// `expect(actual).toEqual(expected)` on JSON.
fn assert_js_eq(actual: &Value, expected: Value) {
    assert_eq!(js_numbers(actual.clone()), js_numbers(expected));
}

/// The TS `completeUsage`: usage with every provider-specific field kept.
fn complete_usage() -> Value {
    json!({
        "input_tokens": 100,
        "output_tokens": 50,
        "total_tokens": 150,
        "input_tokens_details": {
            "cached_tokens": 20,
            "provider_input_metadata": { "preserved": true }
        },
        "output_tokens_details": {
            "reasoning_tokens": 30,
            "provider_output_metadata": ["preserved", 1]
        },
        "context_details": {
            "input_tokens": 90,
            "output_tokens": 40
        },
        "server_side_tool_usage_details": {
            "web_search_calls": 1
        },
        "provider_usage_metadata": {
            "nested": { "preserved": true }
        }
    })
}

/// The TS `createResponse(usage)`: a completed response with no output.
fn create_response(usage: Value) -> Value {
    json!({
        "id": "resp_123",
        "object": "response",
        "status": "completed",
        "model": "grok-4-fast-non-reasoning",
        "output": [],
        "usage": usage
    })
}

// ─────────────────────────────────────────────────────────────────────────────

mod xai_responses_language_model_raw_usage {
    use super::*;

    /// upstream: @ai-sdk/xai src/responses/xai-responses-raw-usage.test.ts › XaiResponsesLanguageModel raw usage › preserves complete raw usage through doGenerate parsing
    #[tokio::test]
    async fn preserves_complete_raw_usage_through_do_generate_parsing() {
        let fetch = json_fetch(&create_response(complete_usage()));

        let result = create_model(&fetch, DEFAULT_MODEL_ID)
            .do_generate(&CallOptions::new(test_prompt()))
            .await
            .expect("should succeed");

        let usage = to_json(&result.usage);
        assert_js_eq(&usage["raw"], complete_usage());
        assert_js_eq(
            &usage["inputTokens"],
            json!({ "total": 100, "noCache": 80, "cacheRead": 20 }),
        );
        assert_js_eq(
            &usage["outputTokens"],
            json!({ "total": 50, "text": 20, "reasoning": 30 }),
        );
    }

    /// upstream: @ai-sdk/xai src/responses/xai-responses-raw-usage.test.ts › XaiResponsesLanguageModel raw usage › preserves complete raw usage from the final doStream usage event
    #[tokio::test]
    async fn preserves_complete_raw_usage_from_the_final_do_stream_usage_event() {
        let initial_usage = json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "provider_usage_metadata": { "stage": "initial" }
        });
        let fetch = stream_fetch(vec![
            json!({
                "type": "response.created",
                "response": {
                    "id": "resp_123",
                    "object": "response",
                    "status": "in_progress",
                    "model": "grok-4-fast-non-reasoning",
                    "output": [],
                    "usage": initial_usage
                }
            }),
            json!({
                "type": "response.completed",
                "response": create_response(complete_usage())
            }),
        ]);

        let parts = run_stream(&fetch, DEFAULT_MODEL_ID, &CallOptions::new(test_prompt())).await;

        let finish = first_part(&parts, "finish");
        assert!(!finish.is_null(), "finish part missing");
        assert_js_eq(&finish["usage"]["raw"], complete_usage());
        assert_js_eq(
            &finish["usage"]["inputTokens"],
            json!({ "total": 100, "noCache": 80, "cacheRead": 20 }),
        );
        assert_js_eq(
            &finish["usage"]["outputTokens"],
            json!({ "total": 50, "text": 20, "reasoning": 30 }),
        );
    }

    /// upstream: @ai-sdk/xai src/responses/xai-responses-raw-usage.test.ts › XaiResponsesLanguageModel raw usage › continues validating known usage field types
    #[tokio::test]
    async fn continues_validating_known_usage_field_types() {
        let mut usage = complete_usage();
        usage["input_tokens"] = json!("100");
        let fetch = json_fetch(&create_response(usage));

        let result = create_model(&fetch, DEFAULT_MODEL_ID)
            .do_generate(&CallOptions::new(test_prompt()))
            .await;

        assert!(
            result.is_err(),
            "doGenerate should reject a string input_tokens"
        );
    }
}
