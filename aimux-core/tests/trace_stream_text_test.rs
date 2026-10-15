//! A `stream_text` call that completes through a `TraceLayer` is traced as
//! completed, and its trace takes part in the LCP audit.

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::shared::RequestInfo;
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{RingTraceStore, TraceLayer, TraceRecord};
use aimux_core::types::{FinishReason, FinishReasonUnified, InputTokenUsage, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

/// Streams one delta and `Finish` for a request body of over two 4 KiB blocks.
struct Streaming;

#[async_trait]
impl LanguageModel for Streaming {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "mock-1"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unreachable!("stream-only mock")
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let usage = Usage {
            input_tokens: InputTokenUsage {
                total: Some(2100),
                ..InputTokenUsage::default()
            },
            ..Usage::default()
        };
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "t".into(),
                delta: "ok".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage,
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request: Some(RequestInfo {
                body: Some(
                    json!({ "messages": [{ "role": "user", "content": "x".repeat(8400) }] }),
                ),
            }),
            response: None,
        })
    }
}

fn records(store: &RingTraceStore) -> Vec<TraceRecord> {
    let mut buf = Vec::new();
    store.export_jsonl(&mut buf).expect("export");
    String::from_utf8(buf)
        .expect("utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one TraceRecord per line"))
        .collect()
}

/// `stream_text`'s pump stops at the terminal part and drops the provider
/// stream without polling it to its end, so the trace must be written when
/// `Finish` passes: waiting for end of stream left every completed call to
/// `Drop`, which traced it as "stream dropped before completion" and skipped
/// its LCP lookup.
#[tokio::test]
async fn a_completed_stream_text_call_is_traced_as_completed() {
    let store = Arc::new(RingTraceStore::new());
    let layer = TraceLayer::new(Arc::new(Streaming), store.clone());
    for _ in 0..2 {
        let result = stream_text(&layer, "hi", GenerateTextOptions::default())
            .await
            .unwrap();
        result.stream.collect::<Vec<_>>().await;
    }
    // The pump task, not the consumer, lets go of the provider stream.
    for _ in 0..100 {
        if store.len() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }

    let recs = records(&store);
    assert_eq!(recs.len(), 2);
    for rec in &recs {
        assert_eq!(rec.error, None, "{rec:?}");
    }
    assert!(
        recs[1].lcp_token_upper.is_some(),
        "the second call repeats the first body, so the LCP audit finds it"
    );
}
