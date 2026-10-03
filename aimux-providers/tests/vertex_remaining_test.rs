//! Remaining Google Vertex AI provider tests — ported from the TS SDK suite.
//!
//! Mirrors `reference/ai/packages/google-vertex/src/google-vertex-provider.test.ts`
//! (provider configuration: auth headers, Express-mode API key, base-URL
//! override, tuned-model restrictions, project/location/token resolution).
//!
//! The TS tests mock `createAuthTokenGenerator` / `createGoogleVertex` and assert
//! on the options passed to the base provider. aimux has no Application Default
//! Credentials: the bearer token is the `access_token` setting (any
//! `Resolvable`, so a host can refresh it) or `GOOGLE_VERTEX_ACCESS_TOKEN`, so
//! each TS scenario is translated to the equivalent observable behaviour: which
//! headers are sent, which mode a request uses, and when settings are loaded.
//! Request URLs and the three host shapes are covered by
//! `vertex_factory_test.rs`.
//!
//! Model-level generate/stream behaviour is already covered by
//! `vertex_model_test.rs`; this file focuses on provider configuration only.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_provider_utils::Resolvable;
use aimux_providers::vertex::{VertexProvider, VertexProviderSettings, create_google_vertex};

// ── Shared helpers ───────────────────────────────────────────────────────────

/// All environment variables a Vertex request may consult.
const ENV_VARS: &[&str] = &[
    "GOOGLE_VERTEX_API_KEY",
    "GOOGLE_VERTEX_ACCESS_TOKEN",
    "GOOGLE_VERTEX_PROJECT",
    "GOOGLE_VERTEX_LOCATION",
];

/// Remove every Vertex env var (test isolation for `#[serial]` env tests).
fn clear_vertex_env() {
    for var in ENV_VARS {
        unsafe {
            std::env::remove_var(var);
        }
    }
}

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn vertex(settings: VertexProviderSettings) -> VertexProvider {
    create_google_vertex(settings).expect("valid settings")
}

/// Standard mode (bearer token) against `base_url`.
fn bearer_at(token: &str, base_url: String) -> VertexProvider {
    vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value(token.to_string())),
        project: Some("my-project".to_string()),
        location: Some("us-central1".to_string()),
        base_url: Some(base_url),
        ..Default::default()
    })
}

fn ok_body() -> serde_json::Value {
    json!({
        "candidates": [{
            "content": { "parts": [{ "text": "hi" }], "role": "model" },
            "finishReason": "STOP", "index": 0
        }]
    })
}

/// Mount a minimal 200 generateContent mock on the standard model path.
async fn mock_ok(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(server)
        .await;
}

// ════════════════════════════════════════════════════════════════════════════
// Provider identity  (TS: google-vertex-provider.test.ts — base wiring)
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn model_provider_is_google_vertex() {
    let provider = vertex(VertexProviderSettings::default());
    assert_eq!(
        provider.chat("gemini-2.0-flash").provider(),
        "google.vertex"
    );
}

#[tokio::test]
async fn language_model_trait_method_returns_boxed_model() {
    let provider = vertex(VertexProviderSettings::default());
    let model = provider.language_model("gemini-2.0-flash").expect("model");
    assert_eq!(model.model_id(), "gemini-2.0-flash");
    assert_eq!(model.provider(), "google.vertex");
}

/// TS: "creates the auth token generator once per provider instance" — a single
/// provider instance can mint multiple models that all share its settings.
#[tokio::test]
async fn one_provider_instance_creates_multiple_models() {
    let provider = vertex(VertexProviderSettings::default());

    let m1 = provider.chat("gemini-2.0-flash");
    let m2 = provider.chat("gemini-2.5-pro");

    assert_eq!(m1.model_id(), "gemini-2.0-flash");
    assert_eq!(m2.model_id(), "gemini-2.5-pro");
    assert_eq!(m1.provider(), "google.vertex");
    assert_eq!(m2.provider(), "google.vertex");
}

