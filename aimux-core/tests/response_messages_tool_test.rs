use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::{MessageContent, Role};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use serde_json::json;

struct ProviderTranscriptModel;

fn finish_reason() -> FinishReason {
    FinishReason {
        unified: FinishReasonUnified::Stop,
        raw: Some("end_turn".to_string()),
    }
}

#[async_trait]
impl LanguageModel for ProviderTranscriptModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "provider-transcript"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![
                GenerateContent::Text {
                    text: "before".to_string(),
                    provider_metadata: None,
                },
                GenerateContent::ToolCall {
                    tool_call_id: "srv-1".to_string(),
                    tool_name: "server_search".to_string(),
                    input: r#"{"query":"Rust"}"#.to_string(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    thought_signature: None,
                    provider_metadata: Some(json!({
                        "mock": { "serverCallId": "wire-1" }
                    })),
                },
                GenerateContent::ToolResult {
                    tool_call_id: "srv-1".to_string(),
                    tool_name: "server_search".to_string(),
                    result: json!({ "answer": "still running" }),
                    is_error: Some(false),
                    preliminary: Some(true),
                    dynamic: Some(true),
                    provider_metadata: Some(json!({
                        "mock": { "serverResultId": "wire-preliminary-1" }
                    })),
                },
                GenerateContent::ToolResult {
                    tool_call_id: "srv-1".to_string(),
                    tool_name: "server_search".to_string(),
                    result: json!({ "answer": 42 }),
                    is_error: Some(false),
                    preliminary: Some(false),
                    dynamic: Some(true),
                    provider_metadata: Some(json!({
                        "mock": { "serverResultId": "wire-result-1" }
                    })),
                },
                GenerateContent::Text {
                    text: "after".to_string(),
                    provider_metadata: None,
                },
            ],
            finish_reason: finish_reason(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let parts = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "text-1".to_string(),
                delta: "before".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ToolCall {
                tool_call_id: "srv-1".to_string(),
                tool_name: "server_search".to_string(),
                input: json!(r#"{"query":"Rust"}"#),
                provider_executed: Some(true),
                dynamic: Some(true),
                thought_signature: None,
                invalid: None,
                error: None,
                provider_metadata: Some(json!({
                    "mock": { "serverCallId": "wire-1" }
                })),
            }),
            Ok(StreamPart::ToolResult {
                tool_call_id: "srv-1".to_string(),
                tool_name: "server_search".to_string(),
                result: json!({ "answer": "still running" }),
                is_error: Some(false),
                preliminary: Some(true),
                dynamic: Some(true),
                provider_metadata: Some(json!({
                    "mock": { "serverResultId": "wire-preliminary-1" }
                })),
            }),
            Ok(StreamPart::ToolResult {
                tool_call_id: "srv-1".to_string(),
                tool_name: "server_search".to_string(),
                result: json!({ "answer": 42 }),
                is_error: Some(false),
                preliminary: Some(false),
                dynamic: Some(true),
                provider_metadata: Some(json!({
                    "mock": { "serverResultId": "wire-result-1" }
                })),
            }),
            Ok(StreamPart::TextDelta {
                id: "text-2".to_string(),
                delta: "after".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: finish_reason(),
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request_body: None,
            response_headers: None,
        })
    }
}

fn assert_provider_transcript(messages: &[aimux_core::message::ModelMessage]) {
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::Assistant);
    let MessageContent::Parts(parts) = &messages[0].content else {
        panic!("expected assistant parts");
    };
    assert_eq!(
        parts.len(),
        4,
        "provider order must be preserved: {parts:?}"
    );
    assert!(matches!(
        &parts[0],
        aimux_core::content::ContentPart::Text { text, .. } if text == "before"
    ));
    assert!(matches!(
        &parts[1],
        aimux_core::content::ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_executed: Some(true),
            provider_options: Some(options),
            ..
        } if tool_call_id == "srv-1"
            && tool_name == "server_search"
            && input == &json!({ "query": "Rust" })
            && options["mock"]["serverCallId"] == "wire-1"
    ));
    assert!(matches!(
        &parts[2],
        aimux_core::content::ContentPart::ToolResult {
            tool_call_id,
            tool_name: Some(tool_name),
            result,
            is_error: Some(false),
            preliminary: Some(false),
            dynamic: Some(true),
            provider_options: Some(options),
        } if tool_call_id == "srv-1"
            && tool_name == "server_search"
            && result == &json!({ "answer": 42 })
            && options["mock"]["serverResultId"] == "wire-result-1"
    ));
    assert!(matches!(
        &parts[3],
        aimux_core::content::ContentPart::Text { text, .. } if text == "after"
    ));
}

#[tokio::test]
async fn generate_response_messages_keep_provider_tool_transcript() {
    let result = generate_text(
        &ProviderTranscriptModel,
        "search",
        GenerateTextOptions::default(),
    )
    .await
    .unwrap();

    assert_provider_transcript(&result.response_messages);
}

#[tokio::test]
async fn stream_response_messages_keep_provider_tool_transcript() {
    let result = stream_text(
        &ProviderTranscriptModel,
        "search",
        GenerateTextOptions::default(),
    )
    .await
    .unwrap()
    .consume()
    .await
    .unwrap();

    assert_provider_transcript(&result.response_messages);
}

