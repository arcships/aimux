//! The Anthropic computer toolset end to end, against `@ai-sdk/anthropic`
//! 4.0.71: member calls in a response become one call of the caller's
//! toolset tool with the member name as `action`, and go back to the API as
//! upstream sends them (`convert-to-anthropic-prompt.ts:663-679`, `:978-1004`;
//! `anthropic-language-model.ts:1347-1366`, `:2086-2110`, `:2604-2686`).

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart,
};
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{ProviderTool, Tool};
use aimux_core::types::Warning;
use aimux_providers::anthropic::{AnthropicProvider, AnthropicProviderSettings, create_anthropic};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

const TOOLSET_ID: &str = "anthropic.computer_toolset_20260801";

fn provider(fetch: &std::sync::Arc<MockFetch>) -> AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some("https://test.invalid/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn computer_tool(name: &str) -> Tool {
    Tool::Provider(ProviderTool {
        id: TOOLSET_ID.parse().unwrap(),
        name: name.to_string(),
        args: serde_json::Map::new(),
    })
}

fn response(content: Value) -> Value {
    json!({
        "id": "msg_1", "type": "message", "role": "assistant", "content": content,
        "model": "test-model", "stop_reason": "tool_use", "stop_sequence": null,
        "usage": {"input_tokens": 4, "output_tokens": 30}
    })
}

fn text_response() -> Canned {
    Canned::json(&json!({
        "id": "msg_2", "type": "message", "role": "assistant",
        "content": [{"type": "text", "text": "done"}],
        "model": "test-model", "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 4, "output_tokens": 1}
    }))
}

fn sse(events: &[Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}

fn anthropic(value: Value) -> SharedProviderOptions {
    provider_namespace("anthropic", value).unwrap()
}

/// A history with one toolset call and its result, then a user turn.
fn history(
    input: Value,
    call_options: Option<SharedProviderOptions>,
    tool_name: &str,
) -> Vec<LanguageModelMessage> {
    vec![
        LanguageModelMessage::user_text("Take a screenshot"),
        LanguageModelMessage::Assistant {
            content: vec![AssistantPart::ToolCall(ToolCallPart {
                tool_call_id: "toolu_1".into(),
                tool_name: tool_name.into(),
                input,
                provider_executed: None,
                provider_options: call_options.clone(),
            })],
            provider_options: None,
        },
        LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "toolu_1".into(),
                tool_name: tool_name.into(),
                output: ToolResultOutput::Text {
                    value: "ok".into(),
                    provider_options: None,
                },
                provider_options: call_options,
            })],
            provider_options: None,
        },
        LanguageModelMessage::user_text("Next"),
    ]
}

async fn sent_messages(
    prompt: Vec<LanguageModelMessage>,
    tools: Option<Vec<Tool>>,
) -> (Value, Vec<u8>, Vec<Warning>) {
    let fetch = MockFetch::new(vec![text_response()]);
    let options = CallOptions {
        max_output_tokens: Some(100),
        tools,
        ..CallOptions::new(prompt)
    };
    let result = provider(&fetch)
        .messages("test-model")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    (
        seen[0].json_body()["messages"].clone(),
        seen[0].body.clone(),
        result.warnings,
    )
}

fn is_missing_action_warning(warning: &Warning, tool_name: &str) -> bool {
    matches!(warning, Warning::Other { message }
        if message == &format!("toolset tool call for tool {tool_name} is missing the action"))
}

