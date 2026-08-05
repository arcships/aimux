//! aimux-node: Node.js binding (napi-rs v3, native path).
//!
//! This is the **flagship binding** — directly uses aimux-providers, bypassing
//! aimux-ffi. napi-rs maps Rust async to JS Promise/AsyncIterator, giving
//! native-TS-quality DX.
//!
//! Design:
//! - Provider model instances live as JS objects backed by Rust `Arc<dyn LanguageModel>`.
//! - `generateText` returns a Promise<string> (JSON-serialized GenerateTextResult).
//! - `streamText` returns an AsyncGenerator yielding StreamPart JSON strings.
//! - The TS wrapper layer (index.ts) parses JSON into typed objects.

mod multimodal;
pub use multimodal::*;

use std::future::Future;
use std::sync::Arc;

use aimux_core::generate::{generate_text, stream_text, GenerateTextOptions};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use napi::bindgen_prelude::*;
use napi_derive::napi;

// ─────────────────────────────────────────────────────────────────────────────
// Model — a provider model instance accessible from JS.
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct Model {
    inner: Arc<dyn LanguageModel>,
}

/// A bridge from a JS `AbortSignal` to the core's runtime cancellation.
///
/// napi's own `AbortSignal` is not `Send` (it holds `Rc`), so it cannot be a
/// parameter of an async napi method. This class is created synchronously
/// (registering the abort callback on the JS thread) and then passed to
/// `generateText` / `streamText` — it only holds an `Arc` and is `Send`.
///
/// ```ts
/// const bridge = signal ? new AbortBridge(signal) : undefined
/// await model.generateText(prompt, options, bridge)
/// ```
#[napi]
pub struct AbortBridge {
    signal: Arc<aimux_core::shared::AbortSignal>,
}

#[napi]
impl AbortBridge {
    #[napi(constructor)]
    pub fn new(signal: napi::bindgen_prelude::AbortSignal) -> Self {
        let core = aimux_core::shared::AbortSignal::new();
        let watcher = core.clone();
        signal.on_abort(move || watcher.abort());
        Self {
            signal: Arc::new(core),
        }
    }

    /// Returns `true` once the underlying JS signal has been aborted.
    #[napi]
    pub fn aborted(&self) -> bool {
        self.signal.is_aborted()
    }
}

impl AbortBridge {
    fn core_signal(&self) -> aimux_core::shared::AbortSignal {
        (*self.signal).clone()
    }
}

#[napi]
impl Model {
    /// Generate text (non-streaming).
    ///
    /// `prompt` — a JSON string: bare prompt (`"text"` or `[{...}]`) or `{"prompt": ...}`.
    /// `options` — optional JSON-serialized `GenerateTextOptions`.
    /// `bridge` — optional `AbortBridge` (wrap a JS `AbortSignal` in one);
    /// aborting the signal cancels the call.
    /// Returns a JSON-serialized `GenerateTextResult`.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate_text(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> Result<String> {
        let parsed_prompt = parse_prompt(&prompt)?;
        let mut opts = parse_opts(options.as_deref())?;
        opts.abort_signal = bridge.map(|b| b.core_signal());

        let result = generate_text(&*self.inner, parsed_prompt, opts)
            .await
            .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;

        serde_json::to_string(&result)
            .map_err(|e| Error::from_reason(format!("[Json] serialize result: {e}")))
    }

    /// Stream text from the model.
    ///
    /// Returns an `AsyncGenerator<string>` yielding `StreamPart` JSON strings.
    /// Use `for await (const part of model.streamText(...))` to consume.
    #[napi(ts_return_type = "Promise<AsyncGenerator<string>>")]
    pub async fn stream_text(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> Result<StreamTextGenerator> {
        let model = self.inner.clone();
        // Extract the core signal on the napi thread; it is `Send` and can
        // move into the spawned task.
        let abort_signal = bridge.map(|b| b.core_signal());

        let (tx, rx) = tokio::sync::mpsc::channel::<std::result::Result<String, String>>(64);

        // Spawn the stream-driving task immediately on napi's tokio runtime.
        napi::tokio::spawn(async move {
            let prompt = match parse_prompt(&prompt) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Err(format!("[InvalidPrompt] invalid prompt: {e}"))).await;
                    return;
                }
            };
            let mut opts = match parse_opts(options.as_deref()) {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(Err(format!("invalid options: {e}"))).await;
                    return;
                }
            };
            opts.abort_signal = abort_signal;

            match stream_text(&*model, prompt, opts).await {
                Ok(stream_result) => {
                    use futures::StreamExt;
                    let mut stream = stream_result.stream;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(part) => {
                                let json = serde_json::to_string(&part)
                                    .unwrap_or_else(|_| "{}".to_string());
                                if tx.send(Ok(json)).await.is_err() {
                                    break; // receiver dropped (JS stopped iterating)
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(format!("[{}] {e}", e.error_type()))).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("[{}] {e}", e.error_type()))).await;
                }
            }
        });

        Ok(StreamTextGenerator {
            rx: std::sync::Arc::new(tokio::sync::Mutex::new(Some(rx))),
        })
    }
}

