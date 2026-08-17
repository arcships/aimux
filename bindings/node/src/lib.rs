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

mod error;
mod multimodal;
pub use multimodal::*;

use std::future::Future;
use std::sync::Arc;

use crate::error::{
    AimuxResult, BindingError, AiMuxBindingError, MResult, StreamItem, parse_wire_json,
    serialize_result,
};
use aimux_core::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_object, generate_text, generate_text_as_openai, stream_text,
    stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use aimux_core::openai_output::OpenAiStreamOptions;
use napi::bindgen_prelude::*;
use napi_derive::napi;

// ─────────────────────────────────────────────────────────────────────────────
// Model — a provider model instance accessible from JS.
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct Model {
    inner: Arc<dyn LanguageModel>,
    /// Probe store — `Some` only for traced models (RFC-0015).
    trace_store: Option<Arc<aimux_core::trace::RingTraceStore>>,
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
    /// Wrap this model in a cache-probe layer (RFC-0015). The returned
    /// model records fingerprints/verdicts on every call and exposes the
    /// `traceAggregate` / `traceSessionChain` / `traceExportJsonl` /
    /// `traceClear` queries.
    #[napi]
    pub fn trace(&self) -> Model {
        let store = Arc::new(aimux_core::trace::RingTraceStore::new());
        let layer = aimux_core::trace::TraceLayer::new(self.inner.clone(), store.clone());
        Model {
            inner: Arc::new(layer),
            trace_store: Some(store),
        }
    }

    /// Wrap this model in a cache-probe layer with the built-in rules
    /// auditor (RFC-0015 §4). `strict` = strict mode (self-hosted single
    /// instance); `false` = shared mode (safe default).
    #[napi]
    pub fn trace_audited(&self, strict: bool) -> Model {
        let store = Arc::new(aimux_core::trace::RingTraceStore::new());
        let layer = aimux_core::trace::TraceLayer::new(self.inner.clone(), store.clone())
            .with_rules_auditor(strict);
        Model {
            inner: Arc::new(layer),
            trace_store: Some(store),
        }
    }

    /// Aggregated probe statistics (RFC-0015 §5.3), filtered by a JSON
    /// `TraceFilter` (optional). Returns a JSON `TraceStats[]` string.
    #[napi]
    pub fn trace_aggregate(&self, filter_json: Option<String>) -> AimuxResult<String> {
        AimuxResult((|| -> crate::error::MResult<String> {
            let Some(store) = &self.trace_store else {
                return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                    "model is not traced; call trace() first".into(),
                )));
            };
            let filter = match filter_json {
                Some(f) => parse_wire_json("filter", &f)?,
                None => Default::default(),
            };
            serialize_result(&store.aggregate(&filter))
        })())
    }

    /// One session's chain view (RFC-0015 §5.3). Returns a JSON
    /// `SessionChainView` string or an error when the session is unknown.
    #[napi]
    pub fn trace_session_chain(&self, session_id: String) -> AimuxResult<String> {
        AimuxResult((|| -> crate::error::MResult<String> {
            let Some(store) = &self.trace_store else {
                return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                    "model is not traced; call trace() first".into(),
                )));
            };
            let view = store.session_chain(&session_id).ok_or_else(|| {
                AiMuxBindingError::from(&AiMuxError::InvalidArgument("unknown session".into()))
            })?;
            serialize_result(&view)
        })())
    }

    /// One session's per-step cache-hit trajectory (RFC-0024 §4.3). Returns
    /// a JSON array of `SessionStepStat` (empty when the session is unknown
    /// or the model is not traced).
    #[napi]
    pub fn trace_session_trajectory(&self, session_id: String) -> AimuxResult<String> {
        AimuxResult((|| -> crate::error::MResult<String> {
            let Some(store) = &self.trace_store else {
                return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                    "model is not traced; call trace() first".into(),
                )));
            };
            let stats = store.session_cache_trajectory(&session_id);
            serialize_result(&stats)
        })())
    }

    /// Export all probe records as JSONL (one `TraceRecord` per line).
    #[napi]
    pub fn trace_export_jsonl(&self) -> AimuxResult<String> {
        AimuxResult((|| -> crate::error::MResult<String> {
            let Some(store) = &self.trace_store else {
                return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                    "model is not traced; call trace() first".into(),
                )));
            };
            let mut buf = Vec::new();
            store
                .export_jsonl(&mut buf)
                .map_err(|e| BindingError::ResultSerialization {
                    message: format!("export: {e}"),
                })?;
            String::from_utf8(buf).map_err(|e| {
                BindingError::ResultSerialization {
                    message: format!("utf8: {e}"),
                }
                .into()
            })
        })())
    }

    /// Clear all probe records of this traced model.
    #[napi]
    pub fn trace_clear(&self) {
        if let Some(store) = &self.trace_store {
            store.clear();
        }
    }

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
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let parsed_prompt = parse_prompt(&prompt)?;
                let mut opts = parse_opts(options.as_deref())?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = generate_text(&*self.inner, parsed_prompt, opts)
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;

                serialize_result(&result)
            }
            .await;
            __r
        })
    }

    /// Generate a structured JSON object from the model (M12, RFC-0016).
    ///
    /// Same signature as [`Model::generate_text`]; returns a JSON-serialized
    /// `GenerateObjectResult`. Pass `response_format: { "Json": { ... } }`
    /// via `options` for schema control; the function applies JSON repair
    /// before parsing.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate_object(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let parsed_prompt = parse_prompt(&prompt)?;
                let mut opts = parse_opts(options.as_deref())?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = generate_object(&*self.inner, parsed_prompt, opts)
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;

                serialize_result(&result)
            }
            .await;
            __r
        })
    }

    /// Consume a stream to completion and return the aggregated result
    /// (M11, RFC-0016).
    ///
    /// Drives `streamText` to completion and returns a JSON-serialized
    /// `StreamTextResultAggregated` (the fully-consumed stream summary).
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn consume_stream_text(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let parsed_prompt = parse_prompt(&prompt)?;
                let mut opts = parse_opts(options.as_deref())?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let stream_result = stream_text(&*self.inner, parsed_prompt, opts)
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;
                let aggregated = stream_result
                    .consume()
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;

                serialize_result(&aggregated)
            }
            .await;
            __r
        })
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
    ) -> AimuxResult<StreamTextGenerator> {
        AimuxResult({
            let __r: crate::error::MResult<StreamTextGenerator> = async {
                let model = self.inner.clone();
                // Extract the core signal on the napi thread; it is `Send` and can
                // move into the spawned task.
                let abort_signal = bridge.map(|b| b.core_signal());

                let (tx, rx) =
                    tokio::sync::mpsc::channel::<std::result::Result<String, AiMuxBindingError>>(64);

                // Spawn the stream-driving task immediately on napi's tokio runtime.
                napi::tokio::spawn(async move {
                    let prompt = match parse_prompt(&prompt) {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    };
                    let mut opts = match parse_opts(options.as_deref()) {
                        Ok(o) => o,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
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
                                        // Not serializable → end the stream with the binding's
                                        // ResultSerialization, never a silent "{}".
                                        let json = match serde_json::to_string(&part) {
                                            Ok(j) => j,
                                            Err(e) => {
                                                let _ = tx
                                                    .send(Err(AiMuxBindingError::from(
                                                        BindingError::ResultSerialization {
                                                            message: format!("stream part: {e}"),
                                                        },
                                                    )))
                                                    .await;
                                                break;
                                            }
                                        };
                                        if tx.send(Ok(json)).await.is_err() {
                                            break; // receiver dropped (JS stopped iterating)
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(AiMuxBindingError::from(&e))).await;
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(AiMuxBindingError::from(&e))).await;
                        }
                    }
                });

                Ok(StreamTextGenerator {
                    rx: std::sync::Arc::new(tokio::sync::Mutex::new(Some(rx))),
                })
            }
            .await;
            __r
        })
    }

    /// Generate text and return an OpenAI Chat Completion (non-streaming).
    ///
    /// Returns a JSON-serialized `ChatCompletion`. Works with any provider.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate_text_as_openai(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let parsed_prompt = parse_prompt(&prompt)?;
                let mut opts = parse_opts(options.as_deref())?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = generate_text_as_openai(&*self.inner, parsed_prompt, opts)
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;

                serialize_result(&result)
            }
            .await;
            __r
        })
    }

    /// Stream text as OpenAI Chat Completion chunks.
    ///
    /// Returns an `AsyncGenerator<string>` yielding `ChatCompletionChunk` JSON
    /// strings. Works with any provider.
    #[napi(ts_return_type = "Promise<AsyncGenerator<string>>")]
    pub async fn stream_text_as_openai(
        &self,
        prompt: String,
        options: Option<String>,
        bridge: Option<&AbortBridge>,
    ) -> AimuxResult<StreamTextGenerator> {
        AimuxResult({
            let __r: crate::error::MResult<StreamTextGenerator> = async {
                let model = self.inner.clone();
                let abort_signal = bridge.map(|b| b.core_signal());

                let (tx, rx) =
                    tokio::sync::mpsc::channel::<std::result::Result<String, AiMuxBindingError>>(64);

                napi::tokio::spawn(async move {
                    let prompt = match parse_prompt(&prompt) {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    };
                    let mut opts = match parse_opts(options.as_deref()) {
                        Ok(o) => o,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    };

                    // Extract OpenAI stream options from providerOptions.openai.stream_options.
                    let stream_options = opts
                        .provider_options
                        .as_ref()
                        .and_then(|po| po.get("openai"))
                        .and_then(|o| o.get("stream_options"))
                        .cloned()
                        .map(|v| OpenAiStreamOptions {
                            include_usage: v
                                .get("include_usage")
                                .and_then(|b| b.as_bool())
                                .unwrap_or(true),
                            include_reasoning: v
                                .get("include_reasoning")
                                .and_then(|b| b.as_bool())
                                .unwrap_or(true),
                        })
                        .unwrap_or_default();

                    opts.abort_signal = abort_signal;

                    match stream_text_as_openai(&*model, prompt, opts, stream_options).await {
                        Ok(stream_result) => {
                            use futures::StreamExt;
                            let mut stream = stream_result.stream;
                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(chunk) => {
                                        // Not serializable → end the stream with the binding's
                                        // ResultSerialization, never a silent "{}".
                                        let json = match serde_json::to_string(&chunk) {
                                            Ok(j) => j,
                                            Err(e) => {
                                                let _ = tx
                                                    .send(Err(AiMuxBindingError::from(
                                                        BindingError::ResultSerialization {
                                                            message: format!("stream chunk: {e}"),
                                                        },
                                                    )))
                                                    .await;
                                                break;
                                            }
                                        };
                                        if tx.send(Ok(json)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(AiMuxBindingError::from(&e))).await;
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(AiMuxBindingError::from(&e))).await;
                        }
                    }
                });

                Ok(StreamTextGenerator {
                    rx: std::sync::Arc::new(tokio::sync::Mutex::new(Some(rx))),
                })
            }
            .await;
            __r
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
    rx: std::sync::Arc<
        tokio::sync::Mutex<
            Option<tokio::sync::mpsc::Receiver<std::result::Result<String, AiMuxBindingError>>>,
        >,
    >,
}

#[napi]
impl AsyncGenerator for StreamTextGenerator {
    type Yield = StreamItem;
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
                        Some(Ok(json)) => Ok(Some(StreamItem::json(json))),
                        // Converting Failure to a napi value (JS thread, Env
                        // present) rejects `next()` with a structured error.
                        Some(Err(m)) => Ok(Some(StreamItem::failure(m))),
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
) -> MResult<aimux_providers::openai::OpenAIConfig> {
    if let Some(url) = &cfg.base_url {
        config = config.with_base_url(url);
    }
    if let Some(ref json_str) = cfg.headers {
        let h: std::collections::HashMap<String, String> =
            parse_wire_json("config.headers", json_str)?;
        config = config.with_headers(h);
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
        let overrides: serde_json::Value = parse_wire_json("config.bodyOverrides", json_str)?;
        config = config.with_body_overrides(overrides);
    }
    Ok(config)
}

/// 初始化全局日志（RFC-0014）。幂等：多次调用无副作用；宿主已自建
/// subscriber 时 no-op。级别：off|error|warn|info|debug|trace（空串回退
/// warn）。`AIMUX_LOG` / `AIMUX_LOG_LEVEL` 环境变量优先级更高。
/// 日志输出到 stderr。
#[napi]
pub fn init_logging(level: String) {
    let level = if level.trim().is_empty() {
        "warn".to_string()
    } else {
        level
    };
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

// ─────────────────────────────────────────────────────────────────────────────
// External provider config (RFC-0020)
// ─────────────────────────────────────────────────────────────────────────────

/// Register external OpenAI-compatible providers from a JSON config string
/// (RFC-0020). Entries override same-named built-ins or add new ones.
///
/// `configJson` shape: `{ "providers": [ { "name": "...", "base_url": "...", ... } ] }`.
/// Malformed JSON text throws a plain `Error` (napi `InvalidArg`); a
/// well-formed document the registry rejects (bad base_url scheme, empty
/// name, unsupported protocol, wrong shape) throws `InvalidArgumentError`.
#[napi]
pub fn register_providers(config_json: String) -> error::AimuxResult<()> {
    AimuxResult((|| -> error::MResult<()> {
        // Malformed text is the binding's finding; the registry reports a
        // schema mismatch as `JsonParse`, which is core's InvalidArgument
        // here (the text already parsed) — JsonParse is for provider responses.
        let _: serde_json::Value = parse_wire_json("config_json", &config_json)?;
        aimux_providers::load_providers_from_json(&config_json).map_err(|e| match e {
            AiMuxError::JsonParse(m) => crate::error::AiMuxBindingError::from(
                &AiMuxError::InvalidArgument(format!("config_json: {m}")),
            ),
            e => crate::error::AiMuxBindingError::from(&e),
        })
    })())
}

/// Set the global proxy configuration (M6, RFC-0016). Must be called before
/// the first `generateText` / `streamText` call; a no-op if the shared HTTP
/// client is already initialised (the shared client is lazily built on first
/// use and locked for the process lifetime).
///
/// `configJson` shape: `{ "http_url": "...", "https_url": "...", "all_url":
/// "...", "no_proxy": "..." }` (all fields optional; omitting all is equivalent
/// to relying on the `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`
/// env vars).
///
/// Throws on invalid JSON.
#[napi]
pub fn init_proxy(config_json: String) -> error::AimuxResult<()> {
    AimuxResult((|| -> error::MResult<()> {
        let config: aimux_provider_utils::ProxyConfig =
            parse_wire_json("config_json", &config_json)?;
        // `init_proxy` returns false when the shared client is already up; treat
        // that as success (idempotent) so callers don't reason about ordering.
        let _ = aimux_provider_utils::init_proxy(config);
        Ok(())
    })())
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording + mock replay (RFC-0023)
// ─────────────────────────────────────────────────────────────────────────────

/// 启动录制(RFC-0023 P1/P2):把完整 `Recording` 写 JSONL 到 `{dir}/recordings.jsonl`
/// (目录自动创建)。录制 **opt-in**;再次调用(不同 dir)替换 recorder。
/// Throws `RecordingError` (`code` = `Init` / `OpenFile` / `Spawn`) when the
/// recorder cannot be set up; the previous recorder, if any, stays in place.
#[napi]
pub fn init_recording(dir: String) -> AimuxResult<()> {
    AimuxResult(
        aimux_core::recording::JsonlRecorder::try_new(dir)
            .map(|rec| {
                aimux_core::recording::init_recording(Some(std::sync::Arc::new(rec)));
            })
            .map_err(AiMuxBindingError::from),
    )
}

/// 启动内存有界录制(RFC-0023 P6):`RingRecorder`,FIFO 淘汰,丢弃计数可查。
///
/// `cap` 可省略:省略时使用库默认容量(等价于 FFI `aimux_init_recording_ring_default()`;
/// 本绑定直接依赖 aimux-core 而非 aimux-ffi,故调用等价 core API `RingRecorder::default()`)。
/// 显式传 `cap == 0` 抛错(与其余语言一致),`cap > 0` 启动指定容量的有界 ring。
#[napi]
pub fn init_recording_ring(cap: Option<u32>) -> error::AimuxResult<()> {
    let result: error::MResult<()> = match cap {
        // 省略 cap:库默认容量(镜像 FFI default 变体)。
        None => {
            aimux_core::recording::init_recording(Some(std::sync::Arc::new(
                aimux_core::recording::RingRecorder::default(),
            )));
            Ok(())
        }
        // 显式 cap == 0:拒绝(与 Kotlin/Java/Python 一致)。
        Some(0) => Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
            "initRecordingRing: cap must be > 0".into(),
        ))),
        // 显式 cap > 0:指定容量的有界 ring。
        Some(c) => {
            aimux_core::recording::init_recording(Some(std::sync::Arc::new(
                aimux_core::recording::RingRecorder::with_capacity(c as usize),
            )));
            Ok(())
        }
    };
    result.into()
}

/// 停止录制:全局 recorder = None(新调用不再录制)。
#[napi]
pub fn recording_stop() {
    aimux_core::recording::init_recording(None);
}

/// 刷盘全局 recorder(阻塞至 JSONL 落盘;ring 模式为 no-op)。
#[napi]
pub fn recording_flush() {
    if let Some(rec) = aimux_core::recording::recorder() {
        rec.flush();
    }
}

/// Flush the global recorder and **report write failures**: throws
/// `RecordingError` (`code` = `WriterGone` / `FlushTimeout` / `Write`) when the
/// data could not be confirmed on disk. `RecordingError` is its own class,
/// not an `AimuxError`. Resolves normally when nothing is recording. The
/// legacy `recordingFlush` stays and never reports.
#[napi]
pub fn recording_try_flush() -> AimuxResult<()> {
    let Some(rec) = aimux_core::recording::recorder() else {
        return AimuxResult(Ok(()));
    };
    AimuxResult(rec.try_flush().map_err(AiMuxBindingError::from))
}

/// 从录制 JSONL 创建 mock 回放模型(RFC-0023 P3):按输入匹配录制响应,
/// **不发真实 API**。返回的 `Model` 可用于 `generateText` / `streamText`。
#[napi]
pub fn mock_replay(recordings_jsonl: String) -> AimuxResult<Model> {
    AimuxResult((|| -> crate::error::MResult<Model> {
        let mut recordings: Vec<aimux_core::recording::Recording> = Vec::new();
        for (idx, line) in recordings_jsonl.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Same split as parse_wire_json, with the line number in whichever
            // message actually reaches JS (binding errors render from `binding`,
            // not `message`).
            let rec: aimux_core::recording::Recording =
                serde_json::from_str(line).map_err(|e| {
                    let message = format!("recordings line {}: {e}", idx + 1);
                    match e.classify() {
                        serde_json::error::Category::Data => {
                            AiMuxBindingError::from(&AiMuxError::InvalidArgument(message))
                        }
                        _ => BindingError::InvalidWireJson {
                            argument: "recordings_jsonl",
                            message,
                        }
                        .into(),
                    }
                })?;
            recordings.push(rec);
        }
        if recordings.is_empty() {
            return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                "no recordings".into(),
            )));
        }
        let model = aimux_core::replay::MockReplayModel::new(
            recordings[0].provider.provider.clone(),
            recordings[0].provider.model_id.clone(),
            recordings,
        );
        Ok(Model {
            inner: std::sync::Arc::new(model),
            trace_store: None,
        })
    })())
}

