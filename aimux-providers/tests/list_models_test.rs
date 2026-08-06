//! Integration tests for `Provider::list_models` (RFC-0027).
//!
//! Each test mounts a single recorded `/models` cassette on a wiremock server,
//! points an `OpenAIProvider` (or registry `provider_handle`) at it, and asserts
//! the returned `ResolvedModel` list. The community catalogue enrichment path is
//! exercised separately with a pre-seeded cache (offline, no network).

mod common;

use std::path::Path;

use serde_json::Value;
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::model_catalogue::ResolvedModel;
use aimux_core::provider::Provider;
use aimux_providers::catalogue::{self, CatalogueSync};
use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
use aimux_providers::{ProviderOptions, provider_handle};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Read a single cassette JSON file and mount its recorded response on `server`
/// at its recorded `(method, path)`. Returns the recorded path so the caller
/// can derive the matching `base_url`.
async fn mount_cassette_file(server: &MockServer, cassette_path: &Path) -> String {
    let text = std::fs::read_to_string(cassette_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", cassette_path.display()));
    let v: Value = serde_json::from_str(&text).expect("cassette is JSON");
    let req = &v["request"];
    let m = req["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
    let p = req["path"]
        .as_str()
        .expect("cassette has request.path")
        .to_string();
    let resp = &v["response"];
    let status = resp["status"].as_u64().unwrap_or(200) as u16;
    let body = resp["body"].as_str().unwrap_or("");
    let mut template = ResponseTemplate::new(status).set_body_bytes(body.as_bytes().to_vec());
    if let Some(headers) = resp["headers"].as_object() {
        for (name, val) in headers {
            if let Some(s) = val.as_str() {
                template = template.append_header(name.as_str(), s);
            }
        }
    }
    Mock::given(method(m.as_str()))
        .and(path(p.as_str()))
        .respond_with(template)
        .mount(server)
        .await;
    p
}

/// Derive the `base_url` to point a provider at `server` such that
/// `{base_url}/models` hits `cassette_path_str` on the mock.
fn base_url_for(server_uri: &str, cassette_path_str: &str) -> String {
    // cassette_path_str is e.g. "/v1/models" or "/models"; strip trailing "/models".
    let base = cassette_path_str
        .strip_suffix("/models")
        .unwrap_or(cassette_path_str);
    format!("{server_uri}{base}")
}

/// Set the catalogue to a fully-offline temp cache dir for the duration of the
/// test. Returns the temp dir path (caller cleans up).
fn offline_catalogue() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aimux-list-models-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // SAFETY: tests are `#[serial]` so no other test touches these env vars
    // concurrently. Rust 2024 made `set_var` unsafe for thread-safety.
    unsafe {
        std::env::set_var("AIMUX_CATALOGUE_OFFLINE", "1");
        std::env::set_var("AIMUX_CATALOGUE_DIR", &dir);
    }
    dir
}

fn cleanup_catalogue(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ── OpenAI-compatible list_models ────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn openai_provider_list_models() {
    let dir = offline_catalogue();
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/openai/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);

    let config = OpenAIConfig::new("test-key").with_base_url(base_url);
    let provider = OpenAIProvider::new(config);
    let models: Vec<ResolvedModel> = provider.list_models().await.unwrap();

    // The recorded OpenAI list includes gpt-4o.
    assert!(models.iter().any(|m| m.id == "gpt-4o"));
    // owned_by carried through.
    assert!(models.iter().all(|m| m.owned_by.is_some()));

    cleanup_catalogue(&dir);
}

#[tokio::test]
#[serial]
async fn deepseek_provider_list_models_via_handle() {
    let dir = offline_catalogue();
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);

    // Registry-backed provider, base_url overridden to the mock.
    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();

    assert_eq!(models.len(), 2);
    assert!(models.iter().any(|m| m.id == "deepseek-v4-flash"));
    assert!(models.iter().any(|m| m.id == "deepseek-v4-pro"));
    assert_eq!(handle.name(), "openai"); // protocol impl name (OpenAI-compat)

    cleanup_catalogue(&dir);
}

#[tokio::test]
#[serial]
async fn provider_handle_then_language_model() {
    // Round-trip: handle.list_models() to discover, then handle.language_model()
    // to build a usable model — the RFC-0027 usage flow.
    let dir = offline_catalogue();
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);

    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();
    let first_id = models[0].id.clone();
    let _model = handle.language_model(&first_id).unwrap();

    cleanup_catalogue(&dir);
}

// ── catalogue enrichment (offline, pre-seeded cache) ─────────────────────────

