//! Trace endpoints: recording list/detail, trace records, jsonl export/import
//! (RFC-0029 §5.1).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::recording::Recording;

use crate::api::err_response;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    pub provider: Option<String>,
    pub session: Option<String>,
    pub status: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/traces` — recordings (newest first) with optional filters.
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let mut recs = state.all_recordings();
    if let Some(p) = &q.provider {
        recs.retain(|r| r.provider.provider == *p);
    }
    if let Some(s) = &q.session {
        recs.retain(|r| r.session_id.as_deref() == Some(s.as_str()));
    }
    if let Some(st) = &q.status {
        recs.retain(|r| format!("{:?}", r.outcome.status).to_lowercase() == *st);
    }
    recs.reverse();
    if let Some(n) = q.limit {
        recs.truncate(n);
    }
    Json(recs).into_response()
}

/// `GET /api/traces/:call_id` — one recording (three-layer detail).
pub async fn detail(State(state): State<AppState>, Path(call_id): Path<String>) -> Response {
    match state.recording(&call_id) {
        Some(rec) => Json(rec).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("recording '{call_id}' not found") })),
        )
            .into_response(),
    }
}

/// `GET /api/trace-records` — cache probe records (RFC-0015 verdicts).
pub async fn records(State(state): State<AppState>) -> Response {
    Json(state.trace_sink.records()).into_response()
}

/// `GET /api/recordings/export` — all recordings as NDJSON.
pub async fn export_jsonl(State(state): State<AppState>) -> Response {
    let mut out = String::new();
    for rec in state.all_recordings() {
        if let Ok(line) = serde_json::to_string(&rec) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    ([(header::CONTENT_TYPE, "application/x-ndjson")], out).into_response()
}

/// `POST /api/recordings/import` — append recordings from NDJSON body.
pub async fn import_jsonl(State(state): State<AppState>, body: String) -> Response {
    let mut count = 0usize;
    for (idx, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Recording>(line) {
            Ok(rec) => {
                state.imported.lock().unwrap().push(rec);
                count += 1;
            }
            Err(e) => {
                return err_response(AiMuxError::JsonParse(format!(
                    "import line {}: {e}",
                    idx + 1
                )));
            }
        }
    }
    Json(json!({ "imported": count })).into_response()
}
