//! Runtime registry validation and by-name preset behavior.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::json;
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use aimux_providers::preset;
use aimux_providers::{PresetSettings, create_provider};

// ── injected transport ───────────────────────────────────────────────────────

/// One request as the transport saw it.
#[derive(Clone, Debug)]
struct Seen {
    url: String,
    /// Lower-cased names (the `http` crate normalizes them).
    headers: BTreeMap<String, String>,
}

#[derive(Default)]
struct MockFetch {
    canned: Mutex<VecDeque<Vec<u8>>>,
    seen: Mutex<Vec<Seen>>,
}

impl MockFetch {
    fn new(canned: Vec<Vec<u8>>) -> Arc<Self> {
        Arc::new(Self {
            canned: Mutex::new(canned.into()),
            seen: Mutex::default(),
        })
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }
}

#[async_trait]
impl Fetch for MockFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.seen.lock().unwrap().push(Seen {
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().expect("ASCII header").to_string(),
                    )
                })
                .collect(),
        });
        let canned = self
            .canned
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| FetchError::Other("no canned response left".into()))?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            headers,
            request.url,
            Bytes::from(canned),
        ))
    }
}

/// Temporarily change an environment variable; used by serial tests.
struct EnvVar {
    name: &'static str,
    saved: Option<String>,
}

impl EnvVar {
    fn set(name: &'static str, value: Option<&str>) -> Self {
        let saved = std::env::var(name).ok();
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        Self { name, saved }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        unsafe {
            match &self.saved {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn chat_ok() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })).unwrap()
}

fn hello() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("hi")])
}

#[serial]
#[test]
fn every_registry_row_loads_and_creates_without_environment() {
    let entries: Vec<_> = preset::entries().collect();
    assert_eq!(entries.len(), 282);
    assert!(
        preset::names()
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    let mut variables = BTreeSet::new();
    for entry in &entries {
        let d = entry.descriptor;
        variables.extend((!d.env_var.is_empty()).then_some(d.env_var.as_str()));
        variables.extend(d.base_url_env.as_deref());
        variables.extend(
            d.params
                .iter()
                .flat_map(|param| param.env.iter().map(String::as_str)),
        );
    }
    let _guards: Vec<_> = variables
        .into_iter()
        .map(|var| EnvVar::set(var, None))
        .collect();
    for entry in entries {
        preset::create(&entry.descriptor.name, PresetSettings::default())
            .unwrap_or_else(|error| panic!("{}: {error}", entry.descriptor.name));
    }
}

#[tokio::test]
#[serial]
async fn a_keyless_row_sends_no_authorization() {
    // The row's optional key variable must be unset for this assertion.
    let _key = EnvVar::set("OLLAMA_API_KEY", None);
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = preset::create(
        "ollama",
        PresetSettings {
            base_url: Some("http://127.0.0.1:9/v1".into()),
            fetch: Some(mock.transport()),
            ..Default::default()
        },
    )
    .unwrap();
    preset.chat("m").do_generate(&hello()).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].headers.contains_key("authorization"));
    assert_eq!(seen[0].url, "http://127.0.0.1:9/v1/chat/completions");
}

#[test]
fn an_unknown_name_has_no_fallback() {
    assert!(preset::lookup("no-such-preset").is_none());
    assert!(
        matches!(create_provider("no-such-preset", PresetSettings::default()),
        Err(AiMuxError::NoSuchProvider { provider_id, .. }) if provider_id == "no-such-preset")
    );
}

#[tokio::test]
async fn a_template_parameter_must_be_a_plain_host_segment() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let result = preset::create(
        "neon",
        PresetSettings {
            params: [("branch_host".into(), "user@host/path".into())].into(),
            fetch: Some(mock.transport()),
            ..Default::default()
        },
    );
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument(message)) if message.contains("branch_host"))
    );
    assert!(mock.seen().is_empty());
}

/// The built-in table holds the vendor packages and every registry row, and a
/// registry over it resolves both the same way.
#[test]
fn default_providers_serve_packages_and_registry_rows_through_one_registry() {
    let providers = aimux_providers::default_providers();
    assert!(
        providers.len() > 282,
        "packages and rows: {}",
        providers.len()
    );
    let registry = aimux_core::create_provider_registry(providers, Default::default());

    let model = registry.language_model("anthropic:claude-x").unwrap();
    assert_eq!(model.provider(), "anthropic.messages");
    let model = registry.language_model("abacus:some/model:v1").unwrap();
    assert_eq!(model.provider(), "abacus.chat");
    assert_eq!(model.model_id(), "some/model:v1");
    // A vendor package wins over the registry row of the same name.
    let model = registry.language_model("groq:m").unwrap();
    assert_eq!(model.provider(), "groq.chat");

    let error = registry.language_model("nope:m").err().unwrap();
    assert!(
        matches!(error, AiMuxError::NoSuchProvider { .. }),
        "{error:?}"
    );
}