/// AsyncGenerator that yields StreamPart JSON strings.
///
/// The stream is started eagerly in `stream_text()` — a tokio task drives
/// the Rust Stream and sends each chunk through a channel. `next()` simply
/// receives from the channel.
#[napi(async_iterator)]
pub struct StreamTextGenerator {
    rx: std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<std::result::Result<String, String>>>>>,
}

#[napi]
impl AsyncGenerator for StreamTextGenerator {
    type Yield = String;
    type Next = ();
    type Return = ();

    fn next(
        &mut self,
        _value: Option<Self::Next>,
    ) -> impl Future<Output = Result<Option<Self::Yield>>> + Send + 'static {
        // Clone the Arc so the async block owns it (no borrow of self).
        let rx = self.rx.clone();
        async move {
            let mut guard = rx.lock().await;
            match guard.as_mut() {
                Some(rx) => {
                    match rx.recv().await {
                        Some(Ok(json)) => Ok(Some(json)),
                        Some(Err(e)) => Err(Error::from_reason(e)),
                        None => Ok(None), // stream finished
                    }
                }
                None => Ok(None), // already exhausted
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider constructors — factory functions exposed to JS.
// ─────────────────────────────────────────────────────────────────────────────

/// Optional provider configuration accepted by factory functions.
///
/// Passed as the 3rd argument to `openai()` / `anthropic()` / `deepseek()`.
/// For backward compatibility a bare string is still accepted (treated as
/// `baseUrl`). See RFC-0017.
#[napi(object)]
pub struct ProviderConfig {
    /// Base URL for API calls (e.g. a relay/proxy endpoint).
    pub base_url: Option<String>,
    /// Extra HTTP headers merged into every request, as a JSON object string
    /// (e.g. `'{"X-Custom":"value"}'`).
    pub headers: Option<String>,
    /// OpenAI organization ID (sent via `OpenAI-Organization` header).
    pub organization: Option<String>,
    /// OpenAI project ID (sent via `OpenAI-Project` header).
    pub project: Option<String>,
    /// Override the provider's retry count. `0` disables retries.
    pub max_retries: Option<u32>,
    /// Provider-level request body overrides as a JSON string (deep-merged
    /// into every request). Per-call `bodyOverrides` in GenerateTextOptions
    /// takes precedence. Pass a JSON object string, e.g.
    /// `'{"enable_thinking": false}'`.
    pub body_overrides: Option<String>,
}

/// Convert a ProviderConfig (from JS) into OpenAIConfig builder steps.
fn apply_provider_config_openai(
    mut config: aimux_providers::openai::OpenAIConfig,
    cfg: &ProviderConfig,
) -> aimux_providers::openai::OpenAIConfig {
    if let Some(url) = &cfg.base_url {
        config = config.with_base_url(url);
    }
    if let Some(ref json_str) = cfg.headers {
        if let Ok(h) = serde_json::from_str::<std::collections::HashMap<String, String>>(json_str) {
            config = config.with_headers(h);
        }
    }
    if let Some(ref org) = cfg.organization {
        config = config.with_org_id(org);
    }
    if let Some(ref proj) = cfg.project {
        config = config.with_project(proj);
    }
    if let Some(max) = cfg.max_retries {
        config = config.with_retry_config(aimux_provider_utils::RetryConfig {
            max_retries: max,
            ..aimux_provider_utils::RetryConfig::default()
        });
    }
    if let Some(ref json_str) = cfg.body_overrides {
        if let Ok(overrides) = serde_json::from_str::<serde_json::Value>(json_str) {
            config = config.with_body_overrides(overrides);
        }
    }
    config
}

/// 初始化全局日志（RFC-0014）。幂等：多次调用无副作用；宿主已自建
/// subscriber 时 no-op。级别：off|error|warn|info|debug|trace（空串回退
/// warn）。`AIMUX_LOG` / `AIMUX_LOG_LEVEL` 环境变量优先级更高。
/// 日志输出到 stderr。
#[napi]
pub fn init_logging(level: String) {
    let level = if level.trim().is_empty() { "warn".to_string() } else { level };
    aimux_providers::init_logging(&level);
}

/// Register the global session store (RFC-0024). Replaces any previous one.
/// Until called, calls are not grouped and the session query functions return
/// empty results.
#[napi]
pub fn init_session_store() {
    aimux_core::session::init_session_store(std::sync::Arc::new(
        aimux_core::session::SessionStore::new(),
    ));
}

/// Enable/disable the global session inferer (RFC-0024, opt-in, off by
/// default). Explicit `sessionId` values always win regardless of this.
#[napi]
pub fn init_session_infer(enabled: bool) {
    aimux_core::session::init_session_infer(enabled);
}

/// Query: all calls of a session (RFC-0024), as a JSON-serialized
/// `SessionCall[]` (ordered by step). Empty array if the session is unknown
/// or no store is registered.
#[napi]
pub fn session_calls(session_id: String) -> Result<String> {
    let calls = aimux_core::session::session_calls(&session_id);
    serde_json::to_string(&calls)
        .map_err(|e| Error::from_reason(format!("[Json] serialize sessionCalls: {e}")))
}

/// Query: all known sessions (RFC-0024), as a JSON-serialized `SessionView[]`.
#[napi]
pub fn list_sessions() -> Result<String> {
    let views = aimux_core::session::list_sessions();
    serde_json::to_string(&views)
        .map_err(|e| Error::from_reason(format!("[Json] serialize listSessions: {e}")))
}

/// Create an OpenAI model instance.
#[napi]
pub async fn openai(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};

    let mut cfg = OpenAIConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            cfg = apply_provider_config_openai(cfg, &opts);
        }
        None => {}
    }
    let provider = OpenAIProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create an Anthropic model instance.
#[napi]
pub async fn anthropic(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};

