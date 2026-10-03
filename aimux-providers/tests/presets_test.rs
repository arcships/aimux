//! The registry presets (`presets::create_<name>` / `presets::<name>()`,
//! RFC-0036 section 5): every row of `provider_registry.json` is an explicit
//! factory that creates without reading the environment or failing, resolves
//! its key and base URL per request, sends no `Authorization` for a keyless
//! (`auth: none`) row, expands template parameters under strict rules, and
//! never falls back to another provider.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, Resolvable,
};
use aimux_providers::{
    AuthMode, PresetFamily, PresetSettings, ProviderOptions, presets, provider, provider_names,
    provider_registry_entry,
};

// ── injected transport ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Canned {
    fn json(body: &Value) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(body).unwrap(),
        }
    }
}

/// One request as the transport saw it.
#[derive(Clone, Debug)]
struct Seen {
    url: String,
    /// Lower-cased names (the `http` crate normalizes them).
    headers: BTreeMap<String, String>,
}

#[derive(Default)]
struct MockFetch {
    canned: Mutex<VecDeque<Canned>>,
    seen: Mutex<Vec<Seen>>,
}

impl MockFetch {
    fn new(canned: Vec<Canned>) -> Arc<Self> {
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
        for (name, value) in &canned.headers {
            headers.insert(
                HeaderName::try_from(name.as_str()).unwrap(),
                HeaderValue::try_from(value.as_str()).unwrap(),
            );
        }
        Ok(FetchResponse::from_bytes(
            StatusCode::from_u16(canned.status).unwrap(),
            headers,
            request.url,
            Bytes::from(canned.body),
        ))
    }
}

/// The settings every Groq-named fixture was recorded with
/// (`scripts/aisdk-fixtures/cases/openai-compatible.mjs`).
/// Sets (or removes) an environment variable and restores it on drop. Tests
/// that use it are `#[serial]`.
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

fn chat_ok() -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    }))
}

fn hello() -> CallOptions {
    CallOptions::new(vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("hi")],
        ..Default::default()
    }])
}

/// Every row of `provider_registry.json`.
fn registry_rows() -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/provider_registry.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn create(
    name: &str,
    settings: PresetSettings,
) -> Result<aimux_providers::preset::PresetProvider, AiMuxError> {
    (presets::lookup(name).expect("a registry row").create)(settings)
}

fn with_transport(mock: &Arc<MockFetch>) -> PresetSettings {
    PresetSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    }
}

fn key(value: &str) -> Option<Resolvable<String>> {
    Some(Resolvable::Value(value.to_string()))
}

async fn one_chat(preset: &aimux_providers::preset::PresetProvider) -> Result<(), AiMuxError> {
    preset.chat("m").do_generate(&hello()).await.map(|_| ())
}

/// A map of the given parameters.
fn params(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// ── the table ────────────────────────────────────────────────────────────────

#[test]
fn the_table_is_the_registry() {
    let rows = registry_rows();
    let mut in_json: Vec<String> = rows
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_string())
        .collect();
    in_json.sort();
    let in_table: Vec<String> = presets::names().map(str::to_string).collect();
    assert_eq!(
        in_table, in_json,
        "presets::names() is the registry, in order"
    );
    assert_eq!(provider_names().count(), rows.len());
    assert!(rows.len() >= 251, "the 251 original rows are all there");
}

#[test]
fn every_row_creates_with_default_settings_and_reports_name_dot_chat() {
    let mut count = 0;
    for entry in presets::entries() {
        let name = entry.descriptor.name;
        let preset = (entry.create)(PresetSettings::default())
            .unwrap_or_else(|e| panic!("create_{name}(default) failed: {e}"));
        assert_eq!(preset.descriptor().name, name);
        assert_eq!(
            preset.chat("m").provider(),
            format!("{name}.chat"),
            "{name}"
        );
        assert_eq!(
            aimux_core::embedding_model::EmbeddingModel::provider(&preset.embedding("m")),
            format!("{name}.embedding")
        );
        assert_eq!(provider_registry_entry(name).unwrap().name, name);
        count += 1;
    }
    assert_eq!(count, registry_rows().len());
}

#[serial]
#[test]
fn creating_reads_no_environment_variable() {
    // Poison every variable a preset could read: creation must not look.
    let mut guards = Vec::new();
    for entry in presets::entries() {
        let d = entry.descriptor;
        if !d.env_var.is_empty() {
            guards.push(EnvVar::set(d.env_var, Some("poison")));
        }
        if let Some(var) = d.base_url_env {
            guards.push(EnvVar::set(var, Some("not a url")));
        }
    }
    for entry in presets::entries() {
        (entry.create)(PresetSettings::default())
            .unwrap_or_else(|e| panic!("{}: {e}", entry.descriptor.name));
    }
    drop(guards);
}

