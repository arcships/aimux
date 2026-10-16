//! Recorder operations. The registry keeps the core `Arc<dyn Recorder>` payload.
//! Recorder/session capture remains process-wide (registered gap G-a).
//!
//! Creation requires `{"backend":{"type":"ring","capacity":2048},
//! "capturePolicy":"full"}` or a `jsonl` backend with `directory`.
//! The full policy uses core recording redaction. There is no implicit policy.
//! All other operations accept `{}`; export returns `{"jsonl":"..."}`.

use std::path::PathBuf;
use std::sync::Arc;

use aimux_core::decision_model::DecideOptions;
use aimux_core::options::CallOptions;
use aimux_core::recording::{
    self, HttpExchange, JsonlRecorder, OutcomeRecord, ProviderRecord, Recorder, RecordingError,
    ResponseRecord, RingRecorder,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode as params};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleId, HandleKind, HandleRegistry};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum CapturePolicy {
    Full,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum BackendConfig {
    Ring { capacity: Option<usize> },
    Jsonl { directory: PathBuf },
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRecorderParams {
    backend: BackendConfig,
    capture_policy: CapturePolicy,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

enum Backend {
    Ring(RingRecorder),
    Jsonl(JsonlRecorder),
}

/// Keeps backend diagnostics. Full capture uses core redaction of sensitive fields.
/// Creating a handle does not attach it; stopping does not flush it.
struct RecorderHandle {
    backend: Backend,
}

impl RecorderHandle {
    fn new(params: CreateRecorderParams) -> Result<Self, OpsError> {
        let CreateRecorderParams {
            backend,
            capture_policy: CapturePolicy::Full,
        } = params;
        let backend = match backend {
            BackendConfig::Ring { capacity } => {
                let capacity = capacity.unwrap_or(recording::DEFAULT_RING_CAPACITY);
                if capacity == 0
                    || capacity > isize::MAX as usize / std::mem::size_of::<recording::Recording>()
                {
                    return Err(invalid("ring capacity must be positive and fit in memory"));
                }
                Backend::Ring(RingRecorder::with_capacity(capacity))
            }
            BackendConfig::Jsonl { directory } => {
                Backend::Jsonl(JsonlRecorder::try_new(directory).map_err(OpsError::Recording)?)
            }
        };
        Ok(Self { backend })
    }

    fn recorder(&self) -> &dyn Recorder {
        match &self.backend {
            Backend::Ring(recorder) => recorder,
            Backend::Jsonl(recorder) => recorder,
        }
    }

    fn stats(&self) -> RecorderStats {
        match &self.backend {
            Backend::Ring(recorder) => RecorderStats::Ring {
                completed: recorder.len(),
                pending: recorder.pending_count(),
                dropped: recorder.dropped_count(),
                pending_evicted: recorder.pending_evicted_count(),
                inconsistent_call_ids: recorder.inconsistent_call_ids(),
            },
            Backend::Jsonl(recorder) => RecorderStats::Jsonl {
                path: recorder.path(),
                dropped: recorder.dropped_count(),
                inconsistent_call_ids: recorder.inconsistent_call_ids(),
                last_write_error: recorder.last_write_error(),
            },
        }
    }

    fn export(&self) -> Result<String, OpsError> {
        match &self.backend {
            Backend::Ring(recorder) => {
                let mut bytes = Vec::new();
                recorder
                    .export_jsonl(&mut bytes)
                    .map_err(|e| OpsError::Recording(RecordingError::Write(e.to_string())))?;
                String::from_utf8(bytes).map_err(|e| {
                    OpsError::Binding(BindingError::ResultSerialization(e.to_string()))
                })
            }
            Backend::Jsonl(recorder) => {
                recorder.try_flush().map_err(OpsError::Recording)?;
                std::fs::read_to_string(recorder.path())
                    .map_err(|e| OpsError::Recording(RecordingError::Write(e.to_string())))
            }
        }
    }
}

impl Recorder for RecorderHandle {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn record_input(&self, id: &str, options: &CallOptions, provider: &str, model: &str) {
        self.recorder().record_input(id, options, provider, model);
    }

    fn record_decision_input(
        &self,
        id: &str,
        options: &DecideOptions,
        provider: &str,
        model: &str,
    ) {
        self.recorder()
            .record_decision_input(id, options, provider, model);
    }

    fn record_session(&self, id: &str, session: &str, step: u32) {
        self.recorder().record_session(id, session, step);
    }

    fn record_provider(&self, id: &str, provider: &ProviderRecord) {
        self.recorder().record_provider(id, provider);
    }

    fn record_exchange(&self, id: &str, exchange: &HttpExchange) {
        self.recorder().record_exchange(id, exchange);
    }

    fn record_exchange_update(
        &self,
        id: &str,
        attempt: u32,
        index: u32,
        response: &ResponseRecord,
        error: Option<String>,
    ) {
        self.recorder()
            .record_exchange_update(id, attempt, index, response, error);
    }

    fn record_outcome(&self, id: &str, outcome: &OutcomeRecord) {
        self.recorder().record_outcome(id, outcome);
    }

    fn record_transport_closed(&self, id: &str) {
        self.recorder().record_transport_closed(id);
    }

    fn flush(&self) {
        self.recorder().flush();
    }

    fn try_flush(&self) -> Result<(), RecordingError> {
        self.recorder().try_flush()
    }
}

fn invalid(message: impl Into<String>) -> OpsError {
    OpsError::Protocol(ProtocolError::InvalidParams(message.into()))
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RecorderHandleResult {
    handle: HandleId,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RecorderExport {
    jsonl: String,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RecorderStats {
    Ring {
        completed: usize,
        pending: usize,
        dropped: u64,
        pending_evicted: u64,
        inconsistent_call_ids: Vec<String>,
    },
    Jsonl {
        path: PathBuf,
        dropped: u64,
        inconsistent_call_ids: Vec<String>,
        last_write_error: Option<String>,
    },
}

fn result(value: impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| BindingError::ResultSerialization(error.to_string()).into())
}

fn recorder_entry(target: Target) -> Result<Arc<dyn Recorder>, OpsError> {
    match target {
        Target::Handle(Entry::Recorder(recorder)) => Ok(recorder),
        target => Err(BindingError::InvalidHandle {
            expected: vec![HandleKind::Recorder],
            actual: match target {
                Target::Global => None,
                Target::Handle(entry) => Some(entry.kind()),
            },
        }
        .into()),
    }
}

fn recorder_handle(recorder: &dyn Recorder) -> Result<&RecorderHandle, OpsError> {
    recorder
        .as_any()
        .and_then(|value| value.downcast_ref())
        .ok_or_else(|| {
            aimux_core::AiMuxError::UnsupportedFunctionality(
                "recorder backend does not expose diagnostics/export".into(),
            )
            .into()
        })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, OpsError> + Send + 'static,
) -> Result<T, OpsError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        OpsError::Protocol(ProtocolError::InvalidState(format!(
            "recorder worker failed: {error}"
        )))
    })?
}

fn create(ctx: Ctx, _target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let params = params(value)?;
        blocking(move || {
            let recorder = Arc::new(RecorderHandle::new(params)?);
            let handle = HandleRegistry::global().insert(ctx.owner, Entry::Recorder(recorder))?;
            result(RecorderHandleResult { handle })
        })
        .await
    })
}

