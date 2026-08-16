//! Remaining Google Vertex AI provider tests — ported from the TS SDK suite.
//!
//! Mirrors `reference/ai/packages/google-vertex/src/google-vertex-provider.test.ts`
//! (provider configuration: auth headers, base URL, env-var resolution, project
//! handling, Express-mode API key, tuned-model restrictions).
//!
//! The TS tests mock `createAuthTokenGenerator` / `createGoogleVertex` and assert
//! on the options passed to the base provider. The Rust provider has no lazy
//! token generator — a bearer token (or API key) is supplied directly to
//! `VertexProviderConfig` — so each TS scenario is translated to the equivalent
//! observable Rust behaviour: which auth variant is selected, which headers are
//! sent, and how the project/location shape the base URL.
//!
//! Model-level generate/stream behaviour is already covered by
//! `vertex_model_test.rs`; this file focuses on provider configuration only.

use serde_json::json;
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_providers::vertex::{VertexAuth, VertexProvider, VertexProviderConfig};

// ── Shared helpers ───────────────────────────────────────────────────────────

/// All environment variables consulted by `VertexProviderConfig::from_env`.
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

/// Mount a minimal 200 generateContent mock on the standard model path.
async fn mock_ok(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "parts": [{ "text": "hi" }], "role": "model" },
                "finishReason": "STOP", "index": 0
            }]
        })))
        .mount(server)
        .await;
}

// ════════════════════════════════════════════════════════════════════════════
// Provider identity  (TS: google-vertex-provider.test.ts — base wiring)
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provider_name_is_google_vertex() {
    let config = VertexProviderConfig::new("test-token", "my-project", "us-central1");
    let provider = VertexProvider::new(config);
    assert_eq!(provider.name(), "google.vertex");
}

#[tokio::test]
async fn language_model_trait_method_returns_boxed_model() {
    let config = VertexProviderConfig::new("test-token", "my-project", "us-central1");
    let provider = VertexProvider::new(config);
    let model = provider.language_model("gemini-2.0-flash").expect("model");
    assert_eq!(model.model_id(), "gemini-2.0-flash");
    assert_eq!(model.provider(), "google.vertex");
}

#[tokio::test]
async fn model_factory_returns_vertex_model() {
    let config = VertexProviderConfig::new("test-token", "my-project", "us-central1");
    let provider = VertexProvider::new(config);
    let model = provider.model("gemini-2.0-flash").expect("model");
    assert_eq!(model.model_id(), "gemini-2.0-flash");
    assert_eq!(model.provider(), "google.vertex");
}

/// TS: "creates the auth token generator once per provider instance" — a single
/// provider instance can mint multiple models that all share its config.
#[tokio::test]
async fn one_provider_instance_creates_multiple_models() {
    let config = VertexProviderConfig::new("test-token", "my-project", "us-central1");
    let provider = VertexProvider::new(config);

    let m1 = provider.model("gemini-2.0-flash").expect("model 1");
    let m2 = provider.model("gemini-2.5-pro").expect("model 2");

    assert_eq!(m1.model_id(), "gemini-2.0-flash");
    assert_eq!(m2.model_id(), "gemini-2.5-pro");
    assert_eq!(m1.provider(), "google.vertex");
    assert_eq!(m2.provider(), "google.vertex");
}

// ════════════════════════════════════════════════════════════════════════════
// Auth headers  (TS: "default headers function should return auth token",
//                    "should use custom headers in addition to auth token",
//                    "should pass options through to base provider when apiKey
//                     is provided")
// ════════════════════════════════════════════════════════════════════════════

/// TS: bearer-token auth resolves to an `Authorization: Bearer {token}` header.
#[tokio::test]
async fn bearer_token_sent_via_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer my-bearer-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "parts": [{ "text": "hi" }], "role": "model" },
                "finishReason": "STOP", "index": 0
            }]
        })))
        .mount(&server)
        .await;

    let config = VertexProviderConfig::new("my-bearer-token", "my-project", "us-central1")
        .with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.model("gemini-2.0-flash").expect("model");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed — mock requires the Authorization: Bearer header");
}

/// TS: custom headers are sent alongside the auth token.
#[tokio::test]
async fn custom_headers_sent_alongside_bearer_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("authorization", "Bearer my-bearer-token"))
        .and(header("custom-header", "custom-value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "parts": [{ "text": "hi" }], "role": "model" },
                "finishReason": "STOP", "index": 0
            }]
        })))
        .mount(&server)
        .await;

    let config = VertexProviderConfig::new("my-bearer-token", "my-project", "us-central1")
        .with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.model("gemini-2.0-flash").expect("model");

    let mut opts = default_options(test_prompt());
    let mut headers = std::collections::HashMap::new();
    headers.insert("Custom-Header".to_string(), "custom-value".to_string());
    opts.headers = Some(headers);

    model
        .do_generate(&opts)
        .await
        .expect("should succeed — mock requires both headers");
}