#[test]
fn default_instances_are_one_provider_each() {
    assert!(std::ptr::eq(presets::ollama(), presets::ollama()));
    assert!(std::ptr::eq(presets::abacus(), presets::abacus()));
    assert_eq!(presets::ollama().chat("m").provider(), "ollama.chat");
    assert_eq!(
        presets::vertex_ai_openai_models().chat("m").provider(),
        "vertex_ai_openai_models.chat"
    );
    assert_eq!(presets::ollama::DESCRIPTOR.name, "ollama");
    let descriptor = presets::groq::DESCRIPTOR;
    assert_eq!(descriptor.family, PresetFamily::Groq);
}

#[test]
fn an_unknown_preset_is_never_another_provider() {
    assert!(presets::lookup("no-such-preset").is_none());
    assert!(presets::lookup("").is_none());
    assert!(presets::lookup("OpenAI").is_none());
    match provider("no-such-preset", Some("k".into()), "m", None) {
        Err(AiMuxError::NoSuchProvider { provider_id }) => {
            assert_eq!(provider_id, "no-such-preset")
        }
        Err(other) => panic!("expected NoSuchProvider, got {other:?}"),
        Ok(_) => panic!("an unknown name must not build a provider"),
    }
}

// ── authentication ───────────────────────────────────────────────────────────

#[test]
fn the_keyless_rows_are_the_local_servers() {
    let keyless: Vec<&str> = presets::entries()
        .filter(|e| e.descriptor.auth == AuthMode::None)
        .map(|e| e.descriptor.name)
        .collect();
    assert_eq!(keyless.len(), 20, "{keyless:?}");
    for name in ["ollama", "vllm", "lmstudio", "llamacpp", "litellm_proxy"] {
        assert!(keyless.contains(&name), "{name}");
    }
    for name in keyless {
        let d = provider_registry_entry(name).unwrap();
        assert_eq!(d.env_var, "", "{name}: no key variable");
        assert!(
            d.base_url_env.is_some(),
            "{name}: configured by a URL variable"
        );
    }
}

#[serial]
#[tokio::test]
async fn a_keyless_preset_sends_no_authorization_and_reads_no_key() {
    // No placeholder key: nothing is injected, whatever the environment holds.
    let _env = EnvVar::set("OPENAI_API_KEY", Some("env-key-should-not-leak"));
    for entry in presets::entries().filter(|e| e.descriptor.auth == AuthMode::None) {
        let name = entry.descriptor.name;
        let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
        let preset = (entry.create)(PresetSettings {
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            ..with_transport(&mock)
        })
        .unwrap();
        one_chat(&preset)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        preset
            .list_models()
            .await
            .map(|_| ())
            .unwrap_or_else(|_| ()); // the canned chat reply is not a model list
        let seen = mock.seen();
        assert!(!seen[0].headers.contains_key("authorization"), "{name}");
        assert!(
            !seen[1].headers.contains_key("authorization"),
            "{name}: discovery"
        );
        assert_eq!(seen[0].url, "http://127.0.0.1:9/v1/chat/completions");
    }
}

#[tokio::test]
async fn a_keyless_preset_still_honors_an_explicit_key() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = create(
        "ollama",
        PresetSettings {
            api_key: key("proxy-token"),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&preset).await.unwrap();
    assert_eq!(
        mock.seen()[0].headers["authorization"],
        "Bearer proxy-token"
    );
}

#[serial]
#[tokio::test]
async fn a_keyed_preset_loads_its_key_per_request() {
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let preset = create("abacus", with_transport(&mock)).unwrap();
    {
        let _env = EnvVar::set("ABACUS_API_KEY", None);
        let error = one_chat(&preset).await.unwrap_err();
        assert!(
            matches!(&error, AiMuxError::LoadApiKey { env_var, .. } if env_var == "ABACUS_API_KEY"),
            "{error:?}"
        );
        assert!(mock.seen().is_empty(), "nothing was sent");
    }
    {
        let _env = EnvVar::set("ABACUS_API_KEY", Some("env-one"));
        one_chat(&preset).await.unwrap();
    }
    {
        let _env = EnvVar::set("ABACUS_API_KEY", Some("env-two"));
        one_chat(&preset).await.unwrap();
    }
    let sent: Vec<String> = mock
        .seen()
        .iter()
        .map(|s| s.headers["authorization"].clone())
        .collect();
    assert_eq!(sent, ["Bearer env-one", "Bearer env-two"]);
}

