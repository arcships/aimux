//! Recording family tests through production dispatch and owner resolution.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aimux_core::recording::{self, Recorder, Recording, RecordingError, RingRecorder};
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::OpsError;
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use serde_json::{Value, json};

fn ctx(owner: Owner) -> Ctx {
    Ctx { owner, abort: None }
}

async fn call(
    owner: Owner,
    handle: HandleId,
    name: &str,
    params: Value,
) -> Result<Value, OpsError> {
    dispatch::call(ctx(owner), handle, name, params).await
}

async fn ring(owner: Owner) -> HandleId {
    let value = call(
        owner,
        HandleId(0),
        "recorder.create",
        json!({
            "backend":{"type":"ring","capacity":2}, "capturePolicy":"full"
        }),
    )
    .await
    .unwrap();
    serde_json::from_value(value["handle"].clone()).unwrap()
}

fn record(recorder: &dyn Recorder, id: &str) {
    let options = serde_json::from_value(json!({
        "prompt":[{"role":"user","content":[{"type":"text","text":"hello"}]}]
    }))
    .unwrap();
    recorder.record_input(id, &options, "test", "model");
    recorder.record_outcome(
        id,
        &recording::OutcomeRecord {
            status: recording::OutcomeStatus::Success,
            ..Default::default()
        },
    );
    recorder.record_transport_closed(id);
}

#[tokio::test]
async fn ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles() {
    let owner = Owner::new_node().unwrap();
    let previous = recording::recorder();
    let first = ring(owner).await;
    assert!(
        match (recording::recorder(), &previous) {
            (Some(current), Some(previous)) => Arc::ptr_eq(&current, previous),
            (None, None) => true,
            _ => false,
        },
        "create must not attach"
    );
    call(owner, first, "recorder.attach", json!({}))
        .await
        .unwrap();
    let snapshot = recording::recorder().unwrap();
    for id in ["first", "second", "third"] {
        record(snapshot.as_ref(), id);
    }
    let stats = call(owner, first, "recorder.stats", json!({}))
        .await
        .unwrap();
    assert_eq!(stats["completed"], 2);
    assert_eq!(stats["dropped"], 1);
    let exported = call(owner, first, "recorder.export", json!({}))
        .await
        .unwrap();
    let records: Vec<Recording> = exported["jsonl"]
        .as_str()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records
            .iter()
            .map(|r| r.call_id.as_str())
            .collect::<Vec<_>>(),
        ["second", "third"]
    );
    assert!(records.iter().all(|r| r.complete));
    let second = ring(owner).await;
    call(owner, second, "recorder.attach", json!({}))
        .await
        .unwrap();
    let active = recording::recorder().unwrap();
    call(owner, first, "recorder.stop", json!({}))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&active, &recording::recorder().unwrap()));
    call(owner, second, "recorder.stop", json!({}))
        .await
        .unwrap();
    call(owner, second, "recorder.stop", json!({}))
        .await
        .unwrap();
    assert!(recording::recorder().is_none());
    record(snapshot.as_ref(), "snapshot-after-stop");
    assert_eq!(
        call(owner, first, "recorder.stats", json!({}))
            .await
            .unwrap()["completed"],
        2
    );
    call(owner, first, "recorder.flush", json!({}))
        .await
        .unwrap();
    call(owner, first, "handle.drop", json!({})).await.unwrap();
    assert_eq!(
        call(owner, first, "recorder.stats", json!({}))
            .await
            .unwrap_err()
            .code(),
        203
    );
    record(snapshot.as_ref(), "snapshot-after-drop");
    stalled_replacement(owner, second).await;
    HandleRegistry::global().release_owner(owner);
    recording::init_recording(previous);
}

