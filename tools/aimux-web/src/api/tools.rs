//! `POST /api/tools/:name` — built-in tool execution (RFC-0029 §6.3).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;

use crate::agents;
use crate::api::err_response;
use crate::state::AppState;

/// Execute a built-in tool and return `{tool, result}`.
pub async fn run(
    State(_state): State<AppState>,
    Path(name): Path<String>,
    Json(input): Json<Value>,
) -> Response {
    match agents::execute(&name, &input) {
        Ok(result) => Json(json!({ "tool": name, "result": result })).into_response(),
        Err(e) => err_response(AiMuxError::InvalidArgument(e)),
    }
}

/// `GET /api/tools` — the built-in tool schemas (JSON Schema), for the Agent
/// page's tool picker and for injecting into model calls.
pub async fn list() -> Response {
    Json(json!({ "tools": agents::tool_schemas() })).into_response()
}
