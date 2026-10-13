//! The retry count, the timeout and the session id are settings of the
//! `generate_text` / `stream_text` operation, not model call options, as in
//! the AI SDK:
//!
//! - `LanguageModelV4CallOptions` has none of them (@ai-sdk/provider@4.0.21
//!   `language-model/v4/language-model-v4-call-options.ts:8-138`);
//! - `maxRetries` and `timeout` are request options of the operation
//!   (ai@7.0.127 `prompt/request-options.ts:104-127`), reported on the
//!   operation's start event (`generate-text/generate-text-events.ts:54-62`).
//!
//! The router, MoA, the trace layer and the recorder read them from the
//! operation they run in.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::moa::{MoaConfig, MoaModel};
use aimux_core::operation::OperationSettings;
use aimux_core::options::{CallOptions, TimeoutConfiguration};
use aimux_core::recording::{Recorder, RingRecorder, init_recording};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::router::{FallbackPolicy, RouterConfig, RouterModel, RuleRouter};
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{TraceLayer, TraceRecord, TraceSink};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use aimux_core::{AbortSignal, ApiCallError};
use async_trait::async_trait;
use futures::StreamExt;

/// A model that fails its first `failures` calls with a retryable error and
/// keeps what each call saw: the serialized call options and the current
/// operation settings.
#[derive(Default)]
struct Probe {
    failures: usize,
    calls: AtomicUsize,
    options: Mutex<Vec<serde_json::Value>>,
    settings: Mutex<Vec<Option<OperationSettings>>>,
}

impl Probe {
    fn failing(failures: usize) -> Self {
        Self {
            failures,
            ..Self::default()
        }
    }

    fn observe(&self, options: &CallOptions) -> Result<(), AiMuxError> {
        self.options
            .lock()
            .unwrap()
            .push(serde_json::to_value(options).unwrap());
        self.settings
            .lock()
            .unwrap()
            .push(OperationSettings::current());
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt <= self.failures {
            return Err(retryable_error());
        }
        Ok(())
    }
}

fn retryable_error() -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(503),
        response_headers: Some(std::collections::HashMap::from([(
            "retry-after-ms".into(),
            "0".into(),
        )])),
        is_retryable: true,
        ..ApiCallError::new(
            "retryable failure",
            "https://example.test",
            serde_json::json!({}),
        )
    }))
}

fn stop() -> FinishReason {
    FinishReason {
        unified: FinishReasonUnified::Stop,
        raw: None,
    }
}