/// TS: "should pass options through to base provider when apiKey is provided" —
/// Express-mode API key uses `x-goog-api-key` (and does NOT send an
/// `Authorization: Bearer` header, i.e. the token generator is not invoked).
#[tokio::test]
async fn api_key_uses_x_goog_api_key_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.0-flash:generateContent"))
        .and(header("x-goog-api-key", "express-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{
                "content": { "parts": [{ "text": "hi" }], "role": "model" },
                "finishReason": "STOP", "index": 0
            }]
        })))
        .mount(&server)
        .await;

    let config = VertexProviderConfig::with_api_key("express-api-key").with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.model("gemini-2.0-flash").expect("model");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed — mock requires the x-goog-api-key header");
}

/// Express-mode config selects the `ApiKey` auth variant (no bearer token).
#[tokio::test]
async fn with_api_key_selects_apikey_auth_variant() {
    let config = VertexProviderConfig::with_api_key("express-key");
    match &config.auth {
        VertexAuth::ApiKey(k) => assert_eq!(k, "express-key"),
        other => panic!("expected ApiKey auth, got {other:?}"),
    }
}

/// Bearer-token config selects the `BearerToken` auth variant.
#[tokio::test]
async fn new_selects_bearer_token_auth_variant() {
    let config = VertexProviderConfig::new("the-token", "proj", "us-central1");
    match &config.auth {
        VertexAuth::BearerToken(t) => assert_eq!(t, "the-token"),
        other => panic!("expected BearerToken auth, got {other:?}"),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Base URL construction  (TS: project / location handling)
// ════════════════════════════════════════════════════════════════════════════

/// TS: the project is threaded into the base URL (projectId → URL path).
#[tokio::test]
async fn new_embeds_project_in_base_url() {
    let config = VertexProviderConfig::new("token", "my-gcp-project", "us-central1");
    assert!(
        config.base_url.contains("/projects/my-gcp-project/"),
        "base_url should embed the project: {}",
        config.base_url
    );
}

/// TS: the location is threaded into the base URL.
#[tokio::test]
async fn new_embeds_location_in_base_url() {
    let config = VertexProviderConfig::new("token", "proj", "us-central1");
    assert!(
        config.base_url.contains("/locations/us-central1/"),
        "base_url should embed the location: {}",
        config.base_url
    );
}

/// Location `us-central1` → `{location}-aiplatform.googleapis.com` host.
#[tokio::test]
async fn base_url_for_regional_location() {
    let config = VertexProviderConfig::new("token", "proj", "us-central1");
    assert_eq!(
        config.base_url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google"
    );
}

/// Location `global` → bare `aiplatform.googleapis.com` host.
#[tokio::test]
async fn base_url_for_global_location() {
    let config = VertexProviderConfig::new("token", "proj", "global");
    assert_eq!(
        config.base_url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/publishers/google"
    );
}

/// Location `eu`/`us` → multi-region `aiplatform.{region}.rep.googleapis.com`.
#[tokio::test]
async fn base_url_for_eu_multi_region() {
    let config = VertexProviderConfig::new("token", "proj", "eu");
    assert_eq!(
        config.base_url,
        "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/proj/locations/eu/publishers/google"
    );
}

#[tokio::test]
async fn base_url_for_us_multi_region() {
    let config = VertexProviderConfig::new("token", "proj", "us");
    assert_eq!(
        config.base_url,
        "https://aiplatform.us.rep.googleapis.com/v1beta1/projects/proj/locations/us/publishers/google"
    );
}

/// Express-mode (API key) uses the `/v1/publishers/google` endpoint (no
/// project/location scoping).
#[tokio::test]
async fn express_mode_base_url() {
    let config = VertexProviderConfig::with_api_key("key");
    assert_eq!(
        config.base_url,
        "https://aiplatform.googleapis.com/v1/publishers/google"
    );
}

/// `with_base_url` overrides the constructed URL and strips a trailing slash.
#[tokio::test]
async fn with_base_url_strips_trailing_slash() {
    let config = VertexProviderConfig::new("token", "proj", "us-central1")
        .with_base_url("https://example.com/v1beta1/");
    assert_eq!(config.base_url, "https://example.com/v1beta1");
}

/// `with_base_url` is honoured end-to-end (request hits the override host).
#[tokio::test]
async fn with_base_url_is_used_for_requests() {
    let server = MockServer::start().await;
    mock_ok(&server).await;

    let config =
        VertexProviderConfig::new("token", "proj", "us-central1").with_base_url(server.uri());
    let provider = VertexProvider::new(config);
    let model = provider.model("gemini-2.0-flash").expect("model");

    model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("request should hit the mock server");
}

// ════════════════════════════════════════════════════════════════════════════
// Tuned-model restriction  (TS: Express mode cannot address tuned endpoints)
// ════════════════════════════════════════════════════════════════════════════

/// TS: a tuned model (`endpoints/…`) cannot be used with Express-mode API key
/// auth — the provider rejects it up front.
#[tokio::test]
async fn tuned_model_rejected_with_api_key_auth() {
    let config = VertexProviderConfig::with_api_key("express-key");
    let provider = VertexProvider::new(config);
    let result = provider.model("endpoints/1234567890");
    assert!(
        result.is_err(),
        "tuned models should be rejected under Express-mode API key auth"
    );
}

/// Tuned models are allowed with standard (bearer-token) auth.
#[tokio::test]
async fn tuned_model_allowed_with_bearer_token_auth() {
    let config = VertexProviderConfig::new("token", "proj", "us-central1");
    let provider = VertexProvider::new(config);
    let model = provider
        .model("endpoints/1234567890")
        .expect("tuned models should be allowed with bearer-token auth");
    assert_eq!(model.model_id(), "endpoints/1234567890");
}

// ════════════════════════════════════════════════════════════════════════════
// from_env  (TS: env-var driven provider creation)
// ════════════════════════════════════════════════════════════════════════════

/// `GOOGLE_VERTEX_API_KEY` selects Express mode (API key auth).
#[tokio::test]
#[serial]
async fn from_env_uses_api_key_when_set() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_API_KEY", "env-api-key");
    }
    let config = VertexProviderConfig::from_env().expect("from_env");
    match config.auth {
        VertexAuth::ApiKey(k) => assert_eq!(k, "env-api-key"),
        other => panic!("expected ApiKey auth, got {other:?}"),
    }
    clear_vertex_env();
}

