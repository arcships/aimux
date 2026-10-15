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
    // key store > provider's registered env var (RFC-0029 §5.5). A custom
    // base_url gets only a key carried in the request.
    let key = wire::resolve_api_key(
        req.api_key.as_deref(),
        &req.provider,
        &state.keys,
        state.loopback,
        model_builder::is_custom_base_url(&req.provider, req.base_url.as_deref()),
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
                text: r.text(),
                finish_reason: serde_json::to_value(r.finish_reason()).unwrap_or_default(),
                usage: serde_json::to_value(r.usage()).unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    use crate::api::router;
    use crate::settings::KeyStore;
    use crate::state::AppState;

    const CHAT: &str = r#"{"id":"chatcmpl-1","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

    /// A one-endpoint OpenAI-compatible server that keeps every raw request.
    async fn capture_server() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        body.len() >= length
                    });
                    if n == 0 || complete {
                        break;
                    }
                }
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw).to_string());
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{CHAT}",
                    CHAT.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (url, seen)
    }

    async fn call(app: axum::Router, body: serde_json::Value) -> (StatusCode, String) {
        let resp = app
            .oneshot(
                axum::http::Request::post("/api/calls")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn request(provider: &str, base_url: &str, api_key: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "provider": provider,
            "model": "m",
            "api_key": api_key,
            "base_url": base_url,
            "stream": false,
            "options": {"max_retries": 0},
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
        })
    }

    /// A caller-supplied base_url never receives the Settings key or the
    /// provider's environment key: with no key in the request, the call goes
    /// out without one.
    #[tokio::test]
    async fn custom_base_url_gets_no_stored_or_env_key() {
        // A registry-style provider whose key variable this test owns.
        aimux_providers::load_providers_from_json(
            r#"{"providers":[{"name":"web-test-custom-base","base_url":"https://example.invalid/v1","env_var":"AIMUX_WEB_TEST_CUSTOM_BASE_KEY"}]}"#,
        )
        .unwrap();
        // SAFETY: This test owns a unique process-wide env key and removes it below.
        unsafe { std::env::set_var("AIMUX_WEB_TEST_CUSTOM_BASE_KEY", "env-secret-7") };

        let keys = KeyStore::from_path(None);
        keys.set("openai", "sk-stored-secret-1", false).unwrap();
        keys.set("web-test-custom-base", "sk-stored-secret-2", false)
            .unwrap();
        let app = router(AppState::with_bind_host_and_store("127.0.0.1", keys));
        let (url, seen) = capture_server().await;

        for provider in ["openai", "web-test-custom-base"] {
            let (status, body) = call(app.clone(), request(provider, &url, None)).await;
            assert_eq!(status, StatusCode::OK, "{provider}: {body}");
        }
        // An env: reference is refused outright for a custom base_url.
        let (status, body) = call(
            app.clone(),
            request(
                "web-test-custom-base",
                &url,
                Some("env:AIMUX_WEB_TEST_CUSTOM_BASE_KEY"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        // A key carried in the request (loopback) is what the endpoint gets.
        let (status, body) = call(app, request("openai", &url, Some("sk-for-this-endpoint"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // SAFETY: Cleanup of the unique test key set above.
        unsafe { std::env::remove_var("AIMUX_WEB_TEST_CUSTOM_BASE_KEY") };

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "the refused call must not reach the server");
        for raw in seen.iter() {
            for secret in ["sk-stored-secret-1", "sk-stored-secret-2", "env-secret-7"] {
                assert!(!raw.contains(secret), "{secret} leaked: {raw}");
            }
        }
        assert!(seen[2].contains("sk-for-this-endpoint"), "{}", seen[2]);
    }
}
