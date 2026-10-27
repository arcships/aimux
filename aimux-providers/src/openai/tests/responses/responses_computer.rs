use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::openai::responses::build_responses_request_body;
use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        base_url: Some("https://api.openai.com/v1".into()),
        api_key: Some("APIKEY".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn computer_call() -> Value {
    json!({
        "type": "computer_call",
        "id": "computer_item_123",
        "call_id": "computer_call_123",
        "status": "completed",
        "pending_safety_checks": [
            {"id": "safety_123", "code": "confirm_action", "message": "Confirm this action."},
        ],
        "actions": [
            {"type": "click", "button": "left", "x": 100, "y": 200, "keys": ["CTRL"]},
            {"type": "double_click", "x": 110, "y": 210, "keys": null},
            {"type": "drag", "path": [{"x": 10, "y": 20}, {"x": 30, "y": 40}]},
            {"type": "keypress", "keys": ["CTRL", "L"]},
            {"type": "move", "x": 120, "y": 220},
            {"type": "screenshot"},
            {"type": "scroll", "x": 130, "y": 230, "scroll_x": 0, "scroll_y": 500},
            {"type": "type", "text": "hello"},
            {"type": "wait"},
        ],
    })
}

fn expected_input() -> Value {
    json!({
        "actions": [
            {"type": "click", "button": "left", "x": 100, "y": 200, "keys": ["CTRL"]},
            {"type": "double_click", "x": 110, "y": 210},
            {"type": "drag", "path": [{"x": 10, "y": 20}, {"x": 30, "y": 40}]},
            {"type": "keypress", "keys": ["CTRL", "L"]},
            {"type": "move", "x": 120, "y": 220},
            {"type": "screenshot"},
            {"type": "scroll", "x": 130, "y": 230, "scrollX": 0, "scrollY": 500},
            {"type": "type", "text": "hello"},
            {"type": "wait"},
        ],
        "pendingSafetyChecks": [
            {"id": "safety_123", "code": "confirm_action", "message": "Confirm this action."},
        ],
        "status": "completed",
    })
}

fn call_options() -> CallOptions {
    serde_json::from_value(json!({
        "prompt": [{"role": "user", "content": [{"type": "text", "text": "Use it."}]}],
        "tools": [{"type": "provider", "id": "openai.computer", "name": "computer", "args": {}}],
    }))
    .unwrap()
}

/// The `input` of `convertToOpenAIResponsesInput` with a computer tool in
/// the request, read from the request body the Responses model builds.
fn convert_input(prompt: Value, store: bool, has_previous_response_id: bool) -> Value {
    let mut openai = json!({ "store": store });
    if has_previous_response_id {
        openai["previousResponseId"] = json!("resp_previous");
    }
    let options: CallOptions = serde_json::from_value(json!({
        "prompt": prompt,
        "tools": [{"type": "provider", "id": "openai.computer", "name": "computer", "args": {}}],
        "providerOptions": {"openai": openai},
    }))
    .unwrap();
    build_responses_request_body("gpt-5.4", &options, false)
        .unwrap()
        .body["input"]
        .clone()
}

mod open_ai_responses_computer_tool {
    use super::*;

    /// upstream: @ai-sdk/openai src/responses/openai-responses-computer.test.ts › OpenAI Responses computer tool › decodes batched actions in non-streaming responses
    #[tokio::test]
    async fn decodes_batched_actions_in_non_streaming_responses() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "id": "resp_computer_test",
            "object": "response",
            "created_at": 1741630255,
            "status": "completed",
            "error": null,
            "incomplete_details": null,
            "instructions": null,
            "max_output_tokens": null,
            "model": "gpt-5.4",
            "output": [computer_call()],
            "usage": {"input_tokens": 100, "output_tokens": 50},
        }))]);

        let result = provider(&fetch)
            .responses("gpt-5.4")
            .do_generate(&call_options())
            .await
            .unwrap();

        let tools = fetch.seen()[0].json_body()["tools"].clone();
        assert_eq!(tools, json!([{"type": "computer"}]));
        assert_eq!(
            serde_json::to_value(&result.content).unwrap(),
            json!([{
                "type": "tool-call",
                "toolCallId": "computer_call_123",
                "toolName": "computer",
                "input": expected_input().to_string(),
                "providerMetadata": {"openai": {"itemId": "computer_item_123"}},
            }])
        );
        assert_eq!(
            serde_json::to_value(result.finish_reason.unified).unwrap(),
            json!("tool-calls")
        );
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-computer.test.ts › OpenAI Responses computer tool › decodes batched actions in streaming responses
    #[tokio::test]
    async fn decodes_batched_actions_in_streaming_responses() {
        let chunks = [
            json!({
                "type": "response.created",
                "response": {
                    "id": "resp_computer_test",
                    "created_at": 1741630255,
                    "model": "gpt-5.4",
                    "service_tier": null,
                },
            }),
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "computer_call",
                    "id": "computer_item_123",
                    "call_id": "computer_call_123",
                    "status": "in_progress",
                },
            }),
            json!({"type": "response.output_item.done", "output_index": 0, "item": computer_call()}),
            json!({
                "type": "response.completed",
                "response": {
                    "incomplete_details": null,
                    "reasoning": null,
                    "service_tier": null,
                    "usage": {
                        "input_tokens": 100,
                        "input_tokens_details": {"cached_tokens": 0},
                        "output_tokens": 50,
                        "output_tokens_details": {"reasoning_tokens": 0},
                    },
                },
            }),
        ];
        let fetch = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: chunks
                .iter()
                .map(|chunk| format!("data:{chunk}\n\n"))
                .collect::<String>()
                .into_bytes(),
        }]);

        let result = provider(&fetch)
            .responses("gpt-5.4")
            .do_stream(&call_options())
            .await
            .unwrap();
        let events: Vec<Value> = result
            .stream
            .map(|part| serde_json::to_value(part.unwrap()).unwrap())
            .collect()
            .await;

        for expected in [
            json!({"type": "tool-input-start", "id": "computer_call_123", "toolName": "computer"}),
            json!({
                "type": "tool-input-delta",
                "id": "computer_call_123",
                "delta": expected_input().to_string(),
            }),
            json!({
                "type": "tool-call",
                "toolCallId": "computer_call_123",
                "toolName": "computer",
                "input": expected_input().to_string(),
                "providerMetadata": {"openai": {"itemId": "computer_item_123"}},
            }),
        ] {
            assert!(
                events.contains(&expected),
                "missing {expected}\nin {events:#?}"
            );
        }
        let last = events.last().unwrap();
        assert_eq!(last["type"], "finish");
        assert_eq!(last["finishReason"]["unified"], "tool-calls");
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-computer.test.ts › OpenAI Responses computer tool › serializes screenshot output (store: $store, previous response: $hasPreviousResponseId)
    #[test]
    fn serializes_screenshot_output() {
        let prompt = json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "tool-call",
                    "toolCallId": "computer_call_123",
                    "toolName": "computer",
                    "input": {
                        "actions": [{"type": "scroll", "x": 10, "y": 20, "scrollX": 0, "scrollY": 100}],
                        "pendingSafetyChecks": [{"id": "safety_123", "code": "confirm_action"}],
                        "status": "completed",
                    },
                    "providerOptions": {"openai": {"itemId": "computer_item_123"}},
                }],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "computer_call_123",
                    "toolName": "computer",
                    "output": {
                        "type": "json",
                        "value": {
                            "output": {
                                "type": "computer_screenshot",
                                "imageUrl": "data:image/png;base64,c2NyZWVuc2hvdA==",
                                "detail": "original",
                            },
                            "acknowledgedSafetyChecks": [{"id": "safety_123", "code": "confirm_action"}],
                        },
                    },
                }],
            },
        ]);
        let output = json!({
            "type": "computer_call_output",
            "call_id": "computer_call_123",
            "output": {
                "type": "computer_screenshot",
                "image_url": "data:image/png;base64,c2NyZWVuc2hvdA==",
                "detail": "original",
            },
            "acknowledged_safety_checks": [{"id": "safety_123", "code": "confirm_action"}],
        });
        for (store, has_previous_response_id, expected_call) in [
            (
                true,
                false,
                Some(json!({"type": "item_reference", "id": "computer_item_123"})),
            ),
            (
                false,
                false,
                Some(json!({
                    "type": "computer_call",
                    "id": "computer_item_123",
                    "call_id": "computer_call_123",
                    "status": "completed",
                    "actions": [{"type": "scroll", "x": 10, "y": 20, "scroll_x": 0, "scroll_y": 100}],
                    "pending_safety_checks": [{"id": "safety_123", "code": "confirm_action"}],
                })),
            ),
            (true, true, None),
        ] {
            let expected: Vec<Value> = expected_call
                .into_iter()
                .chain(std::iter::once(output.clone()))
                .collect();
            assert_eq!(
                convert_input(prompt.clone(), store, has_previous_response_id),
                json!(expected),
                "store: {store}, previous response: {has_previous_response_id}"
            );
        }
    }

    /// upstream: @ai-sdk/openai src/responses/openai-responses-computer.test.ts › OpenAI Responses computer tool › serializes file ID screenshot output
    #[test]
    fn serializes_file_id_screenshot_output() {
        let input = convert_input(
            json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "computer_call_123",
                    "toolName": "computer",
                    "output": {
                        "type": "json",
                        "value": {"output": {"type": "computer_screenshot", "fileId": "file_screenshot_123"}},
                    },
                }],
            }]),
            true,
            false,
        );

        assert_eq!(
            input,
            json!([{
                "type": "computer_call_output",
                "call_id": "computer_call_123",
                "output": {"type": "computer_screenshot", "file_id": "file_screenshot_123"},
            }])
        );
    }
}