/// Every modality reports its own provider string: `google.vertex` for
/// language, embedding and image models, `google.vertex.{method}` for the rest.
#[tokio::test]
async fn provider_strings_by_modality() {
    use aimux_core::embedding_model::EmbeddingModel;
    use aimux_core::image_model::ImageModel;
    use aimux_core::transcription_model::TranscriptionModel;
    use aimux_core::video_model::VideoModel;

    let provider = vertex(VertexProviderSettings::default());
    assert_eq!(
        provider.embedding("textembedding-gecko@001").provider(),
        "google.vertex"
    );
    assert_eq!(
        provider.image("imagen-4.0-generate-001").provider(),
        "google.vertex"
    );
    assert_eq!(
        provider.video("veo-3.0-generate-001").provider(),
        "google.vertex.video"
    );
    assert_eq!(
        provider.transcription("chirp_2").provider(),
        "google.vertex.transcription"
    );
    assert_eq!(
        provider.anthropic_model("claude-sonnet-4-5").provider(),
        "googleVertex.anthropic.messages"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Auth headers  (TS: "default headers function should return auth token",
//                    "should use custom headers in addition to auth token",
//                    "should pass options through to base provider when apiKey
//                     is provided")
// ════════════════════════════════════════════════════════════════════════════

/// TS: bearer-token auth resolves to an `Authorization: Bearer {token}` header.
#[tokio::test]
#[serial]
async fn bearer_token_sent_via_authorization_header() {
    // A request with no API key reads `GOOGLE_VERTEX_API_KEY`: keep the
    // env-driven tests below from selecting Express mode here.
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer my-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let provider = bearer_at("my-bearer-token", server.uri());
    provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed — mock requires the Authorization: Bearer header");
}

/// TS: custom headers are sent alongside the auth token (per-call here).
#[tokio::test]
#[serial]
async fn custom_headers_sent_alongside_bearer_token() {
    // A request with no API key reads `GOOGLE_VERTEX_API_KEY`: keep the
    // env-driven tests below from selecting Express mode here.
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer my-bearer-token"))
        .and(header("custom-header", "custom-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let provider = bearer_at("my-bearer-token", server.uri());
    let mut opts = default_options(test_prompt());
    let mut headers = HashMap::new();
    headers.insert("Custom-Header".to_string(), "custom-value".to_string());
    opts.headers = Some(headers);

    provider
        .chat("gemini-2.0-flash")
        .do_generate(&opts)
        .await
        .expect("should succeed — mock requires both headers");
}

/// TS: `headers` setting, `Resolvable` form: provider headers are sent next to
/// the token and are evaluated on every request.
#[tokio::test]
#[serial]
async fn provider_headers_producer_runs_on_every_request() {
    // A request with no API key reads `GOOGLE_VERTEX_API_KEY`: keep the
    // env-driven tests below from selecting Express mode here.
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer tok"))
        .and(header("x-team", "blue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let provider = vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("tok".to_string())),
        project: Some("p".to_string()),
        location: Some("l".to_string()),
        base_url: Some(server.uri()),
        headers: Some(Resolvable::from_async_fn(move || {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok([("x-team".to_string(), Some("blue".to_string()))]
                    .into_iter()
                    .collect())
            }
        })),
        ..Default::default()
    });
    let model = provider.chat("gemini-2.0-flash");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "creation evaluates nothing"
    );
    for _ in 0..2 {
        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("both headers are sent");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A host that authenticates some other way (its own ADC, a gateway) supplies
/// the `Authorization` header through `headers`; no token is then required.
#[tokio::test]
#[serial]
async fn authorization_from_headers_replaces_the_token() {
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer from-the-host"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let provider = vertex(VertexProviderSettings {
        base_url: Some(server.uri()),
        headers: Some(Resolvable::Value(
            [(
                "Authorization".to_string(),
                Some("Bearer from-the-host".to_string()),
            )]
            .into_iter()
            .collect(),
        )),
        ..Default::default()
    });
    provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("the host's Authorization header is used");
}

/// TS: "should pass options through to base provider when apiKey is provided" —
/// Express-mode API key uses `x-goog-api-key` (and does NOT send an
/// `Authorization: Bearer` header, i.e. the token is never asked for).
#[tokio::test]
#[serial]
async fn api_key_uses_x_goog_api_key_header() {
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("x-goog-api-key", "express-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;

    let provider = vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("express-api-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    });
    provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed — mock requires the x-goog-api-key header");
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[0].headers.get("authorization").is_none(),
        "Express mode sends no bearer token"
    );
}

/// `base_url` is honoured end-to-end (the request hits the override host).
#[tokio::test]
#[serial]
async fn base_url_is_used_for_requests() {
    // A request with no API key reads `GOOGLE_VERTEX_API_KEY`: keep the
    // env-driven tests below from selecting Express mode here.
    clear_vertex_env();
    let server = MockServer::start().await;
    mock_ok(&server).await;

    let provider = bearer_at("token", format!("{}/", server.uri()));
    provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("request should hit the mock server, trailing slash removed");
}

// ════════════════════════════════════════════════════════════════════════════
// Tuned-model restriction  (TS: Express mode cannot address tuned endpoints)
// ════════════════════════════════════════════════════════════════════════════

/// TS: a tuned model (`endpoints/…`) cannot be used with Express-mode API key
/// auth — the request is rejected before anything is sent.
#[tokio::test]
#[serial]
async fn tuned_model_rejected_with_api_key_auth() {
    clear_vertex_env();
    let server = MockServer::start().await;
    mock_ok(&server).await;
    let provider = vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("express-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    });
    let error = provider
        .chat("endpoints/1234567890")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiMuxError::InvalidArgument(_)),
        "tuned models should be rejected under Express-mode API key auth, got {error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// Tuned models are allowed with standard (bearer-token) auth and are served
/// without the `/publishers/google` suffix.
#[tokio::test]
#[serial]
async fn tuned_model_allowed_with_bearer_token_auth() {
    // A request with no API key reads `GOOGLE_VERTEX_API_KEY`: keep the
    // env-driven tests below from selecting Express mode here.
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/locations/l/endpoints/1234567890:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let provider = bearer_at(
        "token",
        format!("{}/locations/l/publishers/google", server.uri()),
    );
    let model = provider.chat("endpoints/1234567890");
    assert_eq!(model.model_id(), "endpoints/1234567890");
    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("tuned models should be allowed with bearer-token auth");
}

// ════════════════════════════════════════════════════════════════════════════
// Environment  (TS: env-var driven provider creation) — loaded per request
// ════════════════════════════════════════════════════════════════════════════

/// `GOOGLE_VERTEX_API_KEY` selects Express mode, even over an access token.
#[tokio::test]
#[serial]
async fn env_api_key_selects_express_mode_over_access_token() {
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("x-goog-api-key", "env-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let provider = vertex(VertexProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    });
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_API_KEY", "env-api-key");
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
    }
    let result = provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await;
    clear_vertex_env();
    result.expect("the env API key authenticates the call");
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].headers.get("authorization").is_none());
}

/// An access token in the environment is used as the bearer token.
#[tokio::test]
#[serial]
async fn env_access_token_is_the_bearer_token() {
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer env-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let provider = vertex(VertexProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    });
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
    }
    let result = provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await;
    clear_vertex_env();
    result.expect("the env access token authenticates the call");
}

/// A blank `GOOGLE_VERTEX_API_KEY` falls through to access-token auth.
#[tokio::test]
#[serial]
async fn blank_env_api_key_falls_through_to_access_token() {
    clear_vertex_env();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer env-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let provider = vertex(VertexProviderSettings {
        base_url: Some(server.uri()),
        ..Default::default()
    });
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_API_KEY", "   ");
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
    }
    let result = provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await;
    clear_vertex_env();
    result.expect("a blank key is no key");
}

