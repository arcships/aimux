//! RFC-0015 × RFC-0023: under an enabled recorder, `generate_text` and
//! `stream_text` through a `TraceLayer` trace each call under the id of its
//! recording, and that recording replays the call the model saw.
//!
//! One test in this binary: `init_recording` is process-global.

use std::sync::{Arc, Mutex};

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::recording::{Recorder, RecordingContext, RingRecorder, init_recording};
use aimux_core::replay::{ReplayOverrides, replay_with_model};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{RingTraceStore, TraceLayer};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use futures::StreamExt;

/// Answers "ok" and keeps the call options of every call, with the call id
/// of the recording scope it ran in.
#[derive(Default)]
struct Echo {
    seen: Mutex<Vec<(CallOptions, Option<String>)>>,
}

impl Echo {
    fn observe(&self, options: &CallOptions) {
        let call_id = RecordingContext::current().map(|context| context.call_id);
        self.seen.lock().unwrap().push((options.clone(), call_id));
    }

    fn calls(&self) -> Vec<(CallOptions, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

fn stop() -> FinishReason {
    FinishReason {
        unified: FinishReasonUnified::Stop,
        raw: None,
    }
}

#[async_trait]
impl LanguageModel for Echo {
    fn provider(&self) -> &str {
        "echo"
    }

    fn model_id(&self) -> &str {
        "echo-1"
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        self.observe(options);
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: "ok".into(),
                provider_metadata: None,
            }],
            finish_reason: stop(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: None,
            request: None,
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        self.observe(options);
        let parts: Vec<Result<StreamPart, AiMuxError>> = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "t".into(),
                delta: "ok".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: stop(),
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request: None,
            response: None,
        })
    }
}

/// Turns the global recorder off again, also when an assertion fails.
struct StopRecording;

impl Drop for StopRecording {
    fn drop(&mut self) {
        init_recording(None);
    }
}

fn options() -> GenerateTextOptions {
    GenerateTextOptions {
        temperature: Some(0.7),
        session_id: Some("sess-rec".into()),
        ..GenerateTextOptions::default()
    }
}

#[tokio::test]
async fn a_traced_call_carries_its_recording_id_and_the_recording_replays() {
    let ring = Arc::new(RingRecorder::new());
    let store = Arc::new(RingTraceStore::new());
    let echo = Arc::new(Echo::default());
    let layer = TraceLayer::new(echo.clone(), store.clone());

    let recording = StopRecording;
    init_recording(Some(ring.clone()));
    generate_text(&layer, "recorded and traced", options())
        .await
        .unwrap();
    let streamed = stream_text(&layer, "recorded and traced", options())
        .await
        .unwrap();
    streamed.stream.collect::<Vec<_>>().await;
    drop(recording);
    // stream_text's pump task, not the consumer, lets go of the provider
    // stream, and the trace of a stream is written then.
    for _ in 0..100 {
        if store.len() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    ring.flush();

    // Both calls are traced in the session under the ids the recorder gave
    // them, not ids the trace layer minted.
    let ids = store
        .session_chain("sess-rec")
        .expect("both calls are traced in the session")
        .record_ids;
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|id| !id.starts_with("trace-")), "{ids:?}");

    // Each trace id names the recording of the call the model saw.
    let calls = echo.calls();
    assert_eq!(calls.len(), 2);
    for (id, (call, call_id)) in ids.iter().zip(&calls) {
        assert_eq!(call_id.as_deref(), Some(id.as_str()));
        let recorded = ring
            .get(id)
            .unwrap_or_else(|| panic!("no recording for the traced call {id}"));
        assert_eq!(recorded.input.prompt, call.prompt);

        // Request replay sends the recorded call again.
        let replayed = Echo::default();
        replay_with_model(&recorded, &replayed, None).await.unwrap();
        let resent = replayed.calls();
        assert_eq!(resent.len(), 1);
        assert_eq!(resent[0].0.prompt, call.prompt);
        assert_eq!(resent[0].0.temperature, Some(0.7));
    }

    // ... and with an override.
    let recorded = ring
        .get(&ids[0])
        .expect("the generate_text call is recorded");
    let prompt = vec![LanguageModelMessage::user_text("overridden")];
    let overrides = ReplayOverrides {
        prompt: Some(prompt.clone()),
        ..ReplayOverrides::default()
    };
    let replayed = Echo::default();
    replay_with_model(&recorded, &replayed, Some(&overrides))
        .await
        .unwrap();
    assert_eq!(replayed.calls()[0].0.prompt, prompt);
}
