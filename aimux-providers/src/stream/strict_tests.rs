//! The protocol-independent checks of strict stream mode
//! (`CallOptions.strict_stream`, #403) on synthetic part sequences.

use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;
use aimux_core::result::StreamResult;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};

use super::strict_guard;

fn options(strict: bool) -> CallOptions {
    CallOptions {
        strict_stream: Some(strict),
        ..CallOptions::new(vec![])
    }
}

fn text_start(id: &str) -> StreamPart {
    StreamPart::TextStart {
        id: id.into(),
        provider_metadata: None,
    }
}

fn text_delta(id: &str, delta: &str) -> StreamPart {
    StreamPart::TextDelta {
        id: id.into(),
        delta: delta.into(),
        provider_metadata: None,
    }
}

fn text_end(id: &str) -> StreamPart {
    StreamPart::TextEnd {
        id: id.into(),
        provider_metadata: None,
    }
}

fn tool_call(id: &str, provider_executed: Option<bool>) -> StreamPart {
    StreamPart::ToolCall(RawToolCall {
        tool_call_id: id.into(),
        tool_name: "weather".into(),
        input: "{}".into(),
        provider_executed,
        dynamic: None,
        provider_metadata: None,
    })
}

fn finish(unified: FinishReasonUnified) -> StreamPart {
    StreamPart::Finish {
        finish_reason: FinishReason { unified, raw: None },
        usage: Usage::default(),
        provider_metadata: None,
    }
}

/// The parts of `parts` run through the guard, as part type names; a
/// failure is `"failed: <message>"`.
async fn run(parts: Vec<StreamPart>, strict: bool) -> Vec<String> {
    let result = StreamResult {
        stream: Box::pin(futures::stream::iter(parts.into_iter().map(Ok))),
        request: None,
        response: None,
    };
    strict_guard(result, &options(strict))
        .stream
        .map(|item| match item {
            Ok(StreamPart::ToolCall(call)) => format!("tool-call {}", call.tool_call_id),
            Ok(part) => serde_json::to_value(&part).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned(),
            Err(AiMuxError::InvalidResponseData { message, .. }) => format!("failed: {message}"),
            Err(other) => format!("other error: {other:?}"),
        })
        .collect()
        .await
}

#[tokio::test]
async fn a_complete_stream_passes_through_unchanged() {
    let parts = vec![
        text_start("0"),
        text_delta("0", "hi"),
        text_end("0"),
        finish(FinishReasonUnified::Stop),
    ];
    let expected = ["text-start", "text-delta", "text-end", "finish"];
    assert_eq!(run(parts.clone(), true).await, expected);
    assert_eq!(run(parts, false).await, expected);
}

#[tokio::test]
async fn a_delta_outside_a_started_block_fails() {
    let parts = vec![text_delta("0", "hi"), finish(FinishReasonUnified::Stop)];
    assert_eq!(
        run(parts.clone(), true).await,
        ["failed: Strict stream: text-delta for text block `0` that was not started"]
    );
    // Off: the AI SDK's lenient behavior.
    assert_eq!(run(parts, false).await, ["text-delta", "finish"]);
}

#[tokio::test]
async fn a_block_started_twice_fails() {
    let parts = vec![
        text_start("0"),
        text_start("0"),
        finish(FinishReasonUnified::Stop),
    ];
    assert_eq!(
        run(parts, true).await,
        [
            "text-start",
            "failed: Strict stream: text block `0` started twice"
        ]
    );
}

#[tokio::test]
async fn a_block_may_start_again_after_it_ended() {
    let parts = vec![
        text_start("0"),
        text_end("0"),
        text_start("0"),
        text_end("0"),
        finish(FinishReasonUnified::Stop),
    ];
    assert_eq!(
        run(parts, true).await,
        ["text-start", "text-end", "text-start", "text-end", "finish"]
    );
}

#[tokio::test]
async fn a_stream_without_finish_fails() {
    let parts = vec![text_start("0"), text_delta("0", "hi")];
    assert_eq!(
        run(parts, true).await,
        [
            "text-start",
            "text-delta",
            "failed: Strict stream: the stream ended without a finish part"
        ]
    );
}

#[tokio::test]
async fn a_client_tool_call_waits_for_finish() {
    let parts = vec![
        tool_call("call-1", None),
        text_start("0"),
        text_end("0"),
        finish(FinishReasonUnified::ToolCalls),
    ];
    assert_eq!(
        run(parts.clone(), true).await,
        ["text-start", "text-end", "tool-call call-1", "finish"]
    );
    assert_eq!(
        run(parts, false).await,
        ["tool-call call-1", "text-start", "text-end", "finish"]
    );
}

#[tokio::test]
async fn a_tool_call_is_not_delivered_when_the_stream_fails_or_errors() {
    assert_eq!(
        run(vec![tool_call("call-1", None)], true).await,
        ["failed: Strict stream: the stream ended without a finish part"]
    );
    assert_eq!(
        run(
            vec![
                tool_call("call-1", None),
                finish(FinishReasonUnified::Error)
            ],
            true
        )
        .await,
        ["finish"]
    );
}

#[tokio::test]
async fn a_provider_executed_tool_call_is_not_held() {
    let parts = vec![
        tool_call("ws-1", Some(true)),
        finish(FinishReasonUnified::Stop),
    ];
    assert_eq!(run(parts, true).await, ["tool-call ws-1", "finish"]);
}

#[tokio::test]
async fn a_stream_that_reported_its_error_is_not_failed_again() {
    let error = StreamPart::Error {
        error: AiMuxError::invalid_response_data("the body could not be decoded".to_string()),
    };
    assert_eq!(
        run(vec![tool_call("call-1", None), error], true).await,
        ["error"]
    );
}
