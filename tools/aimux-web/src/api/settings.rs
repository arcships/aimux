//! `GET/PUT/DELETE /api/settings/keys` — console credential settings
//! (RFC-0029 §5.5). Plaintext keys are stored server-side only and never
//! appear in responses: `GET` returns a masked listing (last 4 chars).
//!
//! Loopback gating: `PUT`/`DELETE` return 403 when the server is bound to a
//! non-loopback host (any LAN client could otherwise plant or exfiltrate
//! keys); `GET` still lists hints but reports `plaintext_entry: false` so
//! the frontend hides the input and points users at `env:` references.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct PutKeyRequest {
    pub provider: String,
    pub key: String,
    /// Persist to the config-dir `keys.json` (0600) in addition to memory.
    #[serde(default)]
    pub remember: bool,
}

/// `GET /api/settings/keys` — masked listing + whether plaintext entry is
/// allowed (loopback binding only).
pub async fn list_keys(State(state): State<AppState>) -> Response {
    let keys: Vec<_> = state
        .keys
        .hints()
        .into_iter()
        .map(|h| {
            json!({
                "provider": h.provider,
                "status": "stored",
                "hint": h.hint,
                "remembered": h.remembered,
            })
        })
        .collect();
    Json(json!({
        "keys": keys,
        "plaintext_entry": state.loopback,
    }))
    .into_response()
}

/// `PUT /api/settings/keys` — save one provider key (memory + optional disk).
pub async fn put_key(State(state): State<AppState>, Json(req): Json<PutKeyRequest>) -> Response {
    if !state.loopback {
        return non_loopback_forbidden();
    }
    let provider = req.provider.trim().to_string();
    if provider.is_empty() || req.key.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "provider and key must be non-empty" })),
        )
            .into_response();
    }
    match state.keys.set(&provider, &req.key, req.remember) {
        Ok(remembered) => Json(json!({
            "provider": provider,
            "stored": true,
            "remembered": remembered,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("key stored in memory, but disk persistence failed: {e}") })),
        )
            .into_response(),
    }
}

/// `DELETE /api/settings/keys/{provider}` — drop the key (memory + disk).
pub async fn delete_key(State(state): State<AppState>, Path(provider): Path<String>) -> Response {
    if !state.loopback {
        return non_loopback_forbidden();
    }
    match state.keys.remove(&provider) {
        Ok(removed) => Json(json!({ "provider": provider, "removed": removed })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to remove persisted key: {e}") })),
        )
            .into_response(),
    }
}

/// 403 with guidance back to `env:` references.
fn non_loopback_forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "key management is disabled: the console is bound to a non-loopback address — \
                      keep using api_key=\"env:VAR\" references or the provider's registered env var"
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use tower::ServiceExt;

    use super::*;
    use crate::api::router;
    use crate::settings::KeyStore;

    fn app(host: &str, keys: KeyStore) -> axum::Router {
        router(AppState::with_bind_host_and_store(host, keys))
    }

    async fn body(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn put_get_delete_round_trip_on_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let keys = KeyStore::from_path(Some(dir.path().join("keys.json")));
        let app = app("127.0.0.1", keys);

        // PUT (remember) → stored + remembered.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::put("/api/settings/keys")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"provider":"openai","key":"sk-secret-abcd","remember":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b.contains("\"remembered\":true"), "body: {b}");

        // GET → masked hint, plaintext never leaks.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/settings/keys")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b.contains("\"plaintext_entry\":true"), "body: {b}");
        assert!(b.contains("…abcd"), "masked hint missing: {b}");
        assert!(!b.contains("sk-secret-abcd"), "plaintext leaked: {b}");

        // DELETE → removed, listing empty.
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::delete("/api/settings/keys/openai")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app
            .oneshot(
                axum::http::Request::get("/api/settings/keys")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let b = body(resp).await;
        assert!(!b.contains("openai"), "provider still listed: {b}");
    }

    #[tokio::test]
    async fn non_loopback_rejects_put_and_delete_but_lists_masked() {
        let dir = tempfile::tempdir().unwrap();
        let keys = KeyStore::from_path(Some(dir.path().join("keys.json")));
        keys.set("anthropic", "sk-lan-secret-777", false).unwrap();
        let app = app("0.0.0.0", keys);

        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::put("/api/settings/keys")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        r#"{"provider":"openai","key":"sk-x","remember":false}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let b = body(resp).await;
        assert!(b.contains("env:"), "403 must guide to env: refs: {b}");

        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::delete("/api/settings/keys/anthropic")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // GET still works: masked + plaintext_entry:false.
        let resp = app
            .oneshot(
                axum::http::Request::get("/api/settings/keys")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body(resp).await;
        assert!(b.contains("\"plaintext_entry\":false"), "body: {b}");
        assert!(b.contains("…-777"), "masked hint missing: {b}");
        assert!(!b.contains("sk-lan-secret-777"), "plaintext leaked: {b}");
    }

    #[tokio::test]
    async fn put_validates_empty_fields() {
        let app = app("localhost", KeyStore::from_path(None));
        let resp = app
            .oneshot(
                axum::http::Request::put("/api/settings/keys")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"provider":"  ","key":"sk-x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