#[async_trait]
impl LanguageModel for Probe {
    fn provider(&self) -> &str {
        "probe"
    }
    fn model_id(&self) -> &str {
        "probe-1"
    }
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        self.observe(options)?;
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
        self.observe(options)?;
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

fn settings() -> OperationSettings {
    OperationSettings {
        max_retries: Some(1),
        timeout: Some(TimeoutConfiguration {
            total_ms: Some(60_000),
            ..TimeoutConfiguration::default()
        }),
        session_id: Some("session-op".into()),
    }
}

fn options_with(settings: &OperationSettings) -> GenerateTextOptions {
    GenerateTextOptions {
        max_retries: settings.max_retries,
        timeout: settings.timeout,
        session_id: settings.session_id.clone(),
        ..Default::default()
    }
}

fn assert_no_operation_settings(options: &serde_json::Value) {
    for key in ["max_retries", "timeout", "session_id"] {
        assert!(
            options.get(key).is_none(),
            "call options carry `{key}`: {options}"
        );
    }
}

#[tokio::test]
async fn the_model_call_gets_no_operation_settings_in_its_options() {
    let probe = Probe::default();
    generate_text(&probe, "hi", options_with(&settings()))
        .await
        .unwrap();
    let result = stream_text(&probe, "hi", options_with(&settings()))
        .await
        .unwrap();
    result.stream.collect::<Vec<_>>().await;

    let options = probe.options.lock().unwrap();
    assert_eq!(options.len(), 2);
    options.iter().for_each(assert_no_operation_settings);
    // The model runs inside the operation and can read them there.
    assert_eq!(
        *probe.settings.lock().unwrap(),
        vec![Some(settings()), Some(settings())]
    );
    // A direct call runs in no operation.
    assert_eq!(OperationSettings::current(), None);
}

#[tokio::test]
async fn the_operation_retry_count_retries_the_model_call() {
    // `prepareRetries({ maxRetries })` of the operation
    // (generate-text.ts:634-637): one retry, then the call fails.
    let probe = Probe::failing(5);
    let error = generate_text(
        &probe,
        "hi",
        GenerateTextOptions {
            max_retries: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AiMuxError::Retry(_)), "{error:?}");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
}

fn router(child: Arc<Probe>) -> RouterModel {
    RouterModel::new(
        vec![child],
        Box::new(RuleRouter),
        FallbackPolicy::None,
        RouterConfig::default(),
    )
}

#[tokio::test]
async fn a_router_child_takes_its_retry_count_from_the_operation() {
    let prompt = aimux_core::language_model_message::LanguageModelPrompt::new();

    // Inside an operation with `max_retries: 0`, the child is called once.
    let child = Arc::new(Probe::failing(1));
    let outcome = OperationSettings {
        max_retries: Some(0),
        ..OperationSettings::default()
    }
    .scope(router(child.clone()).do_generate(&CallOptions::new(prompt.clone())))
    .await;
    assert!(outcome.is_err());
    assert_eq!(child.calls.load(Ordering::SeqCst), 1);

    // With `max_retries: 1` the child gets one retry and succeeds.
    let child = Arc::new(Probe::failing(1));
    let outcome = OperationSettings {
        max_retries: Some(1),
        ..OperationSettings::default()
    }
    .scope(router(child.clone()).do_generate(&CallOptions::new(prompt.clone())))
    .await;
    assert!(outcome.is_ok());
    assert_eq!(child.calls.load(Ordering::SeqCst), 2);

    // Through `generate_text`, the operation's `max_retries` reaches the
    // child the same way.
    let child = Arc::new(Probe::failing(1));
    let error = generate_text(
        &router(child.clone()),
        "hi",
        GenerateTextOptions {
            max_retries: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AiMuxError::ApiCall(_)), "{error:?}");
    assert_eq!(child.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn moa_children_take_their_retry_count_from_the_operation() {
    let prompt = aimux_core::language_model_message::LanguageModelPrompt::new();
    for (max_retries, calls, succeeds) in [(0, 1, false), (1, 2, true)] {
        let reference = Arc::new(Probe::default());
        let aggregator = Arc::new(Probe::failing(1));
        let moa = MoaModel::new(
            vec![reference.clone()],
            aggregator.clone(),
            MoaConfig::default(),
        );
        let outcome = OperationSettings {
            max_retries: Some(max_retries),
            ..OperationSettings::default()
        }
        .scope(moa.do_generate(&CallOptions::new(prompt.clone())))
        .await;
        assert_eq!(outcome.is_ok(), succeeds, "max_retries {max_retries}");
        assert_eq!(aggregator.calls.load(Ordering::SeqCst), calls);
        assert_eq!(reference.calls.load(Ordering::SeqCst), 1);
    }
}

#[derive(Default)]
struct Sink(Mutex<Vec<TraceRecord>>);

impl TraceSink for Sink {
    fn record(&self, record: TraceRecord) {
        self.0.lock().unwrap().push(record);
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn the_trace_layer_takes_the_session_id_from_the_operation() {
    let sink = Arc::new(Sink::default());
    let layer = TraceLayer::new(Arc::new(Probe::default()), sink.clone())
        .with_default_session("session-default".into(), [7; 32]);

    generate_text(&layer, "hi", options_with(&settings()))
        .await
        .unwrap();
    let result = stream_text(&layer, "hi", options_with(&settings()))
        .await
        .unwrap();
    result.stream.collect::<Vec<_>>().await;
    generate_text(&layer, "hi", GenerateTextOptions::default())
        .await
        .unwrap();

    let sessions: Vec<_> = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|record| record.session_id.clone())
        .collect();
    assert_eq!(
        sessions,
        vec![
            Some("session-op".to_string()),
            Some("session-op".to_string()),
            Some("session-default".to_string()),
        ]
    );
}

#[tokio::test]
async fn the_recorded_input_keeps_the_operation_settings_and_replay_restores_them() {
    let ring = Arc::new(RingRecorder::new());
    init_recording(Some(ring.clone()));
    let probe = Probe::default();
    let result = generate_text(
        &probe,
        "operation settings are recorded",
        GenerateTextOptions {
            abort_signal: Some(AbortSignal::new()),
            ..options_with(&settings())
        },
    )
    .await;
    init_recording(None);
    let call_id = result.unwrap().final_step().call_id.clone();
    ring.flush();
    let recording = ring.get(&call_id).expect("the call is recorded");

    // Under the keys the call options used to carry, so the record format
    // is unchanged.
    let options = &recording.input.options;
    assert_eq!(options["max_retries"], 1);
    assert_eq!(options["timeout"]["total_ms"], 60_000);
    assert_eq!(options["session_id"], "session-op");

    let replayed = Probe::default();
    aimux_core::replay::replay_with_model(&recording, &replayed, None)
        .await
        .unwrap();
    assert_eq!(*replayed.settings.lock().unwrap(), vec![Some(settings())]);
    assert_no_operation_settings(&replayed.options.lock().unwrap()[0]);
}
