//! Process-wide state: recording / trace / session wiring (RFC-0029 §4).
//!
//! Wires the global recording (`RingRecorder`), the shared trace sink
//! (`WebTraceSink` → `RingTraceStore` + side list) and the session store at
//! startup, exactly like the RFC's `state.rs` sketch. Every model built by
//! the console is wrapped in a `TraceLayer`, so each call produces both a
//! `Recording` (RFC-0023) and a `TraceRecord` (RFC-0015).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use aimux_core::language_model::LanguageModel;
use aimux_core::recording::{Recording, RingRecorder, init_recording};
use aimux_core::session::{SessionStore, init_session_store};
use aimux_core::trace::{RingTraceStore, TraceLayer, TraceRecord, TraceSink};

/// Bounded side list of `TraceRecord`s for the console UI.
const TRACE_RECORD_CAP: usize = 4096;

/// A `TraceSink` that forwards every record into a `RingTraceStore` (LCP +
/// cache verdicts) while keeping a bounded list of `TraceRecord`s for listing.
///
/// `as_any` exposes the inner `RingTraceStore` — `TraceLayer` reaches the LCP
/// index via `sink.as_any().downcast_ref::<RingTraceStore>()`.
pub struct WebTraceSink {
    inner: Arc<RingTraceStore>,
    records: Arc<Mutex<VecDeque<TraceRecord>>>,
}

impl WebTraceSink {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RingTraceStore::new()),
            records: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// The underlying `RingTraceStore` (LCP / cache logic).
    pub fn inner(&self) -> Arc<RingTraceStore> {
        self.inner.clone()
    }

    /// All probed trace records, oldest first.
    pub fn records(&self) -> Vec<TraceRecord> {
        self.records.lock().unwrap().iter().cloned().collect()
    }
}

impl Default for WebTraceSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceSink for WebTraceSink {
    fn record(&self, rec: TraceRecord) {
        self.inner.record(rec.clone());
        let mut r = self.records.lock().unwrap();
        if r.len() >= TRACE_RECORD_CAP {
            r.pop_front();
        }
        r.push_back(rec);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.inner.as_any()
    }
}

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    /// Global recorder (RFC-0023): every call lands here.
    pub recorder: Arc<RingRecorder>,
    /// Shared trace sink (RFC-0015): cache probing + trace record list.
    pub trace_sink: Arc<WebTraceSink>,
    /// Session grouping (RFC-0024).
    pub session_store: Arc<SessionStore>,
    /// Loaded `MockReplayModel` (offline mock mode, RFC-0023 P3).
    pub mock_model: Arc<Mutex<Option<Arc<dyn LanguageModel>>>>,
    /// Recordings imported from jsonl (RFC-0023), merged into listings.
    pub imported: Arc<Mutex<Vec<Recording>>>,
}

impl AppState {
    pub fn new() -> Self {
        let recorder = Arc::new(RingRecorder::new());
        let trace_sink = Arc::new(WebTraceSink::new());
        let session_store = Arc::new(SessionStore::new());

        // Wire the opt-in globals (no-op cost when disabled elsewhere).
        init_recording(Some(recorder.clone()));
        init_session_store(session_store.clone());

        Self {
            recorder,
            trace_sink,
            session_store,
            mock_model: Arc::new(Mutex::new(None)),
            imported: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Wrap a model in a `TraceLayer` bound to the shared sink, so every call
    /// is both recorded (RFC-0023) and probed (RFC-0015).
    pub fn traced(&self, model: Arc<dyn LanguageModel>) -> Arc<dyn LanguageModel> {
        Arc::new(TraceLayer::new(model, self.trace_sink.clone()))
    }

    /// All completed recordings: in-ring plus imported (imported appended).
    pub fn all_recordings(&self) -> Vec<Recording> {
        let mut recs = self.recorder.completed();
        recs.extend(self.imported.lock().unwrap().iter().cloned());
        recs
    }

    /// The single recording for a `call_id` (ring first, then imported).
    pub fn recording(&self, call_id: &str) -> Option<Recording> {
        self.recorder.get(call_id).or_else(|| {
            self.imported
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.call_id == call_id)
                .cloned()
        })
    }

    /// Best-effort metadata for the call just completed: the newest completed
    /// recording, optionally narrowed by session id.
    pub fn last_meta(&self, session_id: Option<&str>) -> Option<crate::wire::WireMeta> {
        let recs = self.recorder.completed();
        let rec = recs
            .iter()
            .rev()
            .find(|r| session_id.is_none_or(|s| r.session_id.as_deref() == Some(s)))
            .or_else(|| recs.last())?;
        Some(crate::wire::WireMeta {
            call_id: rec.call_id.clone(),
            session_id: rec.session_id.clone(),
            step: rec.step,
            outcome: format!("{:?}", rec.outcome.status).to_lowercase(),
        })
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