#[tokio::test]
async fn a_member_call_goes_back_with_its_action_in_the_input() {
    let (messages, body, warnings) = sent_messages(
        history(
            json!({"action": "left_click", "coordinate": [10, 20]}),
            None,
            "my_computer",
        ),
        Some(vec![computer_tool("my_computer")]),
    )
    .await;
    assert_eq!(
        messages[1]["content"][0],
        json!({
            "type": "tool_use", "id": "toolu_1", "name": "left_click",
            "toolset_name": "computer",
            "input": {"action": "left_click", "coordinate": [10, 20]}
        })
    );
    assert_eq!(
        messages[2]["content"][0],
        json!({"type": "tool_result", "tool_use_id": "toolu_1", "toolset_name": "computer", "content": "ok"})
    );
    // Key order on the sent bytes, as upstream builds the blocks.
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains(
        r#"{"type":"tool_use","id":"toolu_1","name":"left_click","toolset_name":"computer","input":{"action":"left_click","coordinate":[10,20]}}"#
    ), "{body}");
    assert!(body.contains(
        r#"{"type":"tool_result","tool_use_id":"toolu_1","toolset_name":"computer","content":"ok"}"#
    ), "{body}");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn a_member_call_without_an_action_is_sent_under_the_toolset_name() {
    let (messages, _, warnings) = sent_messages(
        history(json!({"coordinate": [1, 2]}), None, "my_computer"),
        Some(vec![computer_tool("my_computer")]),
    )
    .await;
    assert_eq!(
        messages[1]["content"][0],
        json!({
            "type": "tool_use", "id": "toolu_1", "name": "computer",
            "toolset_name": "computer", "input": {"coordinate": [1, 2]}
        })
    );
    // Its result still has the call it answers.
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(is_missing_action_warning(&warnings[0], "my_computer"));
}

#[tokio::test]
async fn a_non_string_action_counts_as_missing_and_a_non_object_input_is_wrapped() {
    let (messages, _, warnings) = sent_messages(
        history(json!({"action": 3}), None, "my_computer"),
        Some(vec![computer_tool("my_computer")]),
    )
    .await;
    assert_eq!(messages[1]["content"][0]["name"], "computer");
    assert_eq!(messages[1]["content"][0]["input"], json!({"action": 3}));
    assert!(is_missing_action_warning(&warnings[0], "my_computer"));

    let (messages, _, warnings) = sent_messages(
        history(json!("not an object"), None, "my_computer"),
        Some(vec![computer_tool("my_computer")]),
    )
    .await;
    assert_eq!(
        messages[1]["content"][0]["input"],
        json!({"rawInvalidInput": "not an object"})
    );
    assert_eq!(messages[1]["content"][0]["name"], "computer");
    assert!(is_missing_action_warning(&warnings[0], "my_computer"));
}

#[tokio::test]
async fn the_toolset_comes_from_the_anthropic_metadata_when_the_tool_is_not_passed() {
    let (messages, _, _) = sent_messages(
        history(
            json!({"action": "screenshot"}),
            Some(anthropic(json!({"toolsetName": "computer"}))),
            "my_computer",
        ),
        None,
    )
    .await;
    assert_eq!(messages[1]["content"][0]["name"], "screenshot");
    assert_eq!(messages[1]["content"][0]["toolset_name"], "computer");
    assert_eq!(
        messages[1]["content"][0]["input"],
        json!({"action": "screenshot"})
    );
    assert_eq!(messages[2]["content"][0]["toolset_name"], "computer");

    // Only `anthropic` is read, as `getAnthropicToolsetName` does; another
    // namespace or a non-string value leaves a plain tool call.
    for options in [
        provider_namespace("vertex", json!({"toolsetName": "computer"})).unwrap(),
        anthropic(json!({"toolsetName": 7})),
    ] {
        let (messages, _, _) = sent_messages(
            history(
                json!({"action": "screenshot"}),
                Some(options),
                "my_computer",
            ),
            None,
        )
        .await;
        assert_eq!(
            messages[1]["content"][0],
            json!({"type": "tool_use", "id": "toolu_1", "name": "my_computer", "input": {"action": "screenshot"}})
        );
        assert!(messages[2]["content"][0].get("toolset_name").is_none());
    }
}