#[napi]
pub fn router(models: Vec<&Model>, config_json: Option<String>) -> AimuxResult<Model> {
    AimuxResult((|| -> crate::error::MResult<Model> {
        if models.is_empty() {
            return Err(AiMuxBindingError::from(&AiMuxError::InvalidArgument(
                "router: models must be non-empty".into(),
            )));
        }
        let children: Vec<Arc<dyn LanguageModel>> =
            models.iter().map(|m| m.inner.clone()).collect();
        let cfg: RouterFfiConfig = match config_json.as_deref() {
            Some(json) => parse_wire_json("config_json", json)?,
            None => RouterFfiConfig::default(),
        };
        let router: Box<dyn aimux_core::router::Router> = match cfg.router.as_deref() {
            Some("weighted") => {
                let weights = cfg.weights.unwrap_or_else(|| vec![1.0; children.len()]);
                Box::new(aimux_core::router::WeightedRouter::new(weights))
            }
            _ => Box::new(aimux_core::router::RuleRouter),
        };
        let fallback = if cfg.fallback.as_deref() == Some("none") {
            aimux_core::router::FallbackPolicy::None
        } else {
            aimux_core::router::FallbackPolicy::OnError
        };
        let router_cfg = aimux_core::router::RouterConfig {
            provider_name: cfg.provider_name.unwrap_or_else(|| "router".into()),
            model_id: cfg.model_id.unwrap_or_else(|| "router".into()),
        };
        let model = aimux_core::router::RouterModel::new(children, router, fallback, router_cfg);
        Ok(Model {
            inner: Arc::new(model),
            trace_store: None,
        })
    })())
}

