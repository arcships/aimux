//! Tests for M11 (streamText aggregation), M12 (generateObject), and M6 (proxy).

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_object};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::MessageContent;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};

use async_trait::async_trait;
use futures::stream;

// ── Mock models ─────────────────────────────────────────────────────────────

/// A mock model whose `do_generate` returns a JSON string (for M12 tests).
struct JsonModel;

#[async_trait]
impl LanguageModel for JsonModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-json"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: r#"{"name":"John","age":25}"#.into(),
                provider_metadata: None,
            }],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: Some("stop".into()),
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

/// A mock model whose `do_generate` returns non-JSON text (for M12 failure test).
struct NonJsonModel;

#[async_trait]
impl LanguageModel for NonJsonModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-nonjson"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: "This is not JSON at all.".into(),
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

/// A mock model whose `do_stream` yields a rich set of StreamParts (for M11).
struct StreamModel;

#[async_trait]
impl LanguageModel for StreamModel {
    fn provider(&self) -> &str {
        "mock"
    }
    fn model_id(&self) -> &str {
        "mock-stream"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!()
    }
    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::ReasoningStart {
                id: "r1".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningDelta {
                id: "r1".into(),
                delta: "Thinking...".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::ReasoningEnd {
                id: "r1".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextDelta {
                id: "t1".into(),
                delta: "Hello ".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::TextDelta {
                id: "t1".into(),
                delta: "world!".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Source {
                id: "s1".into(),
                source_type: "url".into(),
                url: Some("https://example.com".into()),
                title: Some("Example".into()),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: Some("stop".into()),
                },
                usage: Usage {
                    input_tokens: aimux_core::types::TokenUsage {
                        total: Some(10),
                        ..Default::default()
                    },
                    output_tokens: aimux_core::types::TokenUsage {
                        total: Some(20),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(stream::iter(parts)),
            request_body: None,
            response_headers: None,
        })
    }
}

// ── M12: generateObject ──────────────────────────────────────────────────────

#[tokio::test]
async fn generate_object_parses_json_output() {
    let result = generate_object(
        &JsonModel,
        "Extract user info",
        GenerateTextOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(serde_json::json!({"type": "object"})),
                name: None,
                description: None,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(result.object["name"], "John");
    assert_eq!(result.object["age"], 25);
    assert_eq!(result.raw.text, r#"{"name":"John","age":25}"#);
}

#[tokio::test]
async fn generate_object_non_json_fails() {
    let err = generate_object(&NonJsonModel, "whatever", GenerateTextOptions::default())
        .await
        .unwrap_err();

    assert!(
        matches!(err, AiMuxError::JsonParse(ref m) if m.contains("not valid JSON")),
        "expected JsonParse error, got {err:?}"
    );
}

// ── M11: streamText aggregation ──────────────────────────────────────────────

#[tokio::test]
async fn consume_stream_aggregates_text_and_reasoning() {
    let result =
        aimux_core::generate::stream_text(&StreamModel, "hi", GenerateTextOptions::default())
            .await
            .unwrap();
    let agg = result.consume().await.unwrap();

    assert_eq!(agg.text, "Hello world!");
    assert_eq!(agg.reasoning.len(), 1);
    assert_eq!(agg.reasoning[0].text, "Thinking...");
    assert_eq!(agg.reasoning_text, "Thinking...");
}

#[tokio::test]
async fn consume_stream_captures_usage_and_finish_reason() {
    let result =
        aimux_core::generate::stream_text(&StreamModel, "hi", GenerateTextOptions::default())
            .await
            .unwrap();
    let agg = result.consume().await.unwrap();

    assert_eq!(agg.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(agg.raw_finish_reason.as_deref(), Some("stop"));
    assert_eq!(agg.usage.input_tokens.total, Some(10));
    assert_eq!(agg.usage.output_tokens.total, Some(20));
}

#[tokio::test]
async fn consume_stream_captures_sources() {
    let result =
        aimux_core::generate::stream_text(&StreamModel, "hi", GenerateTextOptions::default())
            .await
            .unwrap();
    let agg = result.consume().await.unwrap();

    assert_eq!(agg.sources.len(), 1);
    assert_eq!(agg.sources[0].id, "s1");
    assert_eq!(agg.sources[0].url.as_deref(), Some("https://example.com"));
}

#[tokio::test]
async fn consume_stream_builds_response_messages() {
    let result =
        aimux_core::generate::stream_text(&StreamModel, "hi", GenerateTextOptions::default())
            .await
            .unwrap();
    let agg = result.consume().await.unwrap();

    assert_eq!(agg.response_messages.len(), 1);
    assert_eq!(
        agg.response_messages[0].role,
        aimux_core::message::Role::Assistant
    );
    // The text must be in the response message.
    match &agg.response_messages[0].content {
        MessageContent::Parts(parts) => {
            assert!(parts.iter().any(|p| matches!(p,
                aimux_core::content::ContentPart::Text { text, .. } if text == "Hello world!"
            )));
        }
        _ => panic!("expected Parts"),
    }
}
