//! Replay / mock endpoints (RFC-0023 P3/P4, RFC-0029 Replay page).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::convert_to_language_model_prompt;
use aimux_core::message::ModelPrompt;
use aimux_core::recording::Recording;
use aimux_core::replay::{MockReplayModel, ReplayOverrides, replay_with_model};

use crate::api::err_response;
use crate::state::AppState;
use crate::wire::{self, WireMessage};

#[derive(Deserialize)]
pub struct ReplayRequest {
    pub call_id: String,
    /// Required for recordings whose key source is `explicit`/`unknown`
    /// (only `env:VAR` references accepted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<ReplayOverridesWire>,
}

#[derive(Deserialize)]
pub struct ReplayOverridesWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<WireMessage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[derive(Deserialize)]
pub struct MockLoadRequest {
    /// NDJSON recordings (RFC-0023 format).
    pub jsonl: String,
}

/// `POST /api/replay` — re-send a recorded call against the real API.
pub async fn run(State(state): State<AppState>, Json(req): Json<ReplayRequest>) -> Response {
    match run_inner(state, req).await {
        Ok(resp) => resp,
        Err(e) => err_response(e),
    }
}

async fn run_inner(state: AppState, req: ReplayRequest) -> Result<Response, AiMuxError> {
    let recording = state.recording(&req.call_id).ok_or_else(|| {
        AiMuxError::InvalidArgument(format!("recording '{}' not found", req.call_id))
    })?;

    // Resolve the key for provider rebuild (privacy: never plaintext).
    let key = match recording.provider.api_key_source.as_str() {
        s if s.starts_with("env:") => wire::resolve_api_key(Some(s))?,
        "none" => None,
        _ => wire::resolve_api_key(req.api_key.as_deref()).map_err(|_| {
            AiMuxError::InvalidArgument(
                "this recording has no env key source — pass api_key=\"env:VAR\" to replay".into(),
            )
        })?,
    };

    let model = aimux_providers::rebuild_provider(&recording.provider, key.as_deref())?;

    let mut overrides = ReplayOverrides {
        prompt: None,
        temperature: None,
        max_output_tokens: None,
    };
    if let Some(o) = &req.overrides {
        overrides.temperature = o.temperature;
        overrides.max_output_tokens = o.max_output_tokens;
        if let Some(msgs) = &o.messages {
            let ModelPrompt::Messages(list) = wire::to_model_prompt(msgs)? else {
                return Err(AiMuxError::InvalidArgument(
                    "prompt override must be a message list".into(),
                ));
            };
            overrides.prompt = Some(convert_to_language_model_prompt(&list, None));
        }
    }

    let result = replay_with_model(&recording, &*model, Some(&overrides)).await?;
    Ok(Json(json!({
        "call_id": recording.call_id,
        "text": result.text,
        "finish_reason": serde_json::to_value(result.finish_reason.unified).unwrap_or_default(),
        "usage": serde_json::to_value(&result.usage).unwrap_or_default(),
        "tool_calls": serde_json::to_value(&result.tool_calls).unwrap_or_default(),
        "meta": state.last_meta(recording.session_id.as_deref()),
    }))
    .into_response())
}

/// `POST /api/mock/load` — load NDJSON recordings into a `MockReplayModel`
/// and switch the console to offline mock mode (no real API calls).
pub async fn mock_load(
    State(state): State<AppState>,
    Json(req): Json<MockLoadRequest>,
) -> Response {
    match mock_load_inner(state, req) {
        Ok(resp) => resp,
        Err(e) => err_response(e),
    }
}

fn mock_load_inner(state: AppState, req: MockLoadRequest) -> Result<Response, AiMuxError> {
    let mut recordings: Vec<Recording> = Vec::new();
    for (idx, line) in req.jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec: Recording = serde_json::from_str(line)
            .map_err(|e| AiMuxError::JsonParse(format!("mock load line {}: {e}", idx + 1)))?;
        recordings.push(rec);
    }
    if recordings.is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "mock load: empty recording data".into(),
        ));
    }
    let provider = recordings[0].provider.provider.clone();
    let model_id = recordings[0].provider.model_id.clone();
    let mock = Arc::new(MockReplayModel::new(
        provider.clone(),
        model_id.clone(),
        recordings,
    ));
    *state.mock_model.lock().unwrap() = Some(mock);
    Ok(Json(json!({
        "loaded": true,
        "provider": provider,
        "model": model_id,
    }))
    .into_response())
}