#[napi]
pub fn moa(
    references: Vec<&Model>,
    aggregator: &Model,
    config_json: Option<String>,
) -> AimuxResult<Model> {
    AimuxResult((|| -> crate::error::MResult<Model> {
        let refs: Vec<Arc<dyn LanguageModel>> =
            references.iter().map(|m| m.inner.clone()).collect();
        let cfg: aimux_core::moa::MoaConfig = match config_json.as_deref() {
            Some(json) => parse_wire_json("config_json", json)?,
            None => aimux_core::moa::MoaConfig::default(),
        };
        let model = aimux_core::moa::MoaModel::new(refs, aggregator.inner.clone(), cfg);
        Ok(Model {
            inner: Arc::new(model),
            trace_store: None,
        })
    })())
}

/// Lenient router config (mirrors the FFI-side shape; all fields optional).
#[derive(Default, serde::Deserialize)]
struct RouterFfiConfig {
    router: Option<String>,
    weights: Option<Vec<f64>>,
    fallback: Option<String>,
    provider_name: Option<String>,
    model_id: Option<String>,
}

/// Query: all calls of a session (RFC-0024), as a JSON-serialized
/// `SessionCall[]` (ordered by step). Empty array if the session is unknown
/// or no store is registered.
#[napi]
pub fn session_calls(session_id: String) -> AimuxResult<String> {
    AimuxResult((|| -> crate::error::MResult<String> {
        let calls = aimux_core::session::session_calls(&session_id);
        serialize_result(&calls)
    })())
}

