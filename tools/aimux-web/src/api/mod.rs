//! HTTP API routes (RFC-0029 §5).

pub mod cache_probe;
pub mod calls;
pub mod providers;
pub mod replay;
pub mod sessions;
pub mod settings;
pub mod tools;
pub mod traces;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use aimux_core::error::AiMuxError;

use crate::state::AppState;

/// Build the API router (all routes under `/api`).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/calls", axum::routing::post(calls::call))
        .route("/api/tools/{name}", axum::routing::post(tools::run))
        .route("/api/tools", get(tools::list))
        .route("/api/traces", get(traces::list))
        .route("/api/traces/{call_id}", get(traces::detail))
        .route("/api/trace-records", get(traces::records))
        .route("/api/recordings/export", get(traces::export_jsonl))
        .route(
            "/api/recordings/import",
            axum::routing::post(traces::import_jsonl),
        )
        .route("/api/sessions", get(sessions::list))
        .route("/api/sessions/{id}", get(sessions::detail))
        .route("/api/replay", axum::routing::post(replay::run))
        .route("/api/mock/load", axum::routing::post(replay::mock_load))
        .route("/api/cache-probe", axum::routing::post(cache_probe::run))
        .route("/api/providers", get(providers::list))
        .route(
            "/api/settings/keys",
            get(settings::list_keys).put(settings::put_key),
        )
        .route(
            "/api/settings/keys/{provider}",
            axum::routing::delete(settings::delete_key),
        )
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

/// Render an `AiMuxError` as a JSON error response with a sensible status.
pub fn err_response(e: AiMuxError) -> Response {
    let status = match &e {
        AiMuxError::InvalidArgument(_)
        | AiMuxError::InvalidPrompt(_)
        | AiMuxError::NoSuchProvider { .. }
        | AiMuxError::NoSuchModel { .. } => StatusCode::BAD_REQUEST,
        AiMuxError::UnsupportedFunctionality(_) => StatusCode::NOT_IMPLEMENTED,
        AiMuxError::Timeout(_) | AiMuxError::Aborted(_) => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        axum::Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}