#[serial]
#[tokio::test]
async fn an_explicit_key_never_falls_back_to_the_environment() {
    let _env = EnvVar::set("ABACUS_API_KEY", Some("env-key"));
    let mock = MockFetch::new(vec![chat_ok(), chat_ok()]);
    let preset = create(
        "abacus",
        PresetSettings {
            api_key: key(""),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&preset).await.unwrap();
    assert_eq!(mock.seen()[0].headers["authorization"].trim(), "Bearer");
}

// ── base URL ─────────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn the_base_url_is_explicit_then_the_variable_then_the_default() {
    let _clear = EnvVar::set("OLLAMA_BASE_URL", None);

    // The registry default.
    let mock = MockFetch::new(vec![chat_ok()]);
    one_chat(&create("ollama", with_transport(&mock)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "http://127.0.0.1:11434/v1/chat/completions"
    );

    // The variable overrides the default; it is read per request.
    let _env = EnvVar::set("OLLAMA_BASE_URL", Some("http://ollama.internal:9999/v1/"));
    let mock = MockFetch::new(vec![chat_ok()]);
    one_chat(&create("ollama", with_transport(&mock)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "http://ollama.internal:9999/v1/chat/completions"
    );

    // An explicit URL wins over the variable.
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = create(
        "ollama",
        PresetSettings {
            base_url: Some("https://explicit.example/v1".into()),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&preset).await.unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "https://explicit.example/v1/chat/completions"
    );
}

#[serial]
#[tokio::test]
async fn an_unusable_url_variable_is_a_request_error_naming_it() {
    let _env = EnvVar::set("OLLAMA_BASE_URL", Some("ftp://nope"));
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = create("ollama", with_transport(&mock)).expect("creation reads nothing");
    let error = one_chat(&preset).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument(m) if m.contains("OLLAMA_BASE_URL")),
        "{error:?}"
    );
    assert!(mock.seen().is_empty());
}

#[test]
fn an_explicit_base_url_is_validated_at_creation() {
    for bad in [
        "",
        "gateway.example/v1",
        "ftp://gateway.example",
        "https://",
    ] {
        let result = create(
            "ollama",
            PresetSettings {
                base_url: Some(bad.to_string()),
                ..Default::default()
            },
        );
        assert!(
            matches!(result, Err(AiMuxError::InvalidArgument(_))),
            "{bad:?} must be rejected"
        );
    }
}

// ── template parameters ──────────────────────────────────────────────────────

async fn vertex_url(settings: PresetSettings) -> Result<String, AiMuxError> {
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = create(
        "vertex_ai_openai_models",
        PresetSettings {
            fetch: Some(mock.transport()),
            api_key: key("token"),
            ..settings
        },
    )?;
    one_chat(&preset).await?;
    Ok(mock.seen()[0].url.clone())
}

#[serial]
#[tokio::test]
async fn the_vertex_host_follows_the_location() {
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", None);
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);
    for (location, host) in [
        ("global", "aiplatform.googleapis.com"),
        ("us", "aiplatform.us.rep.googleapis.com"),
        ("eu", "aiplatform.eu.rep.googleapis.com"),
        ("us-central1", "us-central1-aiplatform.googleapis.com"),
        ("europe-west4", "europe-west4-aiplatform.googleapis.com"),
    ] {
        let url = vertex_url(PresetSettings {
            params: params(&[("project", "my-project"), ("location", location)]),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(
            url,
            format!(
                "https://{host}/v1/projects/my-project/locations/{location}/endpoints/openapi/chat/completions"
            )
        );
    }
}

#[serial]
#[tokio::test]
async fn template_parameters_come_from_params_then_the_environment_then_the_default() {
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", Some("env-project"));
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);

    // Environment for the project, the declared default for the location.
    let url = vertex_url(PresetSettings::default()).await.unwrap();
    assert!(
        url.starts_with(
            "https://aiplatform.googleapis.com/v1/projects/env-project/locations/global/"
        ),
        "{url}"
    );

    // An explicit parameter beats the environment.
    let url = vertex_url(PresetSettings {
        params: params(&[("project", "explicit-project")]),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(url.contains("/projects/explicit-project/"), "{url}");

    // The location variable is read too.
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", Some("us"));
    let url = vertex_url(PresetSettings::default()).await.unwrap();
    assert!(
        url.starts_with("https://aiplatform.us.rep.googleapis.com/"),
        "{url}"
    );
}

#[serial]
#[tokio::test]
async fn a_missing_template_parameter_fails_the_request_naming_it() {
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", None);
    let error = vertex_url(PresetSettings::default()).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument(m) if m.contains("`project`") && m.contains("GOOGLE_VERTEX_PROJECT")),
        "{error:?}"
    );

    // The same for a row whose placeholders have no default and no variable.
    for (name, parameter) in [
        ("snowflake", "account_identifier"),
        ("oci", "region"),
        ("neon", "branch_host"),
    ] {
        let mock = MockFetch::new(vec![chat_ok()]);
        let preset = create(
            name,
            PresetSettings {
                api_key: key("k"),
                ..with_transport(&mock)
            },
        )
        .expect("creation never fails");
        let error = one_chat(&preset).await.unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument(m) if m.contains(parameter)),
            "{name}: {error:?}"
        );
        assert!(mock.seen().is_empty(), "{name}: nothing was sent");
    }
}

