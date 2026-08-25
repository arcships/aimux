//! Tests for M7: top-level result aggregation (reasoning/sources/files/responseMessages).
//!
//! Verifies that `generate_text` extracts reasoning, sources, files, and
//! response_messages from the raw `GenerateResult.content` to the top level.

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::{MessageContent, ModelPrompt, Role};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};

use async_trait::async_trait;

/// A mock model that returns a fixed `GenerateResult` with all content types.
struct RichModel;

#[async_trait]
impl LanguageModel for RichModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-1"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![
                GenerateContent::Reasoning {
                    text: "Let me think...".into(),
                    provider_metadata: Some(serde_json::json!({
                        "anthropic": { "signature": "sig-abc123" }
                    })),
                },
                GenerateContent::Text {
                    text: "Hello!".into(),
                    provider_metadata: None,
                },
                GenerateContent::ToolCall {
                    tool_call_id: "tc-1".into(),
                    tool_name: "search".into(),
                    input: r#"{"q":"test"}"#.to_string(),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                },
                GenerateContent::Source {
                    id: "src-1".into(),
                    source_type: "url".into(),
                    url: Some("https://example.com".into()),
                    title: Some("Example".into()),
                    provider_metadata: None,
                },
                GenerateContent::File {
                    data: FileData::Data {
                        data: FileBytes::Base64("iVBOR".into()),
                    },
                    media_type: "image/png".into(),
                    provider_metadata: None,
                },
            ],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: Default::default(),
            request_body: None,
            response_headers: None,
        })
    }
    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        unimplemented!("streaming not needed for M7 tests")
    }
}

/// A mock model that returns only text (no reasoning/sources/files).
struct PlainModel;

#[async_trait]
impl LanguageModel for PlainModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-2"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: "Just text.".into(),
                provider_metadata: None,
            }],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
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
async fn extracts_reasoning_to_top_level() {
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    assert_eq!(result.reasoning.len(), 1);
    assert_eq!(result.reasoning[0].text, "Let me think...");
    assert_eq!(result.reasoning_text, "Let me think...");
}

#[tokio::test]
async fn extracts_sources_to_top_level() {
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.sources[0].id, "src-1");
    assert_eq!(result.sources[0].source_type, "url");
    assert_eq!(
        result.sources[0].url.as_deref(),
        Some("https://example.com")
    );
    assert_eq!(result.sources[0].title.as_deref(), Some("Example"));
}

#[tokio::test]
async fn extracts_files_to_top_level() {
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].media_type, "image/png");
    assert!(matches!(
        &result.files[0].data,
        FileData::Data { data: FileBytes::Base64(b) } if b == "iVBOR"
    ));
}

#[tokio::test]
async fn response_messages_roundtrip_to_next_turn() {
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    // response_messages must contain exactly one assistant message.
    assert_eq!(result.response_messages.len(), 1);
    let msg = &result.response_messages[0];
    assert_eq!(msg.role, Role::Assistant);

    // Its content must be Parts (not bare Text), containing the text.
    match &msg.content {
        MessageContent::Parts(parts) => {
            assert!(
                parts.iter().any(|p| matches!(p,
                    aimux_core::content::ContentPart::Text { text, .. } if text == "Hello!"
                )),
                "response message must contain the generated text"
            );
        }
        other => panic!("expected Parts, got {other:?}"),
    }

    // The response message must be appendable to a prompt for the next turn.
    let next_prompt: ModelPrompt = vec![
        aimux_core::message::ModelMessage::user("hi"),
        result.response_messages[0].clone(),
        aimux_core::message::ModelMessage::user("follow-up"),
    ]
    .into();
    // If this compiles + runs, the round-trip type-checks.
    let _ = next_prompt;
}

#[tokio::test]
async fn plain_result_has_empty_aggregates() {
    let result = generate_text(&PlainModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    assert!(result.reasoning.is_empty(), "no reasoning expected");
    assert!(result.reasoning_text.is_empty());
    assert!(result.sources.is_empty());
    assert!(result.files.is_empty());
    // response_messages still has the assistant message (with text).
    assert_eq!(result.response_messages.len(), 1);
    assert_eq!(result.text, "Just text.");
}

#[tokio::test]
async fn reasoning_included_in_response_messages_with_signature() {
    // Reasoning MUST appear in responseMessages — it carries the Anthropic
    // thinking-block signature which must be echoed back on the next turn.
    // Consistent with AI SDK's toResponseMessages.
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    let msg = &result.response_messages[0];
    let parts = match &msg.content {
        MessageContent::Parts(p) => p,
        _ => panic!("expected Parts"),
    };
    let reasoning_part = parts
        .iter()
        .find(|p| matches!(p, aimux_core::content::ContentPart::Reasoning { .. }));
    assert!(
        reasoning_part.is_some(),
        "reasoning must be included in responseMessages"
    );
    if let Some(aimux_core::content::ContentPart::Reasoning { signature, .. }) = reasoning_part {
        assert_eq!(
            signature.as_deref(),
            Some("sig-abc123"),
            "Anthropic signature must be preserved in responseMessages"
        );
    }
}

#[tokio::test]
async fn tool_calls_included_in_response_messages() {
    // Tool calls must appear in responseMessages so the assistant reply can
    // be replayed in the next turn (the tool result follows as a separate
    // tool message).
    let result = generate_text(&RichModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    let msg = &result.response_messages[0];
    let parts = match &msg.content {
        MessageContent::Parts(p) => p,
        _ => panic!("expected Parts"),
    };
    let tool_call_part = parts.iter().find(|p| {
        matches!(
            p,
            aimux_core::content::ContentPart::ToolCall { tool_name, .. } if tool_name == "search"
        )
    });
    assert!(
        tool_call_part.is_some(),
        "tool call must be included in responseMessages"
    );
}

/// A mock model that returns reasoning with Bedrock-style signature metadata.
struct BedrockModel;

#[async_trait]
impl LanguageModel for BedrockModel {
    fn provider(&self) -> &str {
        "bedrock"
    }
    fn model_id(&self) -> &str {
        "bedrock-1"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![
                GenerateContent::Text {
                    text: "Hello from Bedrock!".into(),
                    provider_metadata: None,
                },
                GenerateContent::Reasoning {
                    text: "Thinking...".into(),
                    // Bedrock stores signature under BOTH "bedrock" and
                    // "amazonBedrock" keys (see bedrock/model.rs:535-540).
                    provider_metadata: Some(serde_json::json!({
                        "amazonBedrock": { "signature": "bedrock-sig-xyz" },
                        "bedrock": { "signature": "bedrock-sig-xyz" }
                    })),
                },
            ],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
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
async fn bedrock_signature_extracted_from_reasoning_metadata() {
    // Bedrock stores the thinking signature under "bedrock"/"amazonBedrock"
    // keys, not "anthropic". The extraction must find it regardless.
    let result = generate_text(&BedrockModel, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    let msg = &result.response_messages[0];
    let parts = match &msg.content {
        MessageContent::Parts(p) => p,
        _ => panic!("expected Parts"),
    };
    let reasoning_part = parts
        .iter()
        .find_map(|p| match p {
            aimux_core::content::ContentPart::Reasoning { signature, .. } => Some(signature),
            _ => None,
        })
        .expect("reasoning must be in responseMessages");
    assert_eq!(
        reasoning_part.as_deref(),
        Some("bedrock-sig-xyz"),
        "Bedrock signature must be extracted from bedrock/amazonBedrock metadata key"
    );
}