#[tokio::test]
async fn a_generated_member_call_round_trips_as_upstream_sends_it() {
    let fetch = MockFetch::new(vec![
        Canned::json(&response(json!([{
            "type": "tool_use", "id": "toolu_1", "name": "left_click",
            "toolset_name": "computer",
            "input": {"coordinate": [10, 20]},
            "caller": {"type": "direct"}
        }]))),
        text_response(),
    ]);
    let tools = Some(vec![computer_tool("my_computer")]);
    let options = CallOptions {
        max_output_tokens: Some(100),
        tools: tools.clone(),
        ..CallOptions::new(vec![LanguageModelMessage::user_text("Click")])
    };
    let model = provider(&fetch).messages("test-model");
    let result = model.do_generate(&options).await.unwrap();
    let GenerateContent::ToolCall(call) = &result.content[0] else {
        panic!("expected a tool call: {:?}", result.content);
    };
    assert_eq!(call.tool_name, "my_computer");
    assert_eq!(
        call.input,
        r#"{"action":"left_click","coordinate":[10,20]}"#
    );
    let metadata = call.provider_metadata.clone().unwrap();
    assert_eq!(
        serde_json::to_string(&metadata).unwrap(),
        r#"{"anthropic":{"toolsetName":"computer","caller":{"type":"direct"}}}"#
    );

    // The next turn sends the call back with the metadata as its options.
    let next = CallOptions {
        max_output_tokens: Some(100),
        tools,
        ..CallOptions::new(history(
            serde_json::from_str(&call.input).unwrap(),
            Some(metadata),
            "my_computer",
        ))
    };
    model.do_generate(&next).await.unwrap();
    let sent = fetch.seen()[1].json_body();
    assert_eq!(
        sent["messages"][1]["content"][0],
        json!({
            "type": "tool_use", "id": "toolu_1", "name": "left_click",
            "toolset_name": "computer",
            "input": {"action": "left_click", "coordinate": [10, 20]},
            "caller": {"type": "direct"}
        })
    );
}

async fn stream_parts(events: &[Value]) -> Vec<StreamPart> {
    let fetch = MockFetch::new(vec![sse(events)]);
    let options = CallOptions {
        max_output_tokens: Some(100),
        tools: Some(vec![computer_tool("my_computer")]),
        ..CallOptions::new(vec![LanguageModelMessage::user_text("Click")])
    };
    provider(&fetch)
        .messages("test-model")
        .do_stream(&options)
        .await
        .unwrap()
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn member_call_events(start_input: Value, deltas: &[&str]) -> Vec<Value> {
    let mut events = vec![
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "content": [], "model": "test-model", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 4, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "left_click", "toolset_name": "computer", "input": start_input}}),
    ];
    events.extend(deltas.iter().map(|delta| {
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": delta}})
    }));
    events.extend([
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": null}, "usage": {"output_tokens": 30}}),
        json!({"type": "message_stop"}),
    ]);
    events
}

/// The tool-input parts of a streamed member call: (start name, deltas,
/// final input, metadata).
fn member_call(parts: &[StreamPart]) -> (String, Vec<String>, String, String) {
    let start = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::ToolInputStart { tool_name, .. } => Some(tool_name.clone()),
            _ => None,
        })
        .expect("a tool-input-start");
    let deltas = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolInputDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    let call = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .expect("a tool call");
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, StreamPart::ToolInputEnd { id, .. } if id == "toolu_1"))
    );
    assert_eq!(call.tool_name, "my_computer");
    (
        start,
        deltas,
        call.input,
        serde_json::to_string(&call.provider_metadata).unwrap(),
    )
}

#[tokio::test]
async fn a_streamed_member_call_is_one_delta_with_the_action_injected() {
    let parts = stream_parts(&member_call_events(
        json!({}),
        &[r#"{"coordinate":"#, r#"[10,20]}"#],
    ))
    .await;
    let (start, deltas, input, metadata) = member_call(&parts);
    assert_eq!(start, "my_computer");
    assert_eq!(deltas, [r#"{"action":"left_click","coordinate":[10,20]}"#]);
    assert_eq!(input, r#"{"action":"left_click","coordinate":[10,20]}"#);
    assert_eq!(metadata, r#"{"anthropic":{"toolsetName":"computer"}}"#);

    // Input given whole in the start block, and no input at all.
    let parts = stream_parts(&member_call_events(json!({"coordinate": [1, 2]}), &[])).await;
    assert_eq!(
        member_call(&parts).2,
        r#"{"action":"left_click","coordinate":[1,2]}"#
    );
    let parts = stream_parts(&member_call_events(json!({}), &[])).await;
    assert_eq!(member_call(&parts).2, r#"{"action":"left_click"}"#);
}

#[tokio::test]
async fn a_streamed_member_input_that_secure_json_parse_refuses_is_kept_raw() {
    for raw in [
        r#"{"coordinate":[1,2]"#,
        r#"{"__proto__":{"x":1}}"#,
        r#"{"a":{"constructor":{"prototype":{}}}}"#,
    ] {
        let parts = stream_parts(&member_call_events(json!({}), &[raw])).await;
        let (_, deltas, input, _) = member_call(&parts);
        assert_eq!(deltas, [raw]);
        assert_eq!(input, raw);
    }
}
