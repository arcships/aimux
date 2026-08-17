//! `POST /api/calls` — one model call (generate or stream, RFC-0029 §5).

use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;

use crate::api::err_response;
use crate::model_builder;
use crate::state::AppState;
use crate::wire::{self, WireCallRequest, WireCallResponse};

/// `POST /api/calls` — validates the wire request, builds (or reuses) the
/// model, and returns either a non-streaming JSON result or an SSE stream of
/// `StreamPart` events + a final `meta` event (trace anchor).
pub async fn call(State(state): State<AppState>, Json(req): Json<WireCallRequest>) -> Response {
    match run(state, req).await {
        Ok(resp) => resp,
        Err(e) => err_response(e),
    }
}

async fn run(state: AppState, req: WireCallRequest) -> Result<Response, AiMuxError> {
    // api_key: explicit spec (env: ref, or plaintext on loopback) > Settings
    // key store > provider's registered env var (RFC-0029 §5.5).
    let key = wire::resolve_api_key(
        req.api_key.as_deref(),
        &req.provider,
        &state.keys,
        state.loopback,
    )?;

    let model: Arc<dyn LanguageModel> = if req.mock {
        let key = format!("{}/{}", req.provider, req.model);
        state
            .mock_models
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                AiMuxError::InvalidArgument(format!(
                    "mock mode requested for '{key}' but no recordings are loaded for it — \
                     POST /api/mock/load first (recordings must include this provider/model)"
                ))
            })?
    } else {
        let m =
            model_builder::build_model(&req.provider, key, &req.model, req.base_url.as_deref())?;
        state.traced(m)
    };

    let options = wire::to_generate_options(&req.options, req.session_id.as_deref())?;
    let prompt = wire::to_model_prompt(&req.messages)?;
    let session_id = req.session_id.clone();

    if req.stream {
        Ok(stream_response(state, model, prompt, options, session_id.as_deref()).await)
    } else {
        non_stream_response(state, model, prompt, options, session_id.as_deref()).await
    }
}

/// Non-streaming: JSON `{text, finish_reason, usage, meta}`.
async fn non_stream_response(
    state: AppState,
    model: Arc<dyn LanguageModel>,
    prompt: ModelPrompt,
    options: GenerateTextOptions,
    session_id: Option<&str>,
) -> Result<Response, AiMuxError> {
    match generate_text(&*model, prompt, options).await {
        Ok(r) => {
            let meta = state.last_meta(session_id);
            let resp = WireCallResponse {
                text: r.text,
                finish_reason: serde_json::to_value(r.finish_reason.unified).unwrap_or_default(),
                usage: serde_json::to_value(&r.usage).unwrap_or_default(),
                meta,
                error: None,
            };
            Ok(Json(resp).into_response())
        }
        Err(e) => Err(e),
    }
}

/// Streaming: SSE of `stream_part` events followed by `meta`.
async fn stream_response(
    state: AppState,
    model: Arc<dyn LanguageModel>,
    prompt: ModelPrompt,
    options: GenerateTextOptions,
    session_id: Option<&str>,
) -> Response {
    match stream_text(&*model, prompt, options).await {
        Ok(mut result) => {
            let state = state.clone();
            let session_id = session_id.map(str::to_string);
            let stream = async_stream::stream! {
                while let Some(item) = result.stream.next().await {
                    match item {
                        Ok(part) => {
                            let data = serde_json::to_string(&part).unwrap_or_else(|_| "{}".into());
                            yield Ok::<Event, Infallible>(
                                Event::default().event("stream_part").data(data),
                            );
                        }
                        Err(e) => {
                            yield Ok::<Event, Infallible>(
                                Event::default().event("error").data(e.to_string()),
                            );
                        }
                    }
                }
                // Trace anchor: the newest completed recording for this call.
                if let Some(meta) = state.last_meta(session_id.as_deref()) {
                    let data = serde_json::to_string(&meta).unwrap_or_default();
                    yield Ok::<Event, Infallible>(Event::default().event("meta").data(data));
                }
            };
            Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response()
        }
        Err(e) => err_response(e),
    }
}
