use futures::StreamExt;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};

use crate::anthropic::{AnthropicMessagesModel, AnthropicProviderSettings, create_anthropic};
use crate::test_support::mock_fetch::{Canned, MockFetch};

const SPLICED_MESSAGE_START: &str = include_str!("fixtures/spliced-message-start.chunks.txt");
const DUPLICATE_MESSAGE_START: &str = include_str!("fixtures/duplicate-message-start.chunks.txt");

/// `createModel(fixtureName)`: every line of the fixture is one SSE `data:` event.
fn create_model(chunks: &str) -> AnthropicMessagesModel {
    let body: String = chunks
        .trim()
        .split('\n')
        .map(|line| format!("data: {line}\n\n"))
        .collect();
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }]);
    create_anthropic(AnthropicProviderSettings {
        base_url: Some("https://api.anthropic.com/v1".into()),
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .messages("claude-3-haiku-20240307")
}

fn prompt() -> Vec<LanguageModelMessage> {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `convertReadableStreamToArray(stream)`; a stream error item counts as an
/// `error` part.
async fn collect(model: &AnthropicMessagesModel, options: &CallOptions) -> Vec<StreamPart> {
    model
        .do_stream(options)
        .await
        .unwrap()
        .stream
        .map(|item| item.unwrap_or_else(|error| StreamPart::Error { error }))
        .collect()
        .await
}

mod anthropic_message_lifecycle {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-message-lifecycle.test.ts › Anthropic message lifecycle › fails when a different message starts while the previous message is open
    #[tokio::test]
    async fn fails_when_a_different_message_starts_while_the_previous_message_is_open() {
        let mut options = CallOptions::new(prompt());
        options.tools = Some(vec![Tool::Function(FunctionTool::new(
            "test-tool",
            json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"],
                "additionalProperties": false,
            }),
        ))]);
        let parts = collect(&create_model(SPLICED_MESSAGE_START), &options).await;

        let error_parts: Vec<&AiMuxError> = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::Error { error } => Some(error),
                _ => None,
            })
            .collect();
        let response_ids: Vec<Option<String>> = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::ResponseMetadata(metadata) => Some(metadata.id.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(error_parts.len(), 1, "{parts:?}");
        assert!(
            matches!(error_parts[0], AiMuxError::InvalidResponseData { .. }),
            "{:?}",
            error_parts[0]
        );
        assert_eq!(response_ids, vec![Some("msg_first".to_string())]);
        assert!(!parts.iter().any(
            |part| matches!(part, StreamPart::ToolCall(call) if call.tool_call_id == "toolu_second")
        ));
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, StreamPart::Finish { .. }))
        );
    }

    /// upstream: @ai-sdk/anthropic src/anthropic-message-lifecycle.test.ts › Anthropic message lifecycle › ignores a duplicate message_start for the already-open message
    #[tokio::test]
    async fn ignores_a_duplicate_message_start_for_the_already_open_message() {
        let parts = collect(
            &create_model(DUPLICATE_MESSAGE_START),
            &CallOptions::new(prompt()),
        )
        .await;
        let response_metadata = parts
            .iter()
            .filter(|part| matches!(part, StreamPart::ResponseMetadata(_)))
            .count();

        assert_eq!(response_metadata, 1);
        assert!(
            !parts
                .iter()
                .any(|part| matches!(part, StreamPart::Error { .. }))
        );
        assert!(
            parts
                .iter()
                .any(|part| matches!(part, StreamPart::Finish { .. }))
        );
    }
}

/// Strict stream mode (`CallOptions.strict_stream`, #403) on the Messages
/// stream: the AI SDK accepts each of these, and so does aimux with the
/// option off.
mod strict_stream {
    use super::*;

    const MESSAGE_START: &str = r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-3-haiku-20240307","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}}"#;
    const TEXT_START: &str =
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
    const TEXT_DELTA: &str =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
    const TEXT_STOP: &str = r#"{"type":"content_block_stop","index":0}"#;
    const MESSAGE_DELTA: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#;
    const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;

    fn options(strict: bool) -> CallOptions {
        CallOptions {
            strict_stream: Some(strict),
            ..CallOptions::new(prompt())
        }
    }

    async fn parts(events: &[&str], strict: bool) -> Vec<StreamPart> {
        collect(&create_model(&events.join("\n")), &options(strict)).await
    }

    fn strict_failure(parts: &[StreamPart]) -> Option<String> {
        match parts.last() {
            Some(StreamPart::Error {
                error: AiMuxError::InvalidResponseData { message, .. },
            }) if message.starts_with("Strict stream: ") => Some(message.clone()),
            _ => None,
        }
    }

    fn has_finish(parts: &[StreamPart]) -> bool {
        parts
            .iter()
            .any(|part| matches!(part, StreamPart::Finish { .. }))
    }

    #[tokio::test]
    async fn a_complete_message_finishes() {
        let events = [
            MESSAGE_START,
            TEXT_START,
            TEXT_DELTA,
            TEXT_STOP,
            MESSAGE_DELTA,
            MESSAGE_STOP,
        ];
        let parts = parts(&events, true).await;
        assert_eq!(strict_failure(&parts), None, "{parts:?}");
        assert!(has_finish(&parts), "{parts:?}");
    }

    /// Without `message_stop` the lenient stream ends silently, with no
    /// `Finish`; the strict stream fails.
    #[tokio::test]
    async fn a_message_without_message_stop_fails() {
        let events = [
            MESSAGE_START,
            TEXT_START,
            TEXT_DELTA,
            TEXT_STOP,
            MESSAGE_DELTA,
        ];
        let lenient = parts(&events, false).await;
        assert_eq!(strict_failure(&lenient), None, "{lenient:?}");
        assert!(!has_finish(&lenient), "{lenient:?}");

        let strict = parts(&events, true).await;
        assert_eq!(
            strict_failure(&strict).as_deref(),
            Some("Strict stream: the stream ended without a finish part"),
            "{strict:?}"
        );
    }

    /// A `content_block_delta` for an index whose block never started.
    #[tokio::test]
    async fn a_delta_outside_a_started_block_fails() {
        let orphan =
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"x"}}"#;
        let events = [
            MESSAGE_START,
            TEXT_START,
            TEXT_DELTA,
            orphan,
            TEXT_STOP,
            MESSAGE_DELTA,
            MESSAGE_STOP,
        ];
        let lenient = parts(&events, false).await;
        assert_eq!(strict_failure(&lenient), None, "{lenient:?}");
        assert!(has_finish(&lenient), "{lenient:?}");

        let strict = parts(&events, true).await;
        let failure = strict_failure(&strict).expect("the strict stream fails");
        assert!(failure.contains("that was not started"), "{failure}");
        assert!(!has_finish(&strict), "{strict:?}");
    }

    /// A second `content_block_start` for an index that is open.
    #[tokio::test]
    async fn a_block_started_twice_fails() {
        let events = [
            MESSAGE_START,
            TEXT_START,
            TEXT_START,
            TEXT_DELTA,
            TEXT_STOP,
            MESSAGE_DELTA,
            MESSAGE_STOP,
        ];
        let lenient = parts(&events, false).await;
        assert_eq!(strict_failure(&lenient), None, "{lenient:?}");

        let strict = parts(&events, true).await;
        let failure = strict_failure(&strict).expect("the strict stream fails");
        assert!(failure.contains("started twice"), "{failure}");
        assert!(!has_finish(&strict), "{strict:?}");
    }
}
