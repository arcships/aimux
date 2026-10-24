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
            matches!(error_parts[0], AiMuxError::InvalidResponseData(_)),
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
