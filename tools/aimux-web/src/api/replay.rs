//! Replay / mock endpoints (RFC-0023 P3/P4, RFC-0029 Replay page).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
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
    /// Fallback for recordings whose key source is `explicit`/`unknown`:
    /// `env:VAR` reference, or a plaintext literal on loopback binds
    /// (RFC-0029 §5.5). Keys saved in Settings apply as well.
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

    // Resolve the key for provider rebuild: the recording's own env: source
    // first, then the request spec, with the Settings key store as the
    // fallback (RFC-0029 §5.5).
    let provider_name = recording.provider.provider.clone();
    let key = match recording.provider.api_key_source.as_str() {
        s if s.starts_with("env:") => {
            wire::resolve_api_key(Some(s), &provider_name, &state.keys, state.loopback)?
        }
        "none" => wire::resolve_api_key(None, &provider_name, &state.keys, state.loopback)?,
        _ => wire::resolve_api_key(
            req.api_key.as_deref(),
            &provider_name,
            &state.keys,
            state.loopback,
        )
        .map_err(|_| {
            AiMuxError::InvalidArgument(
                "this recording has no env key source — pass api_key=\"env:VAR\" or save the \
                 provider key in Settings to replay"
                    .into(),
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

    // Group recordings by (provider, model_id): each model gets its own
    // `MockReplayModel` (a mock model is bound to one provider/model).
    let mut grouped: std::collections::HashMap<(String, String), Vec<Recording>> =
        std::collections::HashMap::new();
    for rec in recordings {
        grouped
            .entry((rec.provider.provider.clone(), rec.provider.model_id.clone()))
            .or_default()
            .push(rec);
    }

    let mut models: std::collections::HashMap<String, Arc<dyn LanguageModel>> =
        std::collections::HashMap::new();
    for ((provider, model_id), recs) in grouped {
        let key = format!("{provider}/{model_id}");
        let mock = Arc::new(MockReplayModel::new(provider, model_id, recs));
        models.insert(key, mock);
    }
    *state.mock_models.lock().unwrap() = models.clone();

    let mut loaded: Vec<serde_json::Value> = Vec::new();
    for (provider, model_id) in models.keys().map(|k| {
        let (p, m) = k.split_once('/').unwrap_or((k, ""));
        (p.to_string(), m.to_string())
    }) {
        loaded.push(json!({ "provider": provider, "model": model_id }));
    }
    Ok(Json(json!({
        "loaded": true,
        "models": loaded,
    }))
    .into_response())
}
