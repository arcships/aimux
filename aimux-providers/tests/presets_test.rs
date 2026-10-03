//! Runtime registry validation and by-name preset behavior.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::json;
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use aimux_providers::preset::{self, PresetProvider};
use aimux_providers::{PresetSettings, provider};

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
    CallOptions::new(vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("hi")],
        ..Default::default()
    }])
}

#[serial]
#[test]
fn every_registry_row_loads_and_creates_without_environment() {
    let entries: Vec<_> = preset::entries().collect();
    assert_eq!(entries.len(), 283);
    assert!(
        preset::names()
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    let mut variables = BTreeSet::new();
    for entry in &entries {
        let d = entry.descriptor;
        variables.extend((!d.env_var.is_empty()).then_some(d.env_var));
        variables.extend(d.base_url_env);
        variables.extend(d.params.iter().flat_map(|param| param.env.iter().copied()));
    }
    let _guards: Vec<_> = variables
        .into_iter()
        .map(|var| EnvVar::set(var, None))
        .collect();
    for entry in entries {
        PresetProvider::create(entry.descriptor, PresetSettings::default())
            .unwrap_or_else(|error| panic!("{}: {error}", entry.descriptor.name));
    }
}

#[tokio::test]
async fn a_keyless_row_sends_no_authorization() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let preset = PresetProvider::create(
        preset::lookup("ollama").unwrap().descriptor,
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
        matches!(provider("no-such-preset", Some("k".into()), "m", None),
        Err(AiMuxError::NoSuchProvider { provider_id }) if provider_id == "no-such-preset")
    );
}

#[tokio::test]
async fn a_template_parameter_must_be_a_plain_host_segment() {
    let mock = MockFetch::new(vec![chat_ok()]);
    let result = PresetProvider::create(
        preset::lookup("neon").unwrap().descriptor,
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