#[tokio::test]
#[serial]
async fn list_models_attaches_catalogue_spec() {
    // Pre-seed the catalogue cache with a deepseek entry, then list_models and
    // verify the spec is attached to the matching runtime model.
    let dir = offline_catalogue();
    std::fs::create_dir_all(&dir).unwrap();

    let anya2a = serde_json::json!({
        "updated_at": "100",
        "providers": {
            "deepseek": {
                "models": [{
                    "id": "deepseek-v4-flash",
                    "type": "chat",
                    "tool_call": true,
                    "limit": { "context": 1000000, "output": 384000 },
                    "reasoning": { "supported": true, "default": true },
                    "extra_capabilities": {
                        "reasoning": { "mode": "effort", "effort": "high" }
                    },
                    "cost": { "input": 0.14, "output": 0.28 }
                }]
            }
        }
    });
    let cat = catalogue::parse_anya2a_all(&anya2a).unwrap();
    let cache_json = serde_json::to_string(&cat).unwrap();
    std::fs::write(dir.join("catalogue.json"), cache_json).unwrap();

    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);

    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();

    let flash = models
        .iter()
        .find(|m| m.id == "deepseek-v4-flash")
        .expect("deepseek-v4-flash in list");
    let spec = flash.spec.as_ref().expect("spec attached from cache");
    assert_eq!(spec.limits.context, Some(1000000));
    assert_eq!(spec.limits.output, Some(384000));
    assert!(spec.capabilities.tool_call);
    let reasoning = spec.reasoning.as_ref().unwrap();
    assert!(reasoning.supported);
    assert_eq!(reasoning.effort_default.as_deref(), Some("high"));
    assert_eq!(spec.cost.as_ref().unwrap().input, Some(0.14));

    // The other model has no cache entry → spec is None.
    let pro = models.iter().find(|m| m.id == "deepseek-v4-pro").unwrap();
    assert!(pro.spec.is_none());

    cleanup_catalogue(&dir);
}

#[tokio::test]
#[serial]
async fn list_models_without_cache_still_returns_runtime() {
    // No catalogue cache → specs are None, but runtime models still returned.
    let dir = offline_catalogue();
    let server = MockServer::start().await;
    let cassette = Path::new("tests/cassettes/deepseek/list_models_smoke.json");
    let recorded_path = mount_cassette_file(&server, cassette).await;
    let base_url = base_url_for(&server.uri(), &recorded_path);

    let opts = ProviderOptions {
        base_url: Some(base_url),
        ..Default::default()
    };
    let handle = provider_handle("deepseek", Some("test-key".into()), Some(opts)).unwrap();
    let models = handle.list_models().await.unwrap();
    assert_eq!(models.len(), 2);
    assert!(models.iter().all(|m| m.spec.is_none()));

    cleanup_catalogue(&dir);
}

// ── default (unsupported) providers ──────────────────────────────────────────

#[tokio::test]
async fn unsupported_provider_returns_unsupported() {
    // A provider that doesn't override list_models returns Unsupported via the
    // trait default, rather than panicking.
    struct StubProvider;
    impl Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn language_model(
            &self,
            _model_id: &str,
        ) -> Result<Box<dyn aimux_core::language_model::LanguageModel>, aimux_core::AiMuxError>
        {
            Err(aimux_core::AiMuxError::Unsupported("none".into()))
        }
    }
    let p = StubProvider;
    let err = p.list_models().await.unwrap_err();
    assert!(matches!(err, aimux_core::AiMuxError::Unsupported(_)));
}

#[test]
fn catalogue_normalize_and_lookup() {
    // Pure unit checks (no async, no network) for the name-mapping layer.
    assert_eq!(
        catalogue::normalize_provider_name("fireworks-ai"),
        "fireworks"
    );
    assert_eq!(
        catalogue::normalize_provider_name("google-vertex"),
        "vertex"
    );
    assert_eq!(
        catalogue::normalize_provider_name("some-unknown-provider"),
        "some_unknown_provider"
    );

    let dir = offline_catalogue();
    std::fs::create_dir_all(&dir).unwrap();
    let sync = CatalogueSync::new().with_cache_dir(dir.clone()).offline();
    let anya2a = serde_json::json!({
        "updated_at": "1",
        "providers": {
            "fireworks-ai": { "models": [
                { "id": "fw-1", "type": "chat", "tool_call": true }
            ]}
        }
    });
    let cat = catalogue::parse_anya2a_all(&anya2a).unwrap();
    // Normalized to "fireworks" → lookup by aimux name works.
    assert!(cat.lookup("fireworks", "fw-1").is_some());
    assert!(cat.lookup("fireworks-ai", "fw-1").is_none());

    // Round-trip through cache (write directly to the cache file).
    let cache_json = serde_json::to_string(&cat).unwrap();
    std::fs::write(dir.join("catalogue.json"), cache_json).unwrap();
    let loaded = sync.load_cached().unwrap().unwrap();
    assert!(loaded.lookup("fireworks", "fw-1").is_some());

    cleanup_catalogue(&dir);
}