/// `GOOGLE_VERTEX_API_KEY` takes precedence over the access-token vars.
#[tokio::test]
#[serial]
async fn from_env_prefers_api_key_over_access_token() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_API_KEY", "env-api-key");
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
        std::env::set_var("GOOGLE_VERTEX_PROJECT", "env-project");
    }
    let config = VertexProviderConfig::from_env().expect("from_env");
    // API key wins → Express mode base URL (no project scoping).
    assert!(
        config.base_url.contains("/v1/publishers/google"),
        "API key should select Express mode: {}",
        config.base_url
    );
    match config.auth {
        VertexAuth::ApiKey(k) => assert_eq!(k, "env-api-key"),
        other => panic!("expected ApiKey auth, got {other:?}"),
    }
    clear_vertex_env();
}

/// Access token + project + location → bearer-token auth with scoped base URL.
#[tokio::test]
#[serial]
async fn from_env_uses_access_token_project_location() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
        std::env::set_var("GOOGLE_VERTEX_PROJECT", "env-project");
        std::env::set_var("GOOGLE_VERTEX_LOCATION", "europe-west1");
    }
    let config = VertexProviderConfig::from_env().expect("from_env");
    match config.auth {
        VertexAuth::BearerToken(t) => assert_eq!(t, "env-token"),
        other => panic!("expected BearerToken auth, got {other:?}"),
    }
    assert!(config.base_url.contains("/projects/env-project/"));
    assert!(config.base_url.contains("/locations/europe-west1/"));
    clear_vertex_env();
}

/// `GOOGLE_VERTEX_LOCATION` defaults to `us-central1` when unset.
#[tokio::test]
#[serial]
async fn from_env_defaults_location_to_us_central1() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
        std::env::set_var("GOOGLE_VERTEX_PROJECT", "env-project");
    }
    let config = VertexProviderConfig::from_env().expect("from_env");
    assert!(
        config.base_url.contains("/locations/us-central1/"),
        "location should default to us-central1: {}",
        config.base_url
    );
    clear_vertex_env();
}

/// Missing every var → error.
#[tokio::test]
#[serial]
async fn from_env_errors_when_all_missing() {
    clear_vertex_env();
    let result = VertexProviderConfig::from_env();
    assert!(result.is_err(), "from_env should error with no credentials");
    clear_vertex_env();
}

/// Access token present but project missing → error.
#[tokio::test]
#[serial]
async fn from_env_errors_when_project_missing() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
    }
    let result = VertexProviderConfig::from_env();
    assert!(
        result.is_err(),
        "from_env should error when GOOGLE_VERTEX_PROJECT is missing"
    );
    clear_vertex_env();
}

/// Project present but access token missing → error.
#[tokio::test]
#[serial]
async fn from_env_errors_when_access_token_missing() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_PROJECT", "env-project");
    }
    let result = VertexProviderConfig::from_env();
    assert!(
        result.is_err(),
        "from_env should error when GOOGLE_VERTEX_ACCESS_TOKEN is missing"
    );
    clear_vertex_env();
}

/// An empty `GOOGLE_VERTEX_API_KEY` falls through to access-token auth.
#[tokio::test]
#[serial]
async fn from_env_empty_api_key_falls_through_to_access_token() {
    clear_vertex_env();
    unsafe {
        std::env::set_var("GOOGLE_VERTEX_API_KEY", "   ");
        std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "env-token");
        std::env::set_var("GOOGLE_VERTEX_PROJECT", "env-project");
    }
    let config = VertexProviderConfig::from_env().expect("from_env");
    match config.auth {
        VertexAuth::BearerToken(t) => assert_eq!(t, "env-token"),
        other => panic!("expected BearerToken auth, got {other:?}"),
    }
    clear_vertex_env();
}
