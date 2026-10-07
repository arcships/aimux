//! Integration checks for aimux's strict UTF-8 decoding (unlike upstream's
//! lossy decoder): a terminal byte error must stay terminal through the
//! response handler and provider, without a successful finish or tool flush.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_providers::{OpenAIConfig, OpenAIProvider};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn corrupted_stream(prefix: Value) -> Vec<Result<StreamPart, AiMuxError>> {
    let server = MockServer::start().await;
    let suffix = json!({
        "choices": [{"delta": {"content": "world"}, "finish_reason": "stop"}]
    });
    let mut body = format!("data: {prefix}\n\n").into_bytes();
    body.extend_from_slice(b"data: \xff\n\n");
    body.extend_from_slice(format!("data: {suffix}\n\ndata: [DONE]\n\n").as_bytes());
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_bytes(body),
        )
        .mount(&server)
        .await;
    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    provider
        .model("gpt-4o")
        .do_stream(&CallOptions::new(vec![]))
        .await
        .expect("the first event is valid")
        .stream
        .collect()
        .await
}

fn assert_terminal_decode_error(parts: &[Result<StreamPart, AiMuxError>]) {
    let errors: Vec<_> = parts
        .iter()
        .filter_map(|part| part.as_ref().err())
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(matches!(errors[0], AiMuxError::ApiCall(_)));
    assert!(!errors[0].is_recoverable_stream_error());
    assert!(!errors[0].is_retryable());
    assert!(parts.last().is_some_and(Result::is_err));
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part, Ok(StreamPart::Finish { .. })))
    );
}

#[tokio::test]
async fn invalid_utf8_after_text_does_not_emit_a_successful_finish() {
    let parts = corrupted_stream(json!({
        "choices": [{"delta": {"content": "hello "}}]
    }))
    .await;
    let text: String = parts
        .iter()
        .filter_map(|part| match part {
            Ok(StreamPart::TextDelta { delta, .. }) => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello ");
    assert_terminal_decode_error(&parts);
}

#[tokio::test]
async fn invalid_utf8_does_not_finalize_an_incomplete_tool_call() {
    let parts = corrupted_stream(json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call-1",
            "type": "function",
            "function": {"name": "lookup", "arguments": "{"}
        }]}}]
    }))
    .await;
    assert!(
        parts
            .iter()
            .any(|part| matches!(part, Ok(StreamPart::ToolInputStart { .. })))
    );
    assert!(!parts.iter().any(|part| matches!(
        part,
        Ok(StreamPart::ToolCall { .. } | StreamPart::ToolInputEnd { .. })
    )));
    assert_terminal_decode_error(&parts);
}
