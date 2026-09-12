//! aimux-python: Python binding (PyO3, native path).
//!
//! Directly uses aimux-providers, bypassing aimux-ffi.
//! PyO3 maps Rust async to Python via a tokio runtime + async generator.

// pyo3 0.22 macros generate unsafe-op-in-unsafe-fn calls that trigger
// edition-2024 lint. Suppress until pyo3 0.23+ lands.
#![allow(unsafe_op_in_unsafe_fn)]

mod error;
mod multimodal;
pub use multimodal::*;

use std::sync::Arc;

use crate::error::{
    AiMuxBindingError, BindingError, binding_py_err, serialize_result, to_py_err, wire_json,
};
use aimux_core::AiMuxError;
use aimux_core::generate::{
    GenerateTextOptions, generate_object, generate_text, generate_text_as_openai, stream_text,
    stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use aimux_core::openai_output::OpenAiStreamOptions;
use pyo3::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Global tokio runtime
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("failed to build tokio runtime"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Model — a provider model instance accessible from Python.
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
struct Model {
    inner: Arc<dyn LanguageModel>,
    /// Probe store — `Some` only for traced models (RFC-0015).
    trace_store: Option<Arc<aimux_core::trace::RingTraceStore>>,
}

#[pymethods]
impl Model {
    /// Wrap this model in a cache-probe layer (RFC-0015) WITHOUT an
    /// auditor (records fingerprints only; verdicts stay None).
    fn trace(&self) -> PyResult<Model> {
        let store = Arc::new(aimux_core::trace::RingTraceStore::new());
        let layer = aimux_core::trace::TraceLayer::new(self.inner.clone(), store.clone());
        Ok(Model {
            inner: Arc::new(layer),
            trace_store: Some(store),
        })
    }

    /// Wrap this model in a probe layer with the built-in rules auditor.
    /// `strict=True` = strict mode (self-hosted single instance);
    /// `strict=False` = shared mode (safe default).
    #[pyo3(signature = (strict=false))]
    fn trace_audited(&self, strict: bool) -> PyResult<Model> {
        let store = Arc::new(aimux_core::trace::RingTraceStore::new());
        let layer = aimux_core::trace::TraceLayer::new(self.inner.clone(), store.clone())
            .with_rules_auditor(strict);
        Ok(Model {
            inner: Arc::new(layer),
            trace_store: Some(store),
        })
    }

    /// Aggregated probe statistics (RFC-0015 §5.3), filtered by an optional
    /// JSON `TraceFilter`. Returns a JSON `TraceStats[]` string.
    #[pyo3(signature = (filter_json=None))]
    fn trace_aggregate(&self, filter_json: Option<&str>) -> PyResult<String> {
        let Some(store) = &self.trace_store else {
            return Err(to_py_err(&AiMuxError::InvalidArgument(
                "model is not traced; call trace() first".into(),
            )));
        };
        let filter = match filter_json {
            Some(f) => wire_json("filter_json", f)?,
            None => Default::default(),
        };
        serialize_result(&store.aggregate(&filter))
    }

    /// One session's chain view. Returns a JSON `SessionChainView` string.
    fn trace_session_chain(&self, session_id: &str) -> PyResult<String> {
        let Some(store) = &self.trace_store else {
            return Err(to_py_err(&AiMuxError::InvalidArgument(
                "model is not traced; call trace() first".into(),
            )));
        };
        let view = store
            .session_chain(session_id)
            .ok_or_else(|| to_py_err(&AiMuxError::InvalidArgument("unknown session".into())))?;
        serialize_result(&view)
    }

    /// Export all probe records as JSONL (one `TraceRecord` per line).
    fn trace_export_jsonl(&self) -> PyResult<String> {
        let Some(store) = &self.trace_store else {
            return Err(to_py_err(&AiMuxError::InvalidArgument(
                "model is not traced; call trace() first".into(),
            )));
        };
        let mut buf = Vec::new();
        store.export_jsonl(&mut buf).map_err(|e| {
            binding_py_err(&BindingError::ResultSerialization {
                message: format!("export: {e}"),
            })
        })?;
        String::from_utf8(buf).map_err(|e| {
            binding_py_err(&BindingError::ResultSerialization {
                message: format!("utf8: {e}"),
            })
        })
    }

    /// Clear all probe records of this traced model.
    fn trace_clear(&self) {
        if let Some(store) = &self.trace_store {
            store.clear();
        }
    }

    /// Generate text (non-streaming).
    ///
    /// prompt_json: JSON string (bare prompt or {"prompt": ...})
    /// opts_json: optional JSON-serialized GenerateTextOptions
    /// Returns JSON-serialized GenerateTextResult.
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn generate_text(&self, prompt_json: &str, opts_json: Option<&str>) -> PyResult<String> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;

        let rt = runtime();
        let result = rt.block_on(async move { generate_text(&*self.inner, prompt, opts).await });

        match result {
            Ok(r) => serialize_result(&r),
            Err(e) => Err(to_py_err(&e)),
        }
    }

    /// Generate a structured JSON object from the model (M12, RFC-0016).
    ///
    /// prompt_json: JSON string (bare prompt or {"prompt": ...})
    /// opts_json: optional JSON-serialized GenerateTextOptions
    /// Returns JSON-serialized GenerateObjectResult. Pass
    /// `response_format: { "Json": { ... } }` via opts_json for schema
    /// control; the function applies JSON repair before parsing.
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn generate_object(&self, prompt_json: &str, opts_json: Option<&str>) -> PyResult<String> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;

        let rt = runtime();
        let result = rt.block_on(async move { generate_object(&*self.inner, prompt, opts).await });

        match result {
            Ok(r) => serialize_result(&r),
            Err(e) => Err(to_py_err(&e)),
        }
    }

    /// Consume a stream to completion and return the aggregated result
    /// (M11, RFC-0016).
    ///
    /// prompt_json: JSON string (bare prompt or {"prompt": ...})
    /// opts_json: optional JSON-serialized GenerateTextOptions
    /// Returns JSON-serialized StreamTextResultAggregated.
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn consume_stream_text(&self, prompt_json: &str, opts_json: Option<&str>) -> PyResult<String> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;

        let rt = runtime();
        let result = rt.block_on(async move {
            let stream_result = stream_text(&*self.inner, prompt, opts).await?;
            stream_result.consume().await
        });

        match result {
            Ok(r) => serialize_result(&r),
            Err(e) => Err(to_py_err(&e)),
        }
    }

    /// Stream text from the model.
    ///
    /// Returns a StreamIterator that yields StreamPart JSON strings.
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn stream_text(&self, prompt_json: &str, opts_json: Option<&str>) -> PyResult<StreamIterator> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;
        let model = self.inner.clone();

        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<String, crate::error::AiMuxBindingError>>(64);

        rt_spawn(async move {
            match stream_text(&*model, prompt, opts).await {
                Ok(stream_result) => {
                    use futures::StreamExt;
                    let mut stream = stream_result.stream;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(part) => {
                                // A part that cannot be serialized ends the stream with the
                                // binding's ResultSerialization — never a silent "{}".
                                let json = match serde_json::to_string(&part) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(
                                                crate::error::BindingError::ResultSerialization {
                                                    message: format!("stream part: {e}"),
                                                }
                                                .into(),
                                            ))
                                            .await;
                                        break;
                                    }
                                };
                                if tx.send(Ok(json)).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) if e.is_recoverable_stream_error() => {
                                // Core keeps the stream alive across a malformed frame;
                                // deliver it as a StreamPart::Error data item and keep
                                // pumping.
                                match serde_json::to_string(
                                    &aimux_core::stream_part::StreamPart::Error { error: e },
                                ) {
                                    Ok(json) => {
                                        if tx.send(Ok(json)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e.into())).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into())).await;
                }
            }
        });

        Ok(StreamIterator { rx })
    }

    /// Generate text as an OpenAI Chat Completion (non-streaming, RFC-0026).
    ///
    /// prompt_json: JSON string (bare prompt or {"prompt": ...})
    /// opts_json: optional JSON-serialized GenerateTextOptions
    /// Returns JSON-serialized ChatCompletion (OpenAI `chat.completion` object).
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn generate_text_as_openai(
        &self,
        prompt_json: &str,
        opts_json: Option<&str>,
    ) -> PyResult<String> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;

        let rt = runtime();
        let result =
            rt.block_on(async move { generate_text_as_openai(&*self.inner, prompt, opts).await });

        match result {
            Ok(r) => serialize_result(&r),
            Err(e) => Err(to_py_err(&e)),
        }
    }

    /// Stream text as OpenAI Chat Completion chunks (RFC-0026).
    ///
    /// Returns a StreamIterator that yields ChatCompletionChunk JSON strings.
    /// Stream options (`include_usage`, `include_reasoning`) are read from
    /// `opts.provider_options.openai.stream_options` (both default to `true`).
    #[pyo3(signature = (prompt_json, opts_json=None))]
    fn stream_text_as_openai(
        &self,
        prompt_json: &str,
        opts_json: Option<&str>,
    ) -> PyResult<StreamIterator> {
        let prompt = parse_prompt(prompt_json)?;
        let opts = parse_opts(opts_json)?;
        let model = self.inner.clone();

        // Extract OpenAI stream options from opts.provider_options.openai.stream_options
        // (same logic as aimux-ffi's stream_text_as_openai_with_signal).
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

        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<String, crate::error::AiMuxBindingError>>(64);

        rt_spawn(async move {
            match stream_text_as_openai(&*model, prompt, opts, stream_options).await {
                Ok(stream_result) => {
                    use futures::StreamExt;
                    let mut stream = stream_result.stream;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(chunk) => {
                                let json = match serde_json::to_string(&chunk) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(
                                                crate::error::BindingError::ResultSerialization {
                                                    message: format!("stream chunk: {e}"),
                                                }
                                                .into(),
                                            ))
                                            .await;
                                        break;
                                    }
                                };
                                if tx.send(Ok(json)).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) if e.is_recoverable_stream_error() => {
                                // Consumers type this path as ChatCompletionChunk; the
                                // error cannot ride it — skip and keep pumping (full
                                // fidelity lives on the StreamPart path).
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e.into())).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into())).await;
                }
            }
        });

        Ok(StreamIterator { rx })
    }
}