/// A local server that answers one chat completion at `/v1/chat/completions`.
async fn chat_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(chat_ok(), "application/json"))
        .mount(&server)
        .await;
    server
}

/// The row's `base_url_env` (`OLLAMA_BASE_URL`, provider_registry.json
/// :1303-1310) is read when a request is made, not when the provider is
/// created (preset.rs:375-384 builds a lazy base URL; :468-477 reads the
/// variable): setting it after `create` sends the call there.
#[tokio::test]
#[serial]
async fn the_base_url_variable_is_read_per_request() {
    let server = chat_server().await;
    let _key = EnvVar::set("OLLAMA_API_KEY", None);
    let _unset = EnvVar::set("OLLAMA_BASE_URL", None);
    let preset = preset::create("ollama", PresetSettings::default()).unwrap();

    let url = format!("{}/v1", server.uri());
    let _set = EnvVar::set("OLLAMA_BASE_URL", Some(url.as_str()));
    preset.chat("m").do_generate(&hello()).await.unwrap();

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.path(), "/v1/chat/completions");
}

/// An explicit `base_url` wins over the variable (preset.rs:376).
#[tokio::test]
#[serial]
async fn an_explicit_base_url_wins_over_the_variable() {
    let server = chat_server().await;
    let _key = EnvVar::set("OLLAMA_API_KEY", None);
    // Port 9 (discard) refuses connections: reaching it fails the call.
    let _url = EnvVar::set("OLLAMA_BASE_URL", Some("http://127.0.0.1:9/v1"));
    let preset = preset::create(
        "ollama",
        PresetSettings {
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        },
    )
    .unwrap();

    preset.chat("m").do_generate(&hello()).await.unwrap();

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// An unusable `OLLAMA_BASE_URL` does not fail the factory; the request fails
/// with an InvalidArgument naming the variable, and nothing is sent
/// (preset.rs:474-476, validate_base_url in provider-utils url.rs:33-56).
#[tokio::test]
#[serial]
async fn an_invalid_base_url_variable_fails_the_request_not_the_factory() {
    let _key = EnvVar::set("OLLAMA_API_KEY", None);
    let _url = EnvVar::set("OLLAMA_BASE_URL", Some("ftp://models.internal/v1"));
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = preset::create(
        "ollama",
        PresetSettings {
            fetch: Some(mock.transport()),
            ..Default::default()
        },
    )
    .expect("the variable is not read when the provider is created");

    let Err(error) = preset.chat("m").do_generate(&hello()).await else {
        panic!("an ftp:// base URL must fail the request");
    };
    assert!(
        matches!(&error, AiMuxError::InvalidArgument(message)
            if message.contains("OLLAMA_BASE_URL") && message.contains("http or https")),
        "{error:?}"
    );
    assert!(mock.seen().is_empty());
}

/// A keyless row with an `env_var` reads it per request as an optional key
/// (`Credential::OptionalEnv`, preset.rs:393; shared/mod.rs:85, :113):
/// `Authorization: Bearer <value>` when set, nothing when empty or unset.
#[tokio::test]
#[serial]
async fn the_optional_key_variable_is_sent_only_when_set() {
    let server = chat_server().await;
    let _url = EnvVar::set("OLLAMA_BASE_URL", None);
    let _key = EnvVar::set("OLLAMA_API_KEY", None);
    let preset = preset::create(
        "ollama",
        PresetSettings {
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        },
    )
    .unwrap();
    let model = preset.chat("m");

    {
        let _set = EnvVar::set("OLLAMA_API_KEY", Some("ollama-secret"));
        model.do_generate(&hello()).await.unwrap();
    }
    {
        let _empty = EnvVar::set("OLLAMA_API_KEY", Some(""));
        model.do_generate(&hello()).await.unwrap();
    }
    model.do_generate(&hello()).await.unwrap();

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 3);
    assert_eq!(
        received[0].headers["authorization"].to_str().unwrap(),
        "Bearer ollama-secret"
    );
    assert!(
        !received[1].headers.contains_key("authorization"),
        "empty key"
    );
    assert!(
        !received[2].headers.contains_key("authorization"),
        "unset key"
    );
}
