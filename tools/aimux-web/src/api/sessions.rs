//! Session endpoints (RFC-0024): grouped call chains.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::state::AppState;

/// `GET /api/sessions` — all known sessions (id, source, calls).
pub async fn list(State(state): State<AppState>) -> Response {
    Json(state.session_store.list_sessions()).into_response()
}

/// `GET /api/sessions/:id` — one session's calls plus its recordings.
pub async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let calls = state.session_store.session_calls(&id);
    let recordings: Vec<_> = state
        .all_recordings()
        .into_iter()
        .filter(|r| r.session_id.as_deref() == Some(id.as_str()))
        .collect();
    Json(json!({
        "session_id": id,
        "calls": calls,
        "recordings": recordings,
    }))
    .into_response()
}
