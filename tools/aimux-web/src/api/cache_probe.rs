//! Cache probe endpoint — online provider prefix-caching probe (RFC-0029
//! CacheProbe page), logic translated from `aimux-cli probe::provider`.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text};
use aimux_core::message::{ModelMessage, ModelPrompt};
use aimux_core::trace::{TraceFilter, TraceLayer};

use crate::api::err_response;
use crate::model_builder;
use crate::state::{AppState, WebTraceSink};
use crate::wire;

#[derive(Deserialize)]
pub struct ProbeRequest {
    pub provider: String,
    pub model: String,
    /// `None` (Settings key store / provider env var) / `env:VAR`, or a
    /// plaintext literal on loopback binds (RFC-0029 §5.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default = "default_max_requests")]
    pub max_requests: usize,
    /// Override the default probe system template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Report what would be sent without calling the API.
    #[serde(default)]
    pub dry_run: bool,
}

fn default_max_requests() -> usize {
    4
}

/// Probe rounds share this session id so the store's session filters isolate them.
const PROBE_SESSION: &str = "cache-probe";

/// `POST /api/cache-probe` — run a cache probe and return per-round results,
/// aggregated stats and the raw trace records.
pub async fn run(State(state): State<AppState>, Json(req): Json<ProbeRequest>) -> Response {
    match run_inner(state, req).await {
        Ok(resp) => resp,
        Err(e) => err_response(e),
    }
}

async fn run_inner(state: AppState, req: ProbeRequest) -> Result<Response, AiMuxError> {
    if req.max_requests == 0 {
        return Err(AiMuxError::InvalidArgument(
            "max_requests must be >= 1".into(),
        ));
    }

    // Dry-run must not require an API key — it only reports the plan.
    if req.dry_run {
        return Ok(Json(json!({
            "dry_run": true,
            "provider": req.provider,
            "model": req.model,
            "rounds": req.max_requests,
            "session": PROBE_SESSION,
            "note": "no real API call was made",
        }))
        .into_response());
    }

    let key = wire::resolve_api_key(
        req.api_key.as_deref(),
        &req.provider,
        &state.keys,
        state.loopback,
    )?;
    let model =
        model_builder::build_model(&req.provider, key, &req.model, req.base_url.as_deref())?;

    // Fresh sink per probe run (like the CLI): a shared ring would let prior
    // runs' history pollute round-1 lookups.
    let sink = Arc::new(WebTraceSink::new());
    let traced = Arc::new(TraceLayer::new(model.clone(), sink.clone()).with_rules_auditor(true));

    let system = req.prompt.clone().unwrap_or_else(default_system);
    let rounds: Vec<Vec<ModelMessage>> = (0..req.max_requests)
        .map(|i| {
            let mut msgs = vec![
                ModelMessage::system(system.clone()),
                ModelMessage::user(format!("Question {i}: what is the capital of Atlantis?")),
            ];
            for j in 0..i {
                msgs.push(ModelMessage::assistant(format!(
                    "The capital of Atlantis is Poseidonia (round {j})."
                )));
                msgs.push(ModelMessage::user(format!(
                    "Follow-up {i}.{j}: how deep is the canal?"
                )));
            }
            msgs
        })
        .collect();

    let mut per_round = Vec::new();
    for (i, msgs) in rounds.iter().enumerate() {
        let options = GenerateTextOptions {
            session_id: Some(PROBE_SESSION.to_string()),
            max_output_tokens: Some(32),
            ..Default::default()
        };
        let started = Instant::now();
        let result = generate_text(&*traced, ModelPrompt::Messages(msgs.clone()), options).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(r) => per_round.push(json!({
                "round": i,
                "cache_read_tokens": r.usage.input_tokens.cache_read.unwrap_or(0),
                "input_total_tokens": r.usage.input_tokens.total,
                "output_tokens": r.usage.output_tokens.total,
                "elapsed_ms": elapsed_ms,
                "text_preview": r.text.chars().take(80).collect::<String>(),
            })),
            Err(e) => per_round.push(json!({ "round": i, "error": e.to_string() })),
        }
    }

    let stats = sink.inner().aggregate(&TraceFilter {
        provider: Some(req.provider.clone()),
        model: None,
        session_id: Some(PROBE_SESSION.to_string()),
        since_unix_ms: None,
    });

    Ok(Json(json!({
        "provider": req.provider,
        "model": req.model,
        "rounds": per_round,
        "stats": stats,
        "records": sink.records(),
    }))
    .into_response())
}

/// Deterministic, cache-friendly system template (long stable prefix so
/// provider-side prompt caching can engage from round 2 on).
fn default_system() -> String {
    const BODY: &str = "The quick brown fox jumps over the lazy dog. Pack my box with five \
        dozen liquor jugs. Sphinx of black quartz, judge my vow. How vexingly quick daft \
        zebras jump. The five boxing wizards jump quickly. A mad boxer shot a quick gloved \
        jab to the jaw of his dizzy opponent. Two driven jocks help fax my big quiz. ";
    let mut s = String::from(
        "You are a careful technical analyst. Keep answers precise and cite numbers.\nContext: ",
    );
    for _ in 0..30 {
        s.push_str(BODY);
    }
    s.push_str(
        "\n\nRemember to verify cache hits by comparing claimed tokens against the client-side \
         prefix length; a stable prefix is the key to reliable caching.",
    );
    s
}