/// Query: all known sessions (RFC-0024), as a JSON-serialized `SessionView[]`.
#[napi]
pub fn list_sessions() -> AimuxResult<String> {
    AimuxResult((|| -> crate::error::MResult<String> {
        let views = aimux_core::session::list_sessions();
        serialize_result(&views)
    })())
}

/// Create an OpenAI model instance.
#[napi]
pub async fn openai(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
            use aimux_core::provider::Provider;
            use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};

            let mut cfg = OpenAIConfig::new(api_key);
            match config {
                Some(Either::A(url)) => {
                    cfg = cfg.with_base_url(url);
                }
                Some(Either::B(opts)) => {
                    cfg = apply_provider_config_openai(cfg, &opts)?;
                }
                None => {}
            }
            let provider = OpenAIProvider::new(cfg);
            let model = provider
                .language_model(&model_id)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create an Anthropic model instance.
#[napi]
pub async fn anthropic(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                        let h: std::collections::HashMap<String, String> =
                            parse_wire_json("config.headers", json_str)?;
                        cfg = cfg.with_headers(h);
                    }
                    if let Some(max) = opts.max_retries {
                        cfg = cfg.with_retry_config(aimux_provider_utils::RetryConfig {
                            max_retries: max,
                            ..aimux_provider_utils::RetryConfig::default()
                        });
                    }
                    if let Some(ref json_str) = opts.body_overrides {
                        let overrides: serde_json::Value =
                            parse_wire_json("config.bodyOverrides", json_str)?;
                        cfg = cfg.with_body_overrides(overrides);
                    }
                }
                None => {}
            }
            let provider = AnthropicProvider::new(cfg);
            let model = provider
                .language_model(&model_id)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create a DeepSeek model instance (registry-backed since RFC-0017 phase 4).