#[tokio::test]
async fn owner_resolution_and_strict_controls_precede_side_effects() {
    let owner = Owner::new_node().unwrap();
    let foreign = Owner::new_python().unwrap();
    let handle = ring(owner).await;
    for name in [
        "recorder.attach",
        "recorder.stop",
        "recorder.flush",
        "recorder.stats",
        "recorder.export",
    ] {
        let foreign_error = call(foreign, handle, name, json!({})).await.unwrap_err();
        let unknown_error = call(foreign, HandleId(u64::MAX), name, json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            serde_json::to_value(foreign_error.envelope()).unwrap(),
            serde_json::to_value(unknown_error.envelope()).unwrap()
        );
        assert_eq!(foreign_error.code(), 203);
        assert_eq!(
            call(owner, handle, name, json!({"unknown":true}))
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    for params in [
        json!({"backend":{"type":"ring"}}),
        json!({"backend":{"type":"ring","unknown":true},"capturePolicy":"full"}),
        json!({"backend":{"type":"ring"},"capturePolicy":"unknown"}),
        json!({"backend":{"type":"ring"},"capturePolicy":"full","unknown":true}),
        json!({"backend":{"type":"ring","capacity":0},"capturePolicy":"full"}),
        json!({"backend":{"type":"ring","capacity":u64::MAX},"capturePolicy":"full"}),
    ] {
        assert_eq!(
            call(owner, HandleId(0), "recorder.create", params)
                .await
                .unwrap_err()
                .code(),
            301
        );
    }
    call(foreign, handle, "handle.drop", json!({}))
        .await
        .unwrap();
    assert_eq!(
        call(owner, handle, "recorder.stats", json!({}))
            .await
            .unwrap()["completed"],
        0
    );
    let wrong = HandleRegistry::global()
        .insert(
            owner,
            Entry::Abort(Arc::new(aimux_core::AbortSignal::new())),
        )
        .unwrap();
    assert_eq!(
        call(owner, wrong, "recorder.flush", json!({}))
            .await
            .unwrap_err()
            .code(),
        203
    );
    // Existing native recorder payloads still attach/flush; absent backend diagnostics are explicit.
    let native = HandleRegistry::global()
        .insert(owner, Entry::Recorder(Arc::new(RingRecorder::new())))
        .unwrap();
    call(owner, native, "recorder.flush", json!({}))
        .await
        .unwrap();
    assert_eq!(
        call(owner, native, "recorder.stats", json!({}))
            .await
            .unwrap_err()
            .code(),
        8
    );
    HandleRegistry::global().release_owner(owner);
}

fn directory() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "aimux-recording-dispatch-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

#[tokio::test]
async fn jsonl_dispatch_exports_real_data_and_preserves_recording_errors() {
    let owner = Owner::new_node().unwrap();
    let dir = directory();
    let value = call(
        owner,
        HandleId(0),
        "recorder.create",
        json!({
            "backend":{"type":"jsonl","directory":dir}, "capturePolicy":"full"
        }),
    )
    .await
    .unwrap();
    let handle: HandleId = serde_json::from_value(value["handle"].clone()).unwrap();
    let Entry::Recorder(recorder) = HandleRegistry::global()
        .get(owner, handle, &[HandleKind::Recorder])
        .unwrap()
    else {
        unreachable!()
    };
    record(recorder.as_ref(), "persisted");
    call(owner, handle, "recorder.flush", json!({}))
        .await
        .unwrap();
    let exported = call(owner, handle, "recorder.export", json!({}))
        .await
        .unwrap();
    let saved: Recording =
        serde_json::from_str(exported["jsonl"].as_str().unwrap().trim()).unwrap();
    assert_eq!(saved.call_id, "persisted");
    assert!(saved.complete);
    assert_eq!(
        std::fs::read_to_string(dir.join("recordings.jsonl")).unwrap(),
        exported["jsonl"]
    );
    std::fs::remove_file(dir.join("recordings.jsonl")).unwrap();
    let error = call(owner, handle, "recorder.export", json!({}))
        .await
        .unwrap_err();
    let envelope = error.envelope();
    assert_eq!(envelope.code, 105);
    assert_eq!(envelope.error["name"], "Write");
    drop(recorder);
    HandleRegistry::global().release_owner(owner);
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "occupied").unwrap();
    let error = call(
        owner,
        HandleId(0),
        "recorder.create",
        json!({
            "backend":{"type":"jsonl","directory":blocker.join("nested")}, "capturePolicy":"full"
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 100);
    let open = dir.join("open");
    std::fs::create_dir_all(open.join("recordings.jsonl")).unwrap();
    let error = call(
        owner,
        HandleId(0),
        "recorder.create",
        json!({
            "backend":{"type":"jsonl","directory":open}, "capturePolicy":"full"
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 101);
    std::fs::remove_dir_all(dir).unwrap();
}

struct StalledFlush {
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    completed: std::sync::atomic::AtomicBool,
}
impl Recorder for StalledFlush {
    fn record_input(&self, _: &str, _: &aimux_core::options::CallOptions, _: &str, _: &str) {}
    fn record_provider(&self, _: &str, _: &recording::ProviderRecord) {}
    fn record_exchange(&self, _: &str, _: &recording::HttpExchange) {}
    fn record_exchange_update(
        &self,
        _: &str,
        _: u32,
        _: u32,
        _: &recording::ResponseRecord,
        _: Option<String>,
    ) {
    }
    fn record_outcome(&self, _: &str, _: &recording::OutcomeRecord) {}
    fn flush(&self) {}
    fn try_flush(&self) -> Result<(), RecordingError> {
        self.started
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        self.completed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Err(RecordingError::FlushTimeout)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn stalled_flush_leaves_single_thread_runtime_and_registry_responsive() {
    let owner = Owner::new_node().unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let recorder = Arc::new(StalledFlush {
        started: Mutex::new(Some(started)),
        release: Mutex::new(wait),
        completed: std::sync::atomic::AtomicBool::new(false),
    });
    let handle = HandleRegistry::global()
        .insert(owner, Entry::Recorder(recorder.clone()))
        .unwrap();
    let task = tokio::spawn(call(owner, handle, "recorder.flush", json!({})));
    ready.await.unwrap();
    assert!(
        !recorder.completed.load(std::sync::atomic::Ordering::SeqCst),
        "flush blocked the runtime"
    );
    assert!(
        HandleRegistry::global()
            .get(owner, handle, &[HandleKind::Recorder])
            .is_ok()
    );
    let unrelated = ring(owner).await;
    assert!(unrelated.0 > handle.0);
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code(), 104);
    assert_eq!(error.envelope().error["name"], "FlushTimeout");
    HandleRegistry::global().release_owner(owner);
}

struct StalledDrop {
    started: Option<tokio::sync::oneshot::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
    completed: Arc<std::sync::atomic::AtomicBool>,
}
// The receiver is guarded for Sync without holding it during recorder lookup.
struct DropRecorder {
    _cleanup: Mutex<StalledDrop>,
}
impl Recorder for DropRecorder {
    fn record_input(&self, _: &str, _: &aimux_core::options::CallOptions, _: &str, _: &str) {}
    fn record_provider(&self, _: &str, _: &recording::ProviderRecord) {}
    fn record_exchange(&self, _: &str, _: &recording::HttpExchange) {}
    fn record_exchange_update(
        &self,
        _: &str,
        _: u32,
        _: u32,
        _: &recording::ResponseRecord,
        _: Option<String>,
    ) {
    }
    fn record_outcome(&self, _: &str, _: &recording::OutcomeRecord) {}
    fn flush(&self) {}
}
impl Drop for StalledDrop {
    fn drop(&mut self) {
        self.started.take().unwrap().send(()).unwrap();
        let _ = self.release.recv_timeout(Duration::from_secs(10));
        self.completed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
async fn stalled_replacement(owner: Owner, replacement: HandleId) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    recording::init_recording(Some(Arc::new(DropRecorder {
        _cleanup: Mutex::new(StalledDrop {
            started: Some(started),
            release: wait,
            completed: completed.clone(),
        }),
    })));
    let task = tokio::spawn(call(owner, replacement, "recorder.attach", json!({})));
    ready.await.unwrap();
    assert!(
        !completed.load(std::sync::atomic::Ordering::SeqCst),
        "replacement blocked the runtime"
    );
    let Entry::Recorder(expected) = HandleRegistry::global()
        .get(owner, replacement, &[HandleKind::Recorder])
        .unwrap()
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&recording::recorder().unwrap(), &expected));
    assert!(
        !completed.load(std::sync::atomic::Ordering::SeqCst),
        "replacement kept the state lock during cleanup"
    );
    release.send(()).unwrap();
    task.await.unwrap().unwrap();
    call(owner, replacement, "recorder.stop", json!({}))
        .await
        .unwrap();
}