fn attach(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let recorder = recorder_entry(target)?;
        blocking(move || {
            recording::init_recording(Some(recorder));
            Ok(Value::Null)
        })
        .await
    })
}

fn stop(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let recorder = recorder_entry(target)?;
        blocking(move || {
            drop(recording::detach_recording_if(&recorder));
            Ok(Value::Null)
        })
        .await
    })
}

fn flush(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let recorder = recorder_entry(target)?;
        blocking(move || {
            recorder.try_flush().map_err(OpsError::Recording)?;
            Ok(Value::Null)
        })
        .await
    })
}

fn stats(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let recorder = recorder_entry(target)?;
        blocking(move || result(recorder_handle(recorder.as_ref())?.stats())).await
    })
}

fn export(_ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let _: EmptyParams = params(value)?;
        let recorder = recorder_entry(target)?;
        blocking(move || {
            result(RecorderExport {
                jsonl: recorder_handle(recorder.as_ref())?.export()?,
            })
        })
        .await
    })
}

const fn descriptor(
    name: &'static str,
    global: bool,
    request_type: &'static str,
    result_type: &'static str,
    side_effects: bool,
    source_symbols: &'static [&'static str],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: if global { &[] } else { &[HandleKind::Recorder] },
        request_type,
        result_type,
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects,
        retry_owner: "none",
        source_symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "recorder.create",
        true,
        "aimux_ops::ops::recording::CreateRecorderParams",
        "aimux_ops::ops::recording::RecorderHandleResult",
        true,
        &[
            "aimux_init_recording",
            "aimux_init_recording_ring",
            "aimux_init_recording_ring_default",
        ],
    ),
    descriptor(
        "recorder.attach",
        false,
        "aimux_ops::ops::recording::EmptyParams",
        "()",
        true,
        &["aimux_init_recording"],
    ),
    descriptor(
        "recorder.stop",
        false,
        "aimux_ops::ops::recording::EmptyParams",
        "()",
        true,
        &["aimux_recording_stop"],
    ),
    descriptor(
        "recorder.flush",
        false,
        "aimux_ops::ops::recording::EmptyParams",
        "()",
        true,
        &["aimux_recording_flush", "aimux_recording_try_flush"],
    ),
    descriptor(
        "recorder.stats",
        false,
        "aimux_ops::ops::recording::EmptyParams",
        "aimux_ops::ops::recording::RecorderStats",
        false,
        &[
            "RingRecorder::len",
            "RingRecorder::dropped_count",
            "JsonlRecorder::last_write_error",
        ],
    ),
    descriptor(
        "recorder.export",
        false,
        "aimux_ops::ops::recording::EmptyParams",
        "aimux_ops::ops::recording::RecorderExport",
        true,
        &["RingRecorder::export_jsonl", "JsonlRecorder::path"],
    ),
];