    let mut cfg = AnthropicConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
            if let Some(ref json_str) = opts.headers {
                if let Ok(h) = serde_json::from_str::<std::collections::HashMap<String, String>>(json_str) {
                    cfg = cfg.with_headers(h);
                }
            }
            if let Some(max) = opts.max_retries {
                cfg = cfg.with_retry_config(aimux_provider_utils::RetryConfig {
                    max_retries: max,
                    ..aimux_provider_utils::RetryConfig::default()
                });
            }
            if let Some(ref json_str) = opts.body_overrides {
                if let Ok(overrides) = serde_json::from_str::<serde_json::Value>(json_str) {
                    cfg = cfg.with_body_overrides(overrides);
                }
            }
        }
        None => {}
    }
    let provider = AnthropicProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a DeepSeek model instance (registry-backed since RFC-0017 phase 4).
#[napi]
pub async fn deepseek(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    let options = provider_options_from_config(config)?;
    let model = aimux_providers::provider("deepseek", Some(api_key), &model_id, options)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a Google Gemini language model instance.
///
/// Google speaks the native `generateContent` protocol (not OpenAI-compatible),
/// so it is **not** registry-backed — `provider("google", ...)` fails with
/// `UnknownProvider`. This factory is the only entry point.
#[napi]
pub async fn google(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::google::{GoogleConfig, GoogleProvider};

    let mut cfg = GoogleConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
        }
        None => {}
    }
    let provider = GoogleProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a Cohere language model instance.
#[napi]
pub async fn cohere(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::cohere::{CohereConfig, CohereProvider};

    let mut cfg = CohereConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
        }
        None => {}
    }
    let provider = CohereProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a Mistral language model instance.
#[napi]
pub async fn mistral(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::mistral::{MistralConfig, MistralProvider};

    let mut cfg = MistralConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
        }
        None => {}
    }
    let provider = MistralProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create an xAI language model instance.
#[napi]
pub async fn xai(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::xai::{XAIConfig, XAIProvider};

    let mut cfg = XAIConfig::new(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
        }
        None => {}
    }
    let provider = XAIProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a Bedrock language model instance (AWS SigV4 credentials).
