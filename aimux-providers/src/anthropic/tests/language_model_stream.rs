//! The Messages stream driven event by event (previously the inline
//! `stream_tests` of `stream.rs`).

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;

use crate::anthropic::api::StreamEvent;
use crate::anthropic::language_model::AnthropicMessagesStream;
use crate::anthropic::language_model_options::CANONICAL;
use crate::stream::testing::run;

fn stream() -> AnthropicMessagesStream {
    AnthropicMessagesStream::with_options_name(CANONICAL)
}

fn event(event: Value) -> (Option<Value>, Result<StreamEvent, AiMuxError>) {
    (
        Some(event.clone()),
        serde_json::from_value(event).map_err(AiMuxError::from),
    )
}

fn message_start() -> (Option<Value>, Result<StreamEvent, AiMuxError>) {
    event(json!({
        "type": "message_start",
        "message": {
            "id": "msg_1", "model": "claude-x",
            "usage": { "input_tokens": 3, "output_tokens": 1 }
        }
    }))
}

fn text_block(index: usize, text: &str) -> [(Option<Value>, Result<StreamEvent, AiMuxError>); 3] {
    [
        event(json!({
            "type": "content_block_start", "index": index,
            "content_block": { "type": "text", "text": "" }
        })),
        event(json!({
            "type": "content_block_delta", "index": index,
            "delta": { "type": "text_delta", "text": text }
        })),
        event(json!({ "type": "content_block_stop", "index": index })),
    ]
}

/// partial: `should stream tool deltas`; full port in src/anthropic/tests/language_model.rs.
#[test]
fn interleaved_text_and_tool_blocks_then_message_stop_finish_the_stream() {
    let mut input = vec![message_start()];
    input.push(event(json!({
        "type": "content_block_start", "index": 1,
        "content_block": { "type": "tool_use", "id": "toolu_1", "name": "weather", "input": {} }
    })));
    input.extend(text_block(0, "Checking"));
    input.extend([
        event(json!({
            "type": "content_block_delta", "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "{\"city\":" }
        })),
        event(json!({
            "type": "content_block_delta", "index": 1,
            "delta": { "type": "input_json_delta", "partial_json": "\"Paris\"}" }
        })),
        event(json!({ "type": "content_block_stop", "index": 1 })),
        event(json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 9 }
        })),
        event(json!({ "type": "message_stop" })),
        // Still read: a later message may follow `message_stop`.
        event(json!({
            "type": "content_block_start", "index": 2,
            "content_block": { "type": "text", "text": "" }
        })),
    ]);
    assert_eq!(
        run(stream(), input),
        [
            "stream-start",
            "response-metadata msg_1",
            "tool-input-start toolu_1 weather",
            "text-start 0",
            "text-delta 0 Checking",
            "text-end 0",
            "tool-input-delta toolu_1 {\"city\":",
            "tool-input-delta toolu_1 \"Paris\"}",
            "tool-input-end toolu_1",
            "tool-call toolu_1 weather {\"city\":\"Paris\"}",
            "finish ToolCalls",
            "text-start 2",
        ]
    );
}

/// partial: `should forward error chunks`; full port in src/anthropic/tests/language_model.rs.
#[test]
fn an_error_event_is_an_error_part_and_the_stream_goes_on() {
    let mut input = vec![message_start()];
    input.push(event(json!({
        "type": "error",
        "error": { "type": "overloaded_error", "message": "Overloaded" }
    })));
    input.extend(text_block(0, "late"));
    input.push(event(json!({ "type": "message_stop" })));
    assert_eq!(
        run(stream(), input),
        [
            "stream-start",
            "response-metadata msg_1",
            "error API call error: HTTP 529: Overloaded",
            "text-start 0",
            "text-delta 0 late",
            "text-end 0",
            "finish Other",
        ]
    );
}

#[test]
fn flush_without_message_stop_emits_no_finish() {
    let mut input = vec![message_start()];
    input.extend(text_block(0, "cut"));
    assert_eq!(
        run(stream(), input),
        [
            "stream-start",
            "response-metadata msg_1",
            "text-start 0",
            "text-delta 0 cut",
            "text-end 0",
        ]
    );
}