pub(crate) fn register(registry: &mut Vec<(&'static OpDescriptor, Handler)>) {
    let handlers: [fn(Ctx, Target, Value) -> CallFuture; 6] =
        [create, attach, stop, flush, stats, export];
    registry.extend(
        OPS.iter()
            .zip(handlers)
            .map(|(descriptor, handler)| (descriptor, Handler::Call(handler))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::recording::{OutcomeStatus, Recording};
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn ring() -> Arc<RecorderHandle> {
        Arc::new(
            RecorderHandle::new(
                params(json!({
                    "backend": {"type": "ring", "capacity": 2}, "capturePolicy": "full"
                }))
                .unwrap(),
            )
            .unwrap(),
        )
    }

    fn record(recorder: &RecorderHandle, id: &str) {
        let options: CallOptions = serde_json::from_value(json!({
            "prompt": [{"role": "user", "content": [{"type": "text", "text": "hello"}]}]
        }))
        .unwrap();
        recorder.record_input(id, &options, "test", "model");
        recorder.record_outcome(
            id,
            &OutcomeRecord {
                status: OutcomeStatus::Success,
                ..Default::default()
            },
        );
        recorder.record_transport_closed(id);
    }

    fn context() -> Ctx {
        Ctx {
            owner: crate::handle::Owner::CProcess,
            abort: None,
            now: std::time::SystemTime::now,
        }
    }

    #[test]
    fn strict_controls_and_explicit_policy() {
        for value in [
            json!({"backend": {"type": "ring"}}),
            json!({"backend": {"type": "ring", "unknown": 1}, "capturePolicy": "full"}),
            json!({"backend": {"type": "ring"}, "capturePolicy": "unknown"}),
            json!({"backend": {"type": "ring"}, "capturePolicy": "full", "unknown": 1}),
        ] {
            assert!(params::<CreateRecorderParams>(value).is_err());
        }
        assert!(params::<EmptyParams>(json!({"unknown": 1})).is_err());
        for capacity in [0, usize::MAX] {
            assert!(
                RecorderHandle::new(CreateRecorderParams {
                    backend: BackendConfig::Ring {
                        capacity: Some(capacity)
                    },
                    capture_policy: CapturePolicy::Full,
                })
                .is_err()
            );
        }
    }

    #[test]
    fn ring_exports_real_recordings_and_reports_eviction() {
        let recorder = ring();
        assert!(recorder.export().unwrap().is_empty());
        for id in ["first", "second", "third"] {
            record(&recorder, id);
        }
        recorder.try_flush().unwrap();
        let exported = recorder.export().unwrap();
        let records: Vec<Recording> = exported
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].call_id, "second");
        assert!(records.iter().all(|r| r.complete));
        assert_eq!(result(recorder.stats()).unwrap()["dropped"], 1);
        assert_eq!(result(recorder.stats()).unwrap()["pending"], 0);
    }

    #[test]
    fn forwards_session_provider_and_stream_exchange_completion() {
        let recorder = ring();
        let exchange: HttpExchange = serde_json::from_value(json!({
            "attempt": 1, "exchangeIndex": 1,
            "request": {"method": "POST", "url": "https://example.test", "headers": [], "body": "request"},
            "timing": {"latencyMs": 1}, "finalized": false
        })).unwrap();
        recorder.record_session("stream", "session", 2);
        recorder.record_provider(
            "stream",
            &ProviderRecord::new("provider-id", "test", "model"),
        );
        recorder.record_exchange("stream", &exchange);
        record(&recorder, "stream");
        assert_eq!(result(recorder.stats()).unwrap()["completed"], 0);
        assert_eq!(result(recorder.stats()).unwrap()["pending"], 1);
        let response: ResponseRecord = serde_json::from_value(json!({
            "status": 200, "headers": [], "body": "completed", "streamChunks": 2
        }))
        .unwrap();
        recorder.record_exchange_update("stream", 1, 1, &response, None);
        let saved: Recording = serde_json::from_str(recorder.export().unwrap().trim()).unwrap();
        assert!(saved.complete);
        assert_eq!(saved.session_id.as_deref(), Some("session"));
        assert_eq!(saved.step, Some(2));
        assert_eq!(saved.provider.provider_id, "provider-id");
        assert_eq!(
            saved.exchanges[0]
                .response
                .as_ref()
                .unwrap()
                .body
                .as_deref(),
            Some("completed")
        );
    }

    fn directory() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "aimux-ops-recorder-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn jsonl_flush_and_export_and_init_failures() {
        let dir = directory();
        let recorder = RecorderHandle::new(CreateRecorderParams {
            backend: BackendConfig::Jsonl {
                directory: dir.clone(),
            },
            capture_policy: CapturePolicy::Full,
        })
        .unwrap();
        record(&recorder, "persisted");
        recorder.try_flush().unwrap();
        let exported = recorder.export().unwrap();
        let saved: Recording = serde_json::from_str(exported.trim()).unwrap();
        assert_eq!(saved.call_id, "persisted");
        assert!(saved.complete);
        assert_eq!(
            std::fs::read_to_string(dir.join("recordings.jsonl")).unwrap(),
            exported
        );
        // Export reports a real read failure rather than returning empty data.
        std::fs::remove_file(dir.join("recordings.jsonl")).unwrap();
        assert!(matches!(
            recorder.export(),
            Err(OpsError::Recording(RecordingError::Write(_)))
        ));
        drop(recorder);
        let blocked_file = dir.join("blocked-file");
        std::fs::create_dir_all(blocked_file.join("recordings.jsonl")).unwrap();
        assert!(matches!(
            RecorderHandle::new(CreateRecorderParams {
                backend: BackendConfig::Jsonl {
                    directory: blocked_file
                },
                capture_policy: CapturePolicy::Full,
            }),
            Err(OpsError::Recording(RecordingError::OpenFile { .. }))
        ));
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "occupied").unwrap();
        assert!(matches!(
            RecorderHandle::new(CreateRecorderParams {
                backend: BackendConfig::Jsonl {
                    directory: blocker.join("nested")
                },
                capture_policy: CapturePolicy::Full,
            }),
            Err(OpsError::Recording(RecordingError::Init { .. }))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn flush_reports_actual_persistence_failure() {
        use std::os::unix::fs::symlink;
        let dir = directory();
        std::fs::create_dir_all(&dir).unwrap();
        symlink("/dev/full", dir.join("recordings.jsonl")).unwrap();
        let recorder = Arc::new(
            RecorderHandle::new(CreateRecorderParams {
                backend: BackendConfig::Jsonl {
                    directory: dir.clone(),
                },
                capture_policy: CapturePolicy::Full,
            })
            .unwrap(),
        );
        record(&recorder, "cannot-sync");
        // The real writer receives ENOSPC when flushing its buffer.
        assert!(matches!(
            flush(
                context(),
                Target::Handle(Entry::Recorder(recorder.clone())),
                json!({})
            )
            .await,
            Err(OpsError::Recording(RecordingError::Write(_)))
        ));
        assert!(recorder.export().is_err());
        drop(recorder);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn attach_is_explicit_and_stop_only_detaches_its_own_recorder() {
        let first = ring();
        let second = ring();
        {
            attach(
                context(),
                Target::Handle(Entry::Recorder(first.clone())),
                json!({}),
            )
            .await
            .unwrap();
            let snapshot = recording::recorder().unwrap();
            attach(
                context(),
                Target::Handle(Entry::Recorder(second.clone())),
                json!({}),
            )
            .await
            .unwrap();
            stop(context(), Target::Handle(Entry::Recorder(first)), json!({}))
                .await
                .unwrap();
            let second_trait: Arc<dyn Recorder> = second.clone();
            assert!(Arc::ptr_eq(&recording::recorder().unwrap(), &second_trait));
            stop(
                context(),
                Target::Handle(Entry::Recorder(second.clone())),
                json!({}),
            )
            .await
            .unwrap();
            assert!(recording::recorder().is_none());
            assert!(!Arc::ptr_eq(&snapshot, &second_trait));
            record(&second, "after-stop");
            assert_eq!(result(second.stats()).unwrap()["completed"], 1);
            assert_eq!(
                stats(
                    context(),
                    Target::Handle(Entry::Recorder(second.clone())),
                    json!({})
                )
                .await
                .unwrap()["completed"],
                1
            );
            assert!(
                export(
                    context(),
                    Target::Handle(Entry::Recorder(second)),
                    json!({"extra": true})
                )
                .await
                .is_err()
            );
        }
    }
}