#[serial]
#[tokio::test]
async fn a_template_row_works_with_its_parameter_or_a_concrete_base_url() {
    let _env = EnvVar::set("CLOUDFLARE_ACCOUNT_ID", Some("acct123"));
    let mock = MockFetch::new(vec![chat_ok(), chat_ok(), chat_ok()]);
    let from_env = create(
        "cloudflare",
        PresetSettings {
            api_key: key("k"),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&from_env).await.unwrap();
    let explicit = create(
        "cloudflare",
        PresetSettings {
            api_key: key("k"),
            params: params(&[("account_id", "other.acct")]),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&explicit).await.unwrap();
    let overridden = create(
        "cloudflare",
        PresetSettings {
            api_key: key("k"),
            base_url: Some("https://gateway.example/v1".into()),
            ..with_transport(&mock)
        },
    )
    .unwrap();
    one_chat(&overridden).await.unwrap();
    let urls: Vec<String> = mock.seen().into_iter().map(|s| s.url).collect();
    assert_eq!(
        urls,
        [
            "https://api.cloudflare.com/client/v4/accounts/acct123/ai/v1/chat/completions",
            "https://api.cloudflare.com/client/v4/accounts/other.acct/ai/v1/chat/completions",
            "https://gateway.example/v1/chat/completions",
        ]
    );
}

#[serial]
#[tokio::test]
async fn bedrock_mantle_reads_its_region_variables_in_order() {
    let _a = EnvVar::set("BEDROCK_MANTLE_REGION", None);
    let _b = EnvVar::set("AWS_REGION", None);
    let url = |settings: PresetSettings| async move {
        let mock = MockFetch::new(vec![chat_ok()]);
        let preset = create(
            "bedrock_mantle",
            PresetSettings {
                api_key: key("k"),
                fetch: Some(mock.transport()),
                ..settings
            },
        )
        .unwrap();
        one_chat(&preset).await.unwrap();
        mock.seen()[0].url.clone()
    };
    assert_eq!(
        url(PresetSettings::default()).await,
        "https://bedrock-mantle.us-east-1.api.aws/v1/chat/completions"
    );
    let _b = EnvVar::set("AWS_REGION", Some("eu-west-1"));
    assert_eq!(
        url(PresetSettings::default()).await,
        "https://bedrock-mantle.eu-west-1.api.aws/v1/chat/completions"
    );
    let _a = EnvVar::set("BEDROCK_MANTLE_REGION", Some("ap-south-1"));
    assert_eq!(
        url(PresetSettings::default()).await,
        "https://bedrock-mantle.ap-south-1.api.aws/v1/chat/completions"
    );
}

#[test]
fn undeclared_derived_and_unsafe_parameters_are_refused_at_creation() {
    let refused = |name: &str, entries: &[(&str, &str)]| {
        matches!(
            create(
                name,
                PresetSettings {
                    params: params(entries),
                    ..Default::default()
                }
            ),
            Err(AiMuxError::InvalidArgument(_))
        )
    };
    // Not declared by the row.
    assert!(refused("vertex_ai_openai_models", &[("projekt", "p")]));
    assert!(
        refused("ollama", &[("project", "p")]),
        "a row without parameters takes none"
    );
    // Derived from `location`: not the caller's to set.
    assert!(refused(
        "vertex_ai_openai_models",
        &[("host", "evil.example")]
    ));
    // A value must be a plain segment: no path, user information, port, query,
    // fragment, whitespace, placeholder or empty value.
    for bad in [
        "a/b",
        "a\\b",
        "user@host",
        "host:8080",
        "p?x=1",
        "p#f",
        "a b",
        "",
        "..",
        "{project}",
        "<p>",
        "p\n",
        "é",
    ] {
        assert!(
            refused("vertex_ai_openai_models", &[("project", bad)]),
            "project {bad:?} must be refused"
        );
        assert!(
            refused("vertex_ai_openai_models", &[("location", bad)]),
            "location {bad:?} must be refused"
        );
    }
    for good in ["my-project", "p_1", "example.com", "123456"] {
        assert!(
            !refused("vertex_ai_openai_models", &[("project", good)]),
            "{good:?}"
        );
    }
}

#[serial]
#[tokio::test]
async fn an_unsafe_value_from_the_environment_is_refused_when_the_request_resolves() {
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", Some("a/b"));
    let error = vertex_url(PresetSettings::default()).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument(m) if m.contains("plain segment")),
        "{error:?}"
    );
}

