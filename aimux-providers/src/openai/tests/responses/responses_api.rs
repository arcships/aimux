//! The upstream zod chunk/response schemas have no Rust type: aimux reads each
//! Responses event as JSON in the stream reducer
//! (`responses_convert::build_responses_event_stream`), so a chunk the schema
//! accepts must pass through the reducer without an `error` part, and a chunk
//! it rejects must surface one (as upstream's `doStream` does for a chunk that
//! fails validation). Usage is the one typed part (`types::ResponsesUsage`,
//! read with `parse_usage` and reported raw by `convert_responses_usage`).

use std::collections::HashMap;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::openai::responses::convert::parse_usage;
use crate::openai::responses::convert_responses_usage;
use crate::openai::responses::responses_convert::build_responses_event_stream;

/// Run one chunk through the stream reducer (after a `response.created`
/// chunk, so the reducer's first-event error check is not what is tested)
/// and return every stream part as JSON.
async fn stream_parts(chunk: Value) -> Vec<Value> {
    let first = json!({
        "type": "response.created",
        "response": {"id": "resp_test", "created_at": 0, "model": "gpt-5"},
    });
    let stream = build_responses_event_stream(
        Some(Ok(first)),
        futures::stream::iter(vec![Ok(chunk)]),
        "openai".into(),
        Vec::new(),
        false,
        "https://api.openai.com/v1/responses".into(),
        json!({}),
        HashMap::new(),
    )
    .unwrap();
    stream
        .map(|part| match part {
            Ok(part) => serde_json::to_value(part).unwrap(),
            Err(error) => json!({"type": "error", "error": error.to_string()}),
        })
        .collect()
        .await
}

fn error_parts(parts: &[Value]) -> Vec<&Value> {
    parts
        .iter()
        .filter(|part| part["type"] == "error")
        .collect()
}

async fn assert_accepted(chunk: Value) {
    let parts = stream_parts(chunk.clone()).await;
    assert_eq!(
        error_parts(&parts),
        Vec::<&Value>::new(),
        "chunk {chunk} was rejected"
    );
    assert_eq!(parts.last().unwrap()["type"], "finish", "{parts:#?}");
}

async fn assert_rejected(chunk: Value) {
    let parts = stream_parts(chunk.clone()).await;
    assert!(
        !error_parts(&parts).is_empty(),
        "chunk {chunk} was accepted: {parts:#?}"
    );
}

const ITEM_EVENTS: [&str; 2] = ["response.output_item.added", "response.output_item.done"];

mod openai_responses_chunk_schema {
    use super::*;

    mod each {
        use super::*;

