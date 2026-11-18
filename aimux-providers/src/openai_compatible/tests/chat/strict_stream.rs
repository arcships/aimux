//! The OpenAI-compatible chat checks of strict stream mode
//! (`CallOptions.strict_stream`, #473). Without the option these chunks are
//! repaired the way the AI SDK repairs them.

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::stream_part::StreamPart;

use crate::test_support::mock_fetch::MockFetch;

use super::super::support::{options, provider, stream_response};

/// The parts of the stream for `chunks`, as part types; a text delta is
/// `text <delta>`, a tool call `tool-call <id>` and a failure
/// `failed: <message>`.
async fn run(chunks: &[Value], strict: bool) -> Vec<String> {
    let fetch = MockFetch::new(vec![stream_response(chunks)]);
    let mut options = options();
    options.strict_stream = Some(strict);
    provider(&fetch)
        .chat("test-model")
        .do_stream(&options)
        .await
        .expect("stream")
        .stream
        .filter_map(|item| async move {
            match item {
                Ok(StreamPart::TextDelta { delta, .. }) => Some(format!("text {delta}")),
                Ok(StreamPart::ToolCall(call)) => Some(format!("tool-call {}", call.tool_call_id)),
                Ok(StreamPart::Finish { finish_reason, .. }) => {
                    Some(format!("finish {:?}", finish_reason.unified))
                }
                Ok(StreamPart::StreamStart { .. }) => None,
                Ok(part) => Some(serde_json::to_value(&part).unwrap()["type"].to_string()),
                Err(AiMuxError::Other { message, .. }) => Some(format!("failed: {message}")),
                Err(other) => Some(format!("other error: {other:?}")),
            }
        })
        .collect()
        .await
}

fn tool_delta(index: u64, id: Option<&str>, name: Option<&str>) -> Value {
    let mut call = json!({"index": index, "type": "function", "function": {"arguments": "{}"}});
    if let Some(id) = id {
        call["id"] = json!(id);
    }
    if let Some(name) = name {
        call["function"]["name"] = json!(name);
    }
    json!({"choices":[{"index":0,"delta":{"tool_calls":[call]},"finish_reason":null}]})
}

fn finish(reason: &str) -> Value {
    json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]})
}

fn last(trace: &[String]) -> &str {
    trace.last().map(String::as_str).unwrap_or_default()
}

#[tokio::test]
async fn only_candidate_zero_is_read() {
    let chunks = [
        json!({"choices":[
            {"index":1,"delta":{"content":"wrong","tool_calls":[{"index":0,"id":"other","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":"tool_calls"},
            {"index":0,"delta":{"content":"selected"},"finish_reason":null}
        ]}),
        json!({"choices":[{"index":1,"delta":{"content":"also wrong"},"finish_reason":"length"}]}),
        finish("stop"),
    ];
    assert_eq!(
        run(&chunks, true).await,
        [
            "\"text-start\"",
            "text selected",
            "\"text-end\"",
            "finish Stop"
        ]
    );
    // Off: the first choice, as in the AI SDK.
    assert!(
        run(&chunks, false)
            .await
            .contains(&"text wrong".to_string())
    );
}

#[tokio::test]
async fn a_tool_call_has_one_identity() {
    let cases = [
        (
            vec![tool_delta(0, Some(""), Some("lookup"))],
            "tool call at index 0 started without an id and a name",
        ),
        (
            vec![tool_delta(0, Some("call-1"), Some(""))],
            "tool call at index 0 started without an id and a name",
        ),
        (
            vec![
                tool_delta(0, Some("call-1"), Some("lookup")),
                tool_delta(1, Some("call-1"), Some("lookup")),
            ],
            "tool call id `call-1` repeated at index 1",
        ),
        (
            vec![
                tool_delta(0, Some("call-1"), Some("lookup")),
                tool_delta(0, Some("changed"), Some("lookup")),
            ],
            "tool call `call-1` at index 0 changed its id or name",
        ),
        (
            vec![
                tool_delta(0, Some("call-1"), Some("lookup")),
                tool_delta(0, Some("call-1"), Some("renamed")),
            ],
            "tool call `call-1` at index 0 changed its id or name",
        ),
    ];
    for (mut chunks, problem) in cases {
        chunks.push(finish("tool_calls"));
        let trace = run(&chunks, true).await;
        assert_eq!(last(&trace), format!("failed: Strict stream: {problem}"));
        assert!(
            !run(&chunks, false)
                .await
                .iter()
                .any(|p| p.starts_with("failed"))
        );
    }
}

#[tokio::test]
async fn the_finish_reason_agrees_with_the_tool_calls() {
    let text = json!({"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]});
    let call = tool_delta(0, Some("call-1"), Some("lookup"));
    for (chunks, problem) in [
        (
            [text, finish("tool_calls")],
            "finish_reason `tool_calls` without a tool call",
        ),
        (
            [call, finish("stop")],
            "finish_reason `stop` with a tool call",
        ),
    ] {
        assert_eq!(
            last(&run(&chunks, true).await),
            format!("failed: Strict stream: {problem}")
        );
    }
}

#[tokio::test]
async fn a_failed_finish_is_not_replaced_by_tool_calls() {
    let chunks = [
        tool_delta(0, Some("call-1"), Some("lookup")),
        finish("length"),
        finish("tool_calls"),
    ];
    assert_eq!(
        last(&run(&chunks, true).await),
        "failed: Strict stream: finish_reason `tool_calls` after `length`"
    );
    assert_eq!(last(&run(&chunks, false).await), "finish ToolCalls");
}

#[tokio::test]
async fn a_delta_after_the_finish_reason_fails() {
    let late = [
        json!({"content":"late"}),
        json!({"refusal":"late refusal"}),
        json!({"reasoning_content":"late thought"}),
        json!({"tool_calls":[{"index":0,"id":"call-late","type":"function","function":{"name":"lookup","arguments":"{}"}}]}),
    ];
    for delta in late {
        let chunks = [
            json!({"choices":[{"index":0,"delta":{"content":"finished"},"finish_reason":"stop"}]}),
            json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
        ];
        assert_eq!(
            run(&chunks, true).await,
            [
                "\"text-start\"",
                "text finished",
                "failed: Strict stream: a delta after finish_reason `stop`"
            ]
        );
    }
}