struct InvalidToolInputModel;

#[async_trait]
impl LanguageModel for InvalidToolInputModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "invalid-tool-input"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::ToolCall {
                tool_call_id: "bad-1".to_string(),
                tool_name: "server_search".to_string(),
                input: "{".to_string(),
                provider_executed: Some(true),
                dynamic: Some(true),
                thought_signature: None,
                provider_metadata: None,
            }],
            finish_reason: finish_reason(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn invalid_primitive_tool_input_is_not_replayed() {
    let result = generate_text(
        &InvalidToolInputModel,
        "search",
        GenerateTextOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.tool_calls[0].input, json!("{"));
    assert_eq!(result.tool_calls[0].invalid, Some(true));

    let MessageContent::Parts(parts) = &result.response_messages[0].content else {
        panic!("expected assistant parts");
    };
    assert!(matches!(
        &parts[0],
        aimux_core::content::ContentPart::ToolCall { input, .. } if input == &json!({})
    ));
}

struct EmptyModel;

#[async_trait]
impl LanguageModel for EmptyModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "empty"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: String::new(),
                provider_metadata: None,
            }],
            finish_reason: finish_reason(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn empty_generation_has_no_empty_assistant_response_message() {
    let result = generate_text(&EmptyModel, "", GenerateTextOptions::default())
        .await
        .unwrap();
    assert!(result.response_messages.is_empty());
}

struct ContentMetadataModel;

#[async_trait]
impl LanguageModel for ContentMetadataModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "content-metadata"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![
                GenerateContent::Text {
                    text: "answer".to_string(),
                    provider_metadata: Some(json!({
                        "google": { "thoughtSignature": "text-generate" }
                    })),
                },
                GenerateContent::Reasoning {
                    text: "think".to_string(),
                    provider_metadata: Some(json!({
                        "anthropic": { "signature": "reason-generate" }
                    })),
                },
            ],
            finish_reason: finish_reason(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let parts = vec![
            Ok(StreamPart::TextStart {
                id: "text-1".to_string(),
                provider_metadata: Some(json!({ "mock": { "phase": "start" } })),
            }),
            Ok(StreamPart::TextDelta {
                id: "text-1".to_string(),
                delta: "answer".to_string(),
                provider_metadata: Some(json!({
                    "google": { "thoughtSignature": "text-delta" }
                })),
            }),
            Ok(StreamPart::TextEnd {
                id: "text-1".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningStart {
                id: "reason-1".to_string(),
                provider_metadata: Some(json!({
                    "anthropic": { "signature": "reason-start" }
                })),
            }),
            Ok(StreamPart::ReasoningDelta {
                id: "reason-1".to_string(),
                delta: "think".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningEnd {
                id: "reason-1".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningStart {
                id: "reason-2".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningDelta {
                id: "reason-2".to_string(),
                delta: "again".to_string(),
                provider_metadata: Some(json!({
                    "anthropic": { "signature": "reason-delta" }
                })),
            }),
            Ok(StreamPart::ReasoningEnd {
                id: "reason-2".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: finish_reason(),
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request_body: None,
            response_headers: None,
        })
    }
}

#[tokio::test]
async fn generate_response_messages_keep_text_and_reasoning_metadata() {
    let result = generate_text(
        &ContentMetadataModel,
        "metadata",
        GenerateTextOptions::default(),
    )
    .await
    .unwrap();
    let MessageContent::Parts(parts) = &result.response_messages[0].content else {
        panic!("expected assistant parts");
    };
    assert!(matches!(
        &parts[0],
        aimux_core::content::ContentPart::Text {
            text,
            provider_options: Some(options),
        } if text == "answer" && options["google"]["thoughtSignature"] == "text-generate"
    ));
    assert!(matches!(
        &parts[1],
        aimux_core::content::ContentPart::Reasoning {
            text,
            signature: Some(signature),
            provider_options: Some(options),
        } if text == "think"
            && signature == "reason-generate"
            && options["anthropic"]["signature"] == "reason-generate"
    ));
}

#[tokio::test]
async fn stream_response_messages_keep_latest_segment_metadata() {
    let result = stream_text(
        &ContentMetadataModel,
        "metadata",
        GenerateTextOptions::default(),
    )
    .await
    .unwrap()
    .consume()
    .await
    .unwrap();
    let MessageContent::Parts(parts) = &result.response_messages[0].content else {
        panic!("expected assistant parts");
    };
    assert_eq!(parts.len(), 3);
    assert!(matches!(
        &parts[0],
        aimux_core::content::ContentPart::Text {
            text,
            provider_options: Some(options),
        } if text == "answer" && options["google"]["thoughtSignature"] == "text-delta"
    ));
    assert!(matches!(
        &parts[1],
        aimux_core::content::ContentPart::Reasoning {
            text,
            signature: Some(signature),
            provider_options: Some(options),
        } if text == "think"
            && signature == "reason-start"
            && options["anthropic"]["signature"] == "reason-start"
    ));
    assert!(matches!(
        &parts[2],
        aimux_core::content::ContentPart::Reasoning {
            text,
            signature: Some(signature),
            provider_options: Some(options),
        } if text == "again"
            && signature == "reason-delta"
            && options["anthropic"]["signature"] == "reason-delta"
    ));
}