        /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › %s › rejects events with $name
        #[tokio::test]
        async fn rejects_events_with_name() {
            for event_type in ITEM_EVENTS {
                for (name, item_property) in [
                    ("missing item", json!({})),
                    ("null item", json!({"item": null})),
                    ("non-object item", json!({"item": "invalid"})),
                    ("type-less item", json!({"item": {"id": "item_1"}})),
                ] {
                    let mut chunk = json!({"type": event_type, "output_index": 0});
                    for (key, value) in item_property.as_object().unwrap() {
                        chunk[key] = value.clone();
                    }
                    let parts = stream_parts(chunk).await;
                    assert!(
                        !error_parts(&parts).is_empty(),
                        "{event_type} with {name} was accepted: {parts:#?}"
                    );
                }
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › %s › accepts responses with $name
        #[tokio::test]
        async fn accepts_responses_with_name() {
            for event_type in ["response.completed", "response.incomplete"] {
                for (_name, response) in [
                    ("missing usage", json!({})),
                    ("null usage", json!({"usage": null})),
                ] {
                    assert_accepted(json!({"type": event_type, "response": response})).await;
                }
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › keeps future item types forward compatible for %s
    #[tokio::test]
    async fn keeps_future_item_types_forward_compatible_for() {
        // Upstream maps the chunk to `unknown_chunk`, which `doStream` skips.
        for event_type in ITEM_EVENTS {
            let parts = stream_parts(json!({
                "type": event_type,
                "output_index": 0,
                "item": {"id": "item_1", "type": "future_output_item"},
            }))
            .await;
            assert_eq!(error_parts(&parts), Vec::<&Value>::new(), "{event_type}");
            let types: Vec<&Value> = parts.iter().map(|part| &part["type"]).collect();
            assert_eq!(
                types,
                vec!["stream-start", "response-metadata", "finish"],
                "{event_type}"
            );
        }
    }

    mod local_shell_call {
        use super::*;

        fn item(event_type: &str) -> Value {
            json!({
                "id": "item_1",
                "type": "local_shell_call",
                "call_id": "call_1",
                "status": if event_type == "response.output_item.added" { "in_progress" } else { "completed" },
                "action": {"type": "exec", "command": ["ls"], "env": {}},
            })
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › %s local_shell_call › accepts valid events
        #[tokio::test]
        async fn accepts_valid_events() {
            for event_type in ITEM_EVENTS {
                assert_accepted(json!({
                    "type": event_type,
                    "output_index": 0,
                    "item": item(event_type),
                }))
                .await;
            }
        }

        /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › %s local_shell_call › rejects events without output_index
        #[tokio::test]
        async fn rejects_events_without_output_index() {
            for event_type in ITEM_EVENTS {
                assert_rejected(json!({"type": event_type, "item": item(event_type)})).await;
            }
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › accepts valid function call arguments done events
    #[tokio::test]
    async fn accepts_valid_function_call_arguments_done_events() {
        assert_accepted(json!({
            "type": "response.function_call_arguments.done",
            "item_id": "item_1",
            "output_index": 0,
            "arguments": "{\"city\":\"Berlin\"}",
        }))
        .await;
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › openaiResponsesChunkSchema › rejects malformed function call arguments done events
    #[tokio::test]
    async fn rejects_malformed_function_call_arguments_done_events() {
        assert_rejected(json!({
            "type": "response.function_call_arguments.done",
            "item_id": "item_1",
            "arguments": "{\"city\":\"Berlin\"}",
        }))
        .await;
    }
}

fn usage() -> Value {
    json!({
        "input_tokens": 12,
        "input_tokens_details": {
            "cached_tokens": 2,
            "cache_write_tokens": 1,
            "orchestration_input_tokens": 4,
            "orchestration_input_cached_tokens": 3,
            "future_input_detail": {"tokens": 5},
        },
        "output_tokens": 8,
        "output_tokens_details": {
            "reasoning_tokens": 3,
            "orchestration_output_tokens": 2,
            "future_output_detail": ["preserved"],
        },
        "total_tokens": 20,
        "future_usage_field": {"value": true},
    })
}

/// The usage cases: a normal response and the three terminal events, each
/// with the place its usage sits.
fn usage_cases(usage: &Value) -> Vec<(&'static str, Value)> {
    vec![
        ("normal response", json!({ "usage": usage })),
        (
            "completed event",
            json!({"type": "response.completed", "response": {"usage": usage}}),
        ),
        (
            "incomplete event",
            json!({"type": "response.incomplete", "response": {"usage": usage}}),
        ),
        (
            "failed event",
            json!({"type": "response.failed", "sequence_number": 1, "response": {"usage": usage}}),
        ),
    ]
}

/// `getUsage`: `value.usage` for a response, `value.response.usage` for an
/// event.
fn get_usage(value: &Value) -> &Value {
    if value.get("type").is_none() {
        &value["usage"]
    } else {
        &value["response"]["usage"]
    }
}

mod open_ai_responses_usage_schemas {
    use super::*;

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › OpenAI Responses usage schemas › preserves complete usage for $name
    #[test]
    fn preserves_complete_usage_for_name() {
        // Both the generate parser and the stream reducer read usage with
        // `parse_usage` and report the wire object through
        // `convert_responses_usage`.
        let usage = usage();
        for (name, value) in usage_cases(&usage) {
            let raw = get_usage(&value);
            let parsed = parse_usage(raw);
            assert!(parsed.is_some(), "{name}");
            let converted = convert_responses_usage(parsed.as_ref(), Some(raw.clone()));
            assert_eq!(json!(converted.raw), usage, "{name}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › OpenAI Responses usage schemas › rejects invalid total_tokens for $name
    #[test]
    fn rejects_invalid_total_tokens_for_name() {
        let mut invalid = usage();
        invalid["total_tokens"] = json!("20");
        for (name, value) in usage_cases(&invalid) {
            assert!(parse_usage(get_usage(&value)).is_none(), "{name}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › OpenAI Responses usage schemas › rejects invalid $name values
    #[test]
    fn rejects_invalid_name_values() {
        let usage = usage();
        let mut cases = Vec::new();
        let mut value = usage.clone();
        value["input_tokens"] = json!("12");
        cases.push(("input_tokens", value));
        let mut value = usage.clone();
        value["input_tokens_details"]["cached_tokens"] = json!("2");
        cases.push(("input_tokens_details.cached_tokens", value));
        let mut value = usage.clone();
        value["output_tokens"] = json!("8");
        cases.push(("output_tokens", value));
        let mut value = usage.clone();
        value["output_tokens_details"]["reasoning_tokens"] = json!("3");
        cases.push(("output_tokens_details.reasoning_tokens", value));
        for (name, value) in cases {
            let response = json!({ "usage": value });
            assert!(parse_usage(get_usage(&response)).is_none(), "{name}");
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-api.test.ts › OpenAI Responses usage schemas › accepts failed events with null usage
    #[tokio::test]
    async fn accepts_failed_events_with_null_usage() {
        assert_accepted(json!({
            "type": "response.failed",
            "sequence_number": 1,
            "response": {"usage": null},
        }))
        .await;
    }
}