#[napi]
pub async fn deepseek(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
            let options = provider_options_from_config(config)?;
            let model = aimux_providers::provider("deepseek", Some(api_key), &model_id, options)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create a Google Gemini language model instance.
///
/// Google speaks the native `generateContent` protocol (not OpenAI-compatible),
/// so it is **not** registry-backed — `provider("google", ...)` fails with
/// `NoSuchProvider`. This factory is the only entry point.
#[napi]
pub async fn google(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create a Cohere language model instance.
#[napi]
pub async fn cohere(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create a Mistral language model instance.
#[napi]
pub async fn mistral(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create an xAI language model instance.
#[napi]
pub async fn xai(
    api_key: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
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
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
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
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

/// Create an Anthropic-on-AWS language model instance (API key + region).
#[napi]
pub async fn anthropic_aws(
    api_key: String,
    region: String,
    model_id: String,
    config: Option<Either<String, ProviderConfig>>,
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
            use aimux_core::provider::Provider;
            use aimux_providers::anthropic_aws::{
                AnthropicAwsProvider, AnthropicAwsProviderConfig,
            };

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
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
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
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
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
            let provider = AzureProvider::new(cfg).map_err(|e| AiMuxBindingError::from(&e))?;
            let model = provider
                .language_model(&deployment)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
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
) -> AimuxResult<Model> {
    AimuxResult({
        let __r: crate::error::MResult<Model> = async {
            let options = match config {
                Some(cfg) => provider_options_from_config(Some(Either::B(cfg)))?,
                None => None,
            };
            let model = aimux_providers::provider(&name, api_key, &model_id, options)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(Model {
                inner: Arc::from(model),
                trace_store: None,
            })
        }
        .await;
        __r
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider handles (RFC-0027) — createProvider / listModels / model
// ─────────────────────────────────────────────────────────────────────────────

/// A provider handle — created by `createProvider`, supports `listModels()`
/// (runtime discovery) and `model()` (build a model from a discovered id).
///
/// ```ts
/// const p = await createProvider('deepseek', apiKey)
/// const models = await p.listModels()
/// const model = await p.model(models[0].id)
/// const result = await generateText(model, 'Hello')
/// ```
#[napi]
pub struct ProviderHandle {
    inner: Arc<dyn aimux_core::provider::Provider>,
}

#[napi]
impl ProviderHandle {
    /// List models available on this provider (runtime discovery via the
    /// provider's `/models` endpoint). Returns a JSON array of sparse
    /// `RuntimeModel` (id / owned_by / created) — **no community enrichment**.
    /// To supplement with model specs (context length, capabilities, reasoning),
    /// call `getModelSpecs` separately and merge in the host (RFC-0027).
    #[napi]
    pub async fn list_models(&self) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let models = self
                    .inner
                    .list_models()
                    .await
                    .map_err(|e| AiMuxBindingError::from(&e))?;
                serialize_result(&models)
            }
            .await;
            __r
        })
    }

    /// Build a language model from a discovered model id. Returns a `Model`
    /// usable with `generateText` / `streamText`.
    #[napi]
    pub async fn model(&self, model_id: String) -> AimuxResult<Model> {
        AimuxResult({
            let __r: crate::error::MResult<Model> = async {
                let m = self
                    .inner
                    .language_model(&model_id)
                    .map_err(|e| AiMuxBindingError::from(&e))?;
                Ok(Model {
                    inner: Arc::from(m),
                    // A model built from a provider handle starts untraced; enable
                    // probing via `Model::trace()` (same as Python binding).
                    trace_store: None,
                })
            }
            .await;
            __r
        })
    }
}

/// Create a **provider handle** (RFC-0027) for a registry-backed provider.
///
/// Unlike `provider()` (which binds to a single model_id), this returns a
/// `ProviderHandle` that supports `listModels()` and `model()`.
///
/// `api_key` may be empty/null to read the provider's env var from the
/// registry entry.
#[napi]
pub async fn create_provider(
    name: String,
    api_key: Option<String>,
    config: Option<ProviderConfig>,
) -> AimuxResult<ProviderHandle> {
    AimuxResult({
        let __r: crate::error::MResult<ProviderHandle> = async {
            let options = match config {
                Some(cfg) => provider_options_from_config(Some(Either::B(cfg)))?,
                None => None,
            };
            let p = aimux_providers::provider_handle(&name, api_key, options)
                .map_err(|e| AiMuxBindingError::from(&e))?;
            Ok(ProviderHandle {
                inner: Arc::from(p),
            })
        }
        .await;
        __r
    })
}

/// Fetch the community model catalogue (anya2a). Returns a JSON-serialized
/// `Catalogue` (provider → model_id → ModelSpec). Thin fetch — no caching.
/// `source_url` may be null for the default endpoint.
#[napi]
pub async fn get_model_specs(source_url: Option<String>) -> AimuxResult<String> {
    AimuxResult({
        let __r: crate::error::MResult<String> = async {
            let cat = aimux_providers::get_model_specs(source_url.as_deref())
                .await
                .map_err(|e| AiMuxBindingError::from(&e))?;
            serialize_result(&cat)
        }
        .await;
        __r
    })
}

/// Build `ProviderOptions` from a Node `ProviderConfig` (3rd factory arg).
fn provider_options_from_config(
    config: Option<Either<String, ProviderConfig>>,
) -> MResult<Option<aimux_providers::ProviderOptions>> {
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
                o.headers = Some(parse_wire_json("config.headers", json_str)?);
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
                o.body_overrides = Some(parse_wire_json("config.bodyOverrides", json_str)?);
            }
            Some(o)
        }
    };
    Ok(opts)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_prompt(json: &str) -> MResult<ModelPrompt> {
    let value: serde_json::Value = parse_wire_json("prompt_json", json)?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
        // Well-formed JSON of the wrong shape: core validation, not a binding
        // transport failure.
        .map_err(|e| {
            AiMuxBindingError::from(&AiMuxError::InvalidArgument(format!("invalid prompt: {e}")))
        })
}

fn parse_opts(json: Option<&str>) -> MResult<GenerateTextOptions> {
    match json {
        None => Ok(GenerateTextOptions::default()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return Ok(GenerateTextOptions::default());
            }
            parse_wire_json("opts_json", s)
        }
    }
}