/// With nothing set, the first request fails with the typed error naming the
/// missing environment variable: a missing token is `LoadApiKey`.
#[tokio::test]
#[serial]
async fn missing_access_token_fails_the_call_with_load_api_key() {
    clear_vertex_env();
    let provider = vertex(VertexProviderSettings {
        project: Some("p".to_string()),
        location: Some("us-central1".to_string()),
        ..Default::default()
    });
    let error = provider
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err();
    match error {
        AiMuxError::LoadApiKey { env_var, .. } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_ACCESS_TOKEN");
        }
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
}

/// A missing project or location is `LoadSetting` naming the variable; the
/// location is checked first, as `createGoogleVertex` does.
#[tokio::test]
#[serial]
async fn missing_project_and_location_fail_the_call_with_load_setting() {
    clear_vertex_env();
    let no_location = vertex(VertexProviderSettings {
        project: Some("p".to_string()),
        access_token: Some(Resolvable::Value("t".to_string())),
        ..Default::default()
    });
    match no_location
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err()
    {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_LOCATION");
            assert_eq!(name, "location");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }
    let no_project = vertex(VertexProviderSettings {
        location: Some("us-central1".to_string()),
        access_token: Some(Resolvable::Value("t".to_string())),
        ..Default::default()
    });
    match no_project
        .chat("gemini-2.0-flash")
        .do_generate(&default_options(test_prompt()))
        .await
        .unwrap_err()
    {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_PROJECT");
            assert_eq!(name, "project");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }
}