#[test]
fn no_row_leaves_a_placeholder_other_than_a_declared_parameter() {
    for entry in presets::entries() {
        let d = entry.descriptor;
        let placeholders: Vec<&str> = d
            .base_url
            .split('{')
            .skip(1)
            .map(|rest| rest.split('}').next().unwrap())
            .collect();
        let declared: Vec<&str> = d.params.iter().map(|p| p.name).collect();
        for placeholder in &placeholders {
            assert!(declared.contains(placeholder), "{}: {placeholder}", d.name);
        }
        for name in &declared {
            assert!(
                placeholders.contains(name),
                "{}: unused parameter {name}",
                d.name
            );
        }
        assert!(!d.base_url.contains(['<', '>', '$']), "{}", d.name);
    }
}

// ── the by-name entry point ──────────────────────────────────────────────────

#[serial]
#[test]
fn the_by_name_entry_point_takes_params_and_refuses_what_a_request_could_not_resolve() {
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", None);
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);
    let ok = provider(
        "vertex_ai_openai_models",
        Some("token".into()),
        "openai/gpt-oss-120b-maas",
        Some(ProviderOptions {
            params: Some(params(&[("project", "p1"), ("location", "us")])),
            ..Default::default()
        }),
    );
    assert!(ok.is_ok(), "{:?}", ok.err());
    for (name, entries) in [
        ("vertex_ai_openai_models", vec![]),
        ("vertex_ai_openai_models", vec![("project", "a/b")]),
        ("ollama", vec![("project", "p")]),
    ] {
        let result = provider(
            name,
            Some("token".into()),
            "m",
            Some(ProviderOptions {
                params: Some(params(&entries)),
                ..Default::default()
            }),
        );
        assert!(
            matches!(result, Err(AiMuxError::InvalidArgument(_))),
            "{name} {entries:?}"
        );
    }
}

#[tokio::test]
async fn organization_project_and_headers_ride_the_registry_options() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })))
        .mount(&server)
        .await;
    let model = provider(
        "abacus",
        Some("k".into()),
        "m",
        Some(ProviderOptions {
            base_url: Some(server.uri()),
            organization: Some("org-1".into()),
            project: Some("proj-1".into()),
            headers: Some(params(&[("x-extra", "1")])),
            ..Default::default()
        }),
    )
    .unwrap();
    model.do_generate(&hello()).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(headers.get("openai-organization").unwrap(), "org-1");
    assert_eq!(headers.get("openai-project").unwrap(), "proj-1");
    assert_eq!(headers.get("x-extra").unwrap(), "1");
    assert_eq!(headers.get("authorization").unwrap(), "Bearer k");
}

#[test]
fn removed_options_are_rejected_not_ignored() {
    for json in [r#"{"max_retries": 2}"#, r#"{"body_overrides": {"a": 1}}"#] {
        assert!(
            serde_json::from_str::<ProviderOptions>(json).is_err(),
            "{json}"
        );
    }
    assert!(serde_json::from_str::<ProviderOptions>(r#"{"params": {"region": "x"}}"#).is_ok());
}

#[test]
fn settings_debug_never_prints_secrets() {
    let printed = format!(
        "{:?}",
        PresetSettings {
            api_key: key("sk-very-secret"),
            ..Default::default()
        }
    );
    assert!(!printed.contains("secret"), "{printed}");
}

fn _assert_presets_are_shareable() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<aimux_providers::preset::PresetProvider>();
}

fn _provider_trait_is_implemented(p: &aimux_providers::preset::PresetProvider) -> &dyn Provider {
    p
}
