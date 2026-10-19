//! `stream_text_as_openai` chunks carry the provider's response id and model
//! (RFC-0026 §4.4: `ResponseMetadata` records id/model). The OpenAI output
//! is an aimux extension with no AI SDK counterpart.

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, stream_text_as_openai};
use aimux_core::language_model::LanguageModel;
use aimux_core::openai_output::{ChatCompletionChunk, OpenAiStreamOptions};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};
use async_trait::async_trait;
use futures::StreamExt;

/// A model whose stream reports `metadata` first, then `text` (no text parts
/// when empty), then finishes.
struct ReportingModel {
    metadata: Option<ResponseMetadata>,
    text: &'static str,
}

#[async_trait]
impl LanguageModel for ReportingModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "requested-model"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unreachable!("the tests stream")
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let mut parts = vec![StreamPart::StreamStart { warnings: vec![] }];
        parts.extend(self.metadata.clone().map(StreamPart::ResponseMetadata));
        if !self.text.is_empty() {
            parts.extend([
                StreamPart::TextStart {
                    id: "0".into(),
                    provider_metadata: None,
                },
                StreamPart::TextDelta {
                    id: "0".into(),
                    delta: self.text.into(),
                    provider_metadata: None,
                },
                StreamPart::TextEnd {
                    id: "0".into(),
                    provider_metadata: None,
                },
            ]);
        }
        parts.push(StreamPart::Finish {
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: Some("stop".into()),
            },
            usage: Usage::default(),
            provider_metadata: None,
        });
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts.into_iter().map(Ok))),
            request: None,
            response: None,
        })
    }
}

fn reported() -> Option<ResponseMetadata> {
    Some(ResponseMetadata {
        id: Some("chatcmpl-upstream".into()),
        timestamp: None,
        model_id: Some("gpt-4o-2026-08-06".into()),
    })
}

async fn chunks(model: ReportingModel) -> Vec<ChatCompletionChunk> {
    stream_text_as_openai(
        &model,
        "hi",
        GenerateTextOptions::default(),
        OpenAiStreamOptions::default(),
    )
    .await
    .unwrap()
    .stream
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .collect::<Result<_, _>>()
    .unwrap()
}

#[tokio::test]
async fn every_chunk_carries_the_reported_id_and_model() {
    let chunks = chunks(ReportingModel {
        metadata: reported(),
        text: "Hello",
    })
    .await;
    assert_eq!(chunks.len(), 3, "{chunks:?}");
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(chunks[1].choices[0].delta.content.as_deref(), Some("Hello"));
    assert_eq!(chunks[2].choices[0].finish_reason.as_deref(), Some("stop"));
    for chunk in &chunks {
        assert_eq!(chunk.id, "chatcmpl-upstream");
        assert_eq!(chunk.model, "gpt-4o-2026-08-06");
    }
}

#[tokio::test]
async fn a_stream_without_content_still_starts_with_the_role_frame() {
    let chunks = chunks(ReportingModel {
        metadata: reported(),
        text: "",
    })
    .await;
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(chunks[1].choices[0].finish_reason.as_deref(), Some("stop"));
    assert!(chunks.iter().all(|chunk| chunk.id == "chatcmpl-upstream"));
}

#[tokio::test]
async fn without_a_report_the_chunks_share_a_generated_id_and_the_requested_model() {
    let chunks = chunks(ReportingModel {
        metadata: None,
        text: "Hello",
    })
    .await;
    assert_eq!(chunks.len(), 3, "{chunks:?}");
    assert!(chunks[0].id.starts_with("chatcmpl-"));
    for chunk in &chunks {
        assert_eq!(chunk.id, chunks[0].id);
        assert_eq!(chunk.model, "requested-model");
    }
}

#[tokio::test]
async fn without_a_report_or_content_the_chunks_keep_the_generated_id() {
    let chunks = chunks(ReportingModel {
        metadata: None,
        text: "",
    })
    .await;
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant")
    );
    assert!(chunks[0].id.starts_with("chatcmpl-"), "{chunks:?}");
    for chunk in &chunks {
        assert_eq!(chunk.id, chunks[0].id);
        assert_eq!(chunk.model, "requested-model");
    }
}