/// Python iterator that yields StreamPart JSON strings from a tokio channel.
#[pyclass]
struct StreamIterator {
    rx: tokio::sync::mpsc::Receiver<Result<String, crate::error::AiMuxBindingError>>,
}

#[pymethods]
impl StreamIterator {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        // Block on the next channel item, allowing other Python threads to run.
        let item = py.allow_threads(|| runtime().block_on(self.rx.recv()));

        match item {
            Some(Ok(json)) => Ok(Some(json.to_object(py).into())),
            Some(Err(f)) => Err(f.to_py_err()),
            None => Ok(None), // stream finished
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider constructors
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn openai(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};

    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create an Anthropic model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn anthropic(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};

    let mut config = AnthropicConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = AnthropicProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a DeepSeek model instance (registry-backed since RFC-0017 phase 4).
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn deepseek(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    let options = base_url.map(|url| aimux_providers::ProviderOptions {
        base_url: Some(url.to_string()),
        ..Default::default()
    });
    let model = aimux_providers::provider("deepseek", Some(api_key.to_string()), model_id, options)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a Google Gemini language model instance (native `generateContent`
/// protocol — not OpenAI-compatible, so not registry-backed).
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn google(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::google::{GoogleConfig, GoogleProvider};

    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = GoogleProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a Cohere language model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn cohere(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::cohere::{CohereConfig, CohereProvider};

    let mut config = CohereConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = CohereProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a Mistral language model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn mistral(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::mistral::{MistralConfig, MistralProvider};

    let mut config = MistralConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = MistralProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create an xAI language model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn xai(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::xai::{XAIConfig, XAIProvider};

    let mut config = XAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = XAIProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a Bedrock language model instance (AWS SigV4 credentials).
#[pyfunction]
#[pyo3(signature = (access_key_id, secret_access_key, region, model_id, base_url=None))]
fn bedrock(
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::bedrock::{BedrockProvider, BedrockProviderConfig};

    let mut config = BedrockProviderConfig::new(access_key_id, secret_access_key, region);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = BedrockProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a Vertex AI language model instance (GCP bearer token).
#[pyfunction]
#[pyo3(signature = (access_token, project, location, model_id, base_url=None))]
fn vertex(
    access_token: &str,
    project: &str,
    location: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::vertex::{VertexProvider, VertexProviderConfig};

    let mut config = VertexProviderConfig::new(access_token, project, location);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = VertexProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create an Anthropic-on-AWS language model instance (API key + region).
#[pyfunction]
#[pyo3(signature = (api_key, region, model_id, base_url=None))]
fn anthropic_aws(
    api_key: &str,
    region: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::anthropic_aws::{AnthropicAwsProvider, AnthropicAwsProviderConfig};

    let mut config = AnthropicAwsProviderConfig::with_api_key(api_key, region);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = AnthropicAwsProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create an Azure OpenAI language model instance (API key + resource name).
///
/// The deployment name is passed as `model_id`; `api_version` is optional.
#[pyfunction]
#[pyo3(signature = (api_key, resource_name, deployment, api_version=None, base_url=None))]
fn azure(
    api_key: &str,
    resource_name: &str,
    deployment: &str,
    api_version: Option<&str>,
    base_url: Option<&str>,
) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::azure::{AzureConfig, AzureProvider};

    let mut config = AzureConfig::new().with_api_key(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    if let Some(version) = api_version {
        if !version.is_empty() {
            config = config.with_api_version(version);
        }
    }
    if !resource_name.is_empty() {
        config = config.with_resource_name(resource_name);
    }
    let provider = AzureProvider::new(config).map_err(|e| to_py_err(&e))?;
    let model = provider
        .language_model(deployment)
        .map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

/// Create a language model from the built-in registry by provider name
/// (RFC-0017 phase 4). `api_key=None` reads the provider's env var.
/// `config_json` is a serialized `ProviderOptions` object (`base_url` /
/// `headers` / `organization` / `project` / `max_retries` /
/// `body_overrides`); the `base_url` parameter wins over the JSON field.
#[pyfunction]
#[pyo3(signature = (name, api_key, model_id, base_url=None, config_json=None))]
fn provider(
    name: &str,
    api_key: Option<String>,
    model_id: &str,
    base_url: Option<&str>,
    config_json: Option<&str>,
) -> PyResult<Model> {
    let mut options: Option<aimux_providers::ProviderOptions> = match config_json {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => Some(wire_json("config_json", s)?),
        _ => None,
    };
    if let Some(url) = base_url {
        options.get_or_insert_with(Default::default).base_url = Some(url.to_string());
    }
    let model =
        aimux_providers::provider(name, api_key, model_id, options).map_err(|e| to_py_err(&e))?;
    Ok(Model {
        inner: Arc::from(model),
        trace_store: None,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider handles (RFC-0027) — createProvider / listModels / model
// ─────────────────────────────────────────────────────────────────────────────

/// A provider handle — created by `create_provider`, supports `list_models()`
/// (runtime discovery) and `model()` (build a model from a discovered id).
#[pyclass]
struct ProviderHandle {
    inner: Arc<dyn aimux_core::provider::Provider>,
}

#[pymethods]
impl ProviderHandle {
    /// List models available on this provider (runtime discovery + anya2a spec).
    /// Returns a JSON array of RuntimeModel.
    fn list_models(&self) -> PyResult<String> {
        let rt = runtime();
        let models = rt
            .block_on(async { self.inner.list_models().await })
            .map_err(|e| to_py_err(&e))?;
        serialize_result(&models)
    }

    /// Build a language model from a discovered model id.
    fn model(&self, model_id: &str) -> PyResult<Model> {
        let m = self
            .inner
            .language_model(model_id)
            .map_err(|e| to_py_err(&e))?;
        Ok(Model {
            inner: Arc::from(m),
            trace_store: None,
        })
    }
}

/// Create a **provider handle** (RFC-0027) for a registry-backed provider.
///
/// Unlike `provider()` (which binds to a single model_id), this returns a
/// `ProviderHandle` that supports `list_models()` and `model()`.
#[pyfunction]
#[pyo3(signature = (name, api_key=None, base_url=None, config_json=None))]
fn create_provider(
    name: &str,
    api_key: Option<String>,
    base_url: Option<&str>,
    config_json: Option<&str>,
) -> PyResult<ProviderHandle> {
    let mut options: Option<aimux_providers::ProviderOptions> = match config_json {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => Some(wire_json("config_json", s)?),
        _ => None,
    };
    if let Some(url) = base_url {
        options.get_or_insert_with(Default::default).base_url = Some(url.to_string());
    }
    let p = aimux_providers::provider_handle(name, api_key, options).map_err(|e| to_py_err(&e))?;
    Ok(ProviderHandle {
        inner: Arc::from(p),
    })
}

/// Fetch the community model catalogue (RFC-0027) and return it as a JSON
/// string (serialized `Catalogue`). `source_url` defaults to the anya2a
/// `dist/all.json`.
#[pyfunction]
fn get_model_specs(source_url: Option<&str>) -> PyResult<String> {
    let catalogue = runtime()
        .block_on(async { aimux_providers::get_model_specs(source_url).await })
        .map_err(|e| to_py_err(&e))?;
    serialize_result(&catalogue)
}

// ─────────────────────────────────────────────────────────────────────────────
// Module
// ─────────────────────────────────────────────────────────────────────────────

/// 初始化全局日志（RFC-0014）。幂等：多次调用无副作用；宿主已自建
/// subscriber 时 no-op。级别：off|error|warn|info|debug|trace（空串回退
/// warn）。`AIMUX_LOG` / `AIMUX_LOG_LEVEL` 环境变量优先级更高。
/// 日志输出到 stderr。
#[pyfunction]
fn init_logging(level: &str) {
    let level = if level.trim().is_empty() {
        "warn"
    } else {
        level
    };
    aimux_providers::init_logging(level);
}

/// Register external OpenAI-compatible providers from a JSON config string
/// (RFC-0020). Entries override same-named built-ins or add new ones.
///
/// `config_json` shape: `{ "providers": [ { "name": "...", "base_url": "...", ... } ] }`.
/// Malformed JSON text raises `ValueError`; a well-formed document the
/// registry rejects (bad base_url scheme, empty name, unsupported protocol,
/// wrong shape) raises `InvalidArgumentError`.
#[pyfunction]
fn register_providers(config_json: &str) -> PyResult<()> {
    let _: serde_json::Value = wire_json("config_json", config_json)?;
    aimux_providers::load_providers_from_json(config_json).map_err(|e| match e {
        AiMuxError::JsonParse(m) => {
            to_py_err(&AiMuxError::InvalidArgument(format!("config_json: {m}")))
        }
        e => to_py_err(&e),
    })
}

/// Set the global proxy configuration (M6, RFC-0016). Must be called before the
/// first `generate_text` / `stream_text` call; a no-op if the shared HTTP
/// client is already initialised.
///
/// `config_json` shape: `{ "http_url": "...", "https_url": "...", "all_url":
/// "...", "no_proxy": "..." }` (all fields optional). Raises `ValueError` on
/// malformed JSON, `AimuxError` on a bad value.
#[pyfunction]
fn init_proxy(config: &str) -> PyResult<()> {
    let proxy_config: aimux_provider_utils::ProxyConfig = wire_json("config", config)?;
    // `init_proxy` returns false when the shared client is already up; treat
    // that as success (idempotent).
    let _ = aimux_provider_utils::init_proxy(proxy_config);
    Ok(())
}

/// Register the global session store (RFC-0024). Replaces any previous one.
/// Until called, calls are not grouped and the query functions return empty
/// results.
#[pyfunction]
fn init_session_store() {
    aimux_core::session::init_session_store(std::sync::Arc::new(
        aimux_core::session::SessionStore::new(),
    ));
}

/// Enable/disable the global session inferer (RFC-0024, opt-in, off by
/// default). Explicit `session_id` values always win regardless.
#[pyfunction]
fn init_session_infer(enabled: bool) {
    aimux_core::session::init_session_infer(enabled);
}

/// Query: all calls of a session (RFC-0024), as a JSON-serialized
/// `SessionCall[]` (ordered by step). Empty array if unknown / no store.
#[pyfunction]
fn session_calls(session_id: &str) -> PyResult<String> {
    serialize_result(&aimux_core::session::session_calls(session_id))
}

/// Query: all known sessions (RFC-0024), as a JSON-serialized `SessionView[]`.
#[pyfunction]
fn list_sessions() -> PyResult<String> {
    serialize_result(&aimux_core::session::list_sessions())
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording + mock replay (RFC-0023)
// ─────────────────────────────────────────────────────────────────────────────

/// 启动录制(RFC-0023):把完整 `Recording` 写 JSONL 到 `{dir}/recordings.jsonl`
/// (目录自动创建)。录制 opt-in;再次调用(不同 dir)替换 recorder。
/// Raises `RecordingError` (``code`` "Init" / "OpenFile" / "Spawn") when the
/// recorder cannot be set up; the previous recorder, if any, stays in place.
#[pyfunction]
fn init_recording(dir: &str) -> PyResult<()> {
    let rec = aimux_core::recording::JsonlRecorder::try_new(dir.to_string())
        .map_err(|e| AiMuxBindingError::from(e).to_py_err())?;
    aimux_core::recording::init_recording(Some(std::sync::Arc::new(rec)));
    Ok(())
}

/// 启动内存有界录制(RFC-0023 P6):FIFO ring,丢弃计数可查。
///
/// `cap` 可省略:省略时使用库默认容量(等价于 FFI `aimux_init_recording_ring_default()`;
/// 本绑定直接依赖 aimux-core 而非 aimux-ffi,故调用等价 core API `RingRecorder::default()`)。
/// 显式传 `cap == 0` 报错(保持与各绑定统一的"传 0 报错"语义)。
#[pyfunction]
#[pyo3(signature = (cap=None))]
fn init_recording_ring(cap: Option<u64>) -> PyResult<()> {
    match cap {
        // 省略 cap:库默认容量(镜像 FFI default 变体)。
        None => aimux_core::recording::init_recording(Some(std::sync::Arc::new(
            aimux_core::recording::RingRecorder::default(),
        ))),
        // 显式 cap == 0:报错(保持统一语义)。
        Some(0) => {
            return Err(to_py_err(&AiMuxError::InvalidArgument(
                "init_recording_ring: cap must be > 0".into(),
            )));
        }
        // 显式 cap > 0:指定容量的有界 ring。
        Some(c) => aimux_core::recording::init_recording(Some(std::sync::Arc::new(
            aimux_core::recording::RingRecorder::with_capacity(c as usize),
        ))),
    }
    Ok(())
}

/// 停止录制:全局 recorder = None(新调用不再录制)。
#[pyfunction]
fn recording_stop() {
    aimux_core::recording::init_recording(None);
}

/// 刷盘全局 recorder(阻塞至 JSONL 落盘;ring 模式 no-op)。
#[pyfunction]
fn recording_flush() {
    if let Some(rec) = aimux_core::recording::recorder() {
        rec.flush();
    }
}

/// Flush the global recorder and **report write failures**: raises
/// `RecordingError` (``code`` is "WriterGone" / "FlushTimeout" / "Write") when
/// the data could not be confirmed on disk. `RecordingError` is its own
/// exception type, not an `AimuxError`. Returns normally when nothing is
/// recording. The legacy `recording_flush` stays and never reports.
#[pyfunction]
fn recording_try_flush() -> PyResult<()> {
    let Some(rec) = aimux_core::recording::recorder() else {
        return Ok(());
    };
    rec.try_flush()
        .map_err(|e| AiMuxBindingError::from(e).to_py_err())
}

/// 从录制 JSONL 创建 mock 回放 model(RFC-0023 P3):按输入匹配录制响应,
/// 不发真实 API。返回的 Model 可用于 generate_text / stream_text。
#[pyfunction]
fn mock_replay(recordings_jsonl: &str) -> PyResult<Model> {
    let mut recordings: Vec<aimux_core::recording::Recording> = Vec::new();
    for (idx, line) in recordings_jsonl.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Same split as everywhere else: text that does not parse is the
        // binding's (ValueError); a parsed line with the wrong shape is core's
        // InvalidArgument. Both name the line.
        let rec: aimux_core::recording::Recording = serde_json::from_str(line).map_err(|e| {
            let msg = format!("recordings line {}: {e}", idx + 1);
            match e.classify() {
                serde_json::error::Category::Data => to_py_err(&AiMuxError::InvalidArgument(msg)),
                _ => binding_py_err(&BindingError::InvalidWireJson {
                    argument: "recordings_jsonl",
                    message: msg,
                }),
            }
        })?;
        recordings.push(rec);
    }
    if recordings.is_empty() {
        return Err(to_py_err(&AiMuxError::InvalidArgument(
            "no recordings".into(),
        )));
    }
    let model = aimux_core::replay::MockReplayModel::new(
        recordings[0].provider.provider.clone(),
        recordings[0].provider.model_id.clone(),
        recordings,
    );
    Ok(Model {
        inner: Arc::new(model),
        trace_store: None,
    })
}

/// Create a RouterModel (RFC-0021) over the given child models. The returned
/// model routes each call to one child and falls back across the rest on error
/// (per config_json). `models` must be non-empty.
///
/// `config_json` (optional): {"router": "rule"|"weighted", "weights": [...],
/// "fallback": "on_error"|"none", "provider_name", "model_id"}.
#[pyfunction]
#[pyo3(signature = (models, config_json=None))]
fn router(models: Vec<PyRef<Model>>, config_json: Option<&str>) -> PyResult<Model> {
    if models.is_empty() {
        return Err(to_py_err(&AiMuxError::InvalidArgument(
            "router: models must be non-empty".into(),
        )));
    }
    let children: Vec<Arc<dyn LanguageModel>> = models.iter().map(|m| m.inner.clone()).collect();
    let cfg: RouterFfiConfig = match config_json {
        Some(json) => wire_json("config_json", json)?,
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
}

/// Create a MoaModel (RFC-0022) over reference models + one aggregator.
/// References fan out in parallel, then the aggregator synthesizes a final
/// answer. `references` may be empty (runs aggregator only).
///
/// `config_json` (optional) is a serialized MoaConfig.
#[pyfunction]
#[pyo3(signature = (references, aggregator, config_json=None))]
fn moa(
    references: Vec<PyRef<Model>>,
    aggregator: PyRef<Model>,
    config_json: Option<&str>,
) -> PyResult<Model> {
    let refs: Vec<Arc<dyn LanguageModel>> = references.iter().map(|m| m.inner.clone()).collect();
    let cfg: aimux_core::moa::MoaConfig = match config_json {
        Some(json) => wire_json("config_json", json)?,
        None => aimux_core::moa::MoaConfig::default(),
    };
    let model = aimux_core::moa::MoaModel::new(refs, aggregator.inner.clone(), cfg);
    Ok(Model {
        inner: Arc::new(model),
        trace_store: None,
    })
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

#[pymodule]
fn aimux(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    crate::error::register(m)?;
    m.add_class::<Model>()?;
    m.add_class::<StreamIterator>()?;
    m.add_function(wrap_pyfunction!(init_logging, m)?)?;
    m.add_function(wrap_pyfunction!(init_session_store, m)?)?;
    m.add_function(wrap_pyfunction!(init_session_infer, m)?)?;
    m.add_function(wrap_pyfunction!(session_calls, m)?)?;
    m.add_function(wrap_pyfunction!(list_sessions, m)?)?;
    m.add_function(wrap_pyfunction!(init_recording, m)?)?;
    m.add_function(wrap_pyfunction!(init_recording_ring, m)?)?;
    m.add_function(wrap_pyfunction!(recording_stop, m)?)?;
    m.add_function(wrap_pyfunction!(recording_flush, m)?)?;
    m.add_function(wrap_pyfunction!(recording_try_flush, m)?)?;
    m.add_function(wrap_pyfunction!(mock_replay, m)?)?;
    m.add_function(wrap_pyfunction!(router, m)?)?;
    {
        use crate::multimodal::start_transcription_session;
        m.add_function(wrap_pyfunction!(start_transcription_session, m)?)?;
    }
    m.add_function(wrap_pyfunction!(moa, m)?)?;
    m.add_function(wrap_pyfunction!(register_providers, m)?)?;
    m.add_function(wrap_pyfunction!(init_proxy, m)?)?;
    m.add_function(wrap_pyfunction!(openai, m)?)?;
    m.add_function(wrap_pyfunction!(anthropic, m)?)?;
    m.add_function(wrap_pyfunction!(deepseek, m)?)?;
    m.add_function(wrap_pyfunction!(google, m)?)?;
    m.add_function(wrap_pyfunction!(cohere, m)?)?;
    m.add_function(wrap_pyfunction!(mistral, m)?)?;
    m.add_function(wrap_pyfunction!(xai, m)?)?;
    m.add_function(wrap_pyfunction!(bedrock, m)?)?;
    m.add_function(wrap_pyfunction!(vertex, m)?)?;
    m.add_function(wrap_pyfunction!(anthropic_aws, m)?)?;
    m.add_function(wrap_pyfunction!(azure, m)?)?;
    m.add_function(wrap_pyfunction!(provider, m)?)?;
    m.add_function(wrap_pyfunction!(create_provider, m)?)?;
    m.add_function(wrap_pyfunction!(get_model_specs, m)?)?;
    m.add_class::<ProviderHandle>()?;

    // Multimodal classes.
    m.add_class::<EmbeddingModel>()?;
    m.add_class::<SpeechModel>()?;
    m.add_class::<ImageModel>()?;
    m.add_class::<TranscriptionModel>()?;
    m.add_class::<crate::multimodal::TranscriptionSession>()?;
    m.add_class::<RerankingModel>()?;
    m.add_class::<VideoModel>()?;
    m.add_class::<SearchModel>()?;
    m.add_class::<Files>()?;

    // Multimodal factory functions.
    m.add_function(wrap_pyfunction!(openai_embedding, m)?)?;
    m.add_function(wrap_pyfunction!(openai_speech, m)?)?;
    m.add_function(wrap_pyfunction!(openai_image, m)?)?;
    m.add_function(wrap_pyfunction!(openai_transcription, m)?)?;
    m.add_function(wrap_pyfunction!(openai_files, m)?)?;
    m.add_function(wrap_pyfunction!(cohere_embedding, m)?)?;
    m.add_function(wrap_pyfunction!(cohere_reranking, m)?)?;
    m.add_function(wrap_pyfunction!(google_embedding, m)?)?;
    m.add_function(wrap_pyfunction!(google_image, m)?)?;
    m.add_function(wrap_pyfunction!(google_video, m)?)?;
    m.add_function(wrap_pyfunction!(tavily_search, m)?)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn rt_spawn<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    runtime().spawn(future);
}

fn parse_prompt(json: &str) -> PyResult<ModelPrompt> {
    // Malformed text is the binding's failure; a well-formed prompt the core
    // would reject is core-owned.
    let value: serde_json::Value = wire_json("prompt_json", json)?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
        .map_err(|e| to_py_err(&AiMuxError::InvalidArgument(format!("invalid prompt: {e}"))))
}

fn parse_opts(json: Option<&str>) -> PyResult<GenerateTextOptions> {
    match json {
        None => Ok(GenerateTextOptions::default()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return Ok(GenerateTextOptions::default());
            }
            wire_json("opts_json", s)
        }
    }
}