#[napi]
pub async fn bedrock(
    access_key_id: String,
    secret_access_key: String,
    region: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::bedrock::{BedrockProvider, BedrockProviderConfig};

    let mut cfg = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
    if let Some(cfg_config) = config {
        match cfg_config {
            Either::A(url) => {
                cfg = cfg.with_base_url(url);
            }
            Either::B(opts) => {
                if let Some(url) = &opts.base_url {
                    cfg = cfg.with_base_url(url);
                }
            }
        }
    }
    let provider = BedrockProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a Vertex AI language model instance (GCP bearer token).
#[napi]
pub async fn vertex(
    access_token: String,
    project: String,
    location: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::vertex::{VertexProvider, VertexProviderConfig};

    let mut cfg = VertexProviderConfig::new(access_token, project, location);
    if let Some(cfg_config) = config {
        match cfg_config {
            Either::A(url) => {
                cfg = cfg.with_base_url(url);
            }
            Either::B(opts) => {
                if let Some(url) = &opts.base_url {
                    cfg = cfg.with_base_url(url);
                }
            }
        }
    }
    let provider = VertexProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create an Anthropic-on-AWS language model instance (API key + region).
#[napi]
pub async fn anthropic_aws(
    api_key: String,
    region: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::anthropic_aws::{AnthropicAwsProvider, AnthropicAwsProviderConfig};

    let mut cfg = AnthropicAwsProviderConfig::with_api_key(api_key, region);
    if let Some(cfg_config) = config {
        match cfg_config {
            Either::A(url) => {
                cfg = cfg.with_base_url(url);
            }
            Either::B(opts) => {
                if let Some(url) = &opts.base_url {
                    cfg = cfg.with_base_url(url);
                }
            }
        }
    }
    let provider = AnthropicAwsProvider::new(cfg);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create an Azure OpenAI language model instance (API key + resource name).
///
/// The deployment name is passed as `model_id`; `api_version` is optional.
#[napi]
pub async fn azure(
    api_key: String,
    resource_name: String,
    deployment: String,
    api_version: Option<String>,
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::azure::{AzureConfig, AzureProvider};

    let mut cfg = AzureConfig::new().with_api_key(api_key);
    match config {
        Some(Either::A(url)) => {
            cfg = cfg.with_base_url(url);
        }
        Some(Either::B(opts)) => {
            if let Some(url) = &opts.base_url {
                cfg = cfg.with_base_url(url);
            }
        }
        None => {}
    }
    if let Some(version) = api_version {
        if !version.is_empty() {
            cfg = cfg.with_api_version(version);
        }
    }
    if !resource_name.is_empty() {
        cfg = cfg.with_resource_name(resource_name);
    }
    let provider = AzureProvider::new(cfg)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    let model = provider
        .language_model(&deployment)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a language model from the built-in registry by provider name
/// (RFC-0017 phase 4). `api_key` may be empty/null to read the provider's env
/// var from the registry entry.
#[napi]
pub async fn provider(
    name: String,
    api_key: Option<String>,
    model_id: String,
    config: Option<ProviderConfig>,
) -> Result<Model> {
    let options = match config {
        Some(cfg) => provider_options_from_config(Some(Either::B(cfg)))?,
        None => None,
    };
    let model = aimux_providers::provider(&name, api_key, &model_id, options)
        .map_err(|e| Error::from_reason(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Build `ProviderOptions` from a Node `ProviderConfig` (3rd factory arg).
fn provider_options_from_config(
    config: Option<Either<String, ProviderConfig>>,
) -> Result<Option<aimux_providers::ProviderOptions>> {
    let opts = match config {
        None => None,
        Some(Either::A(url)) => Some(aimux_providers::ProviderOptions {
            base_url: Some(url),
            ..Default::default()
        }),
        Some(Either::B(cfg)) => {
            let mut o = aimux_providers::ProviderOptions::default();
            if let Some(url) = cfg.base_url {
                o.base_url = Some(url);
            }
            if let Some(ref json_str) = cfg.headers {
                if let Ok(h) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(json_str)
                {
                    o.headers = Some(h);
                }
            }
            if let Some(org) = cfg.organization {
                o.organization = Some(org);
            }
            if let Some(proj) = cfg.project {
                o.project = Some(proj);
            }
            if let Some(max) = cfg.max_retries {
                o.max_retries = Some(max);
            }
            if let Some(ref json_str) = cfg.body_overrides {
                if let Ok(overrides) = serde_json::from_str::<serde_json::Value>(json_str) {
                    o.body_overrides = Some(overrides);
                }
            }
            Some(o)
        }
    };
    Ok(opts)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_prompt(json: &str) -> Result<ModelPrompt> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::from_reason(format!("[Json] invalid prompt JSON: {e}")))?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
        .map_err(|e| Error::from_reason(format!("[Json] invalid prompt: {e}")))
}

fn parse_opts(json: Option<&str>) -> Result<GenerateTextOptions> {
    match json {
        None => Ok(GenerateTextOptions::default()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return Ok(GenerateTextOptions::default());
            }
            serde_json::from_str(s)
                .map_err(|e| Error::from_reason(format!("[Json] invalid options JSON: {e}")))
        }
    }
}
