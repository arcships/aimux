//! aimux-python: Python binding (PyO3, native path).
//!
//! Directly uses aimux-providers, bypassing aimux-ffi.
//! PyO3 maps Rust async to Python via a tokio runtime + async generator.

// pyo3 0.22 macros generate unsafe-op-in-unsafe-fn calls that trigger
// edition-2024 lint. Suppress until pyo3 0.23+ lands.
#![allow(unsafe_op_in_unsafe_fn)]

mod multimodal;
pub use multimodal::*;

use std::sync::Arc;

use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text, stream_text_as_openai,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
use aimux_core::openai_output::OpenAiStreamOptions;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Global tokio runtime
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("failed to build tokio runtime")
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Model — a provider model instance accessible from Python.
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
struct Model {
    inner: Arc<dyn LanguageModel>,
}

#[pymethods]
impl Model {
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
        let result = rt.block_on(async move {
            generate_text(&*self.inner, prompt, opts).await
        });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| PyRuntimeError::new_err(format!("[Json] serialize result: {e}"))),
            Err(e) => Err(PyRuntimeError::new_err(format!("[{}] {e}", e.error_type()))),
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

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(64);

        rt_spawn(async move {
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
                                    break;
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
        let result = rt.block_on(async move {
            generate_text_as_openai(&*self.inner, prompt, opts).await
        });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| PyRuntimeError::new_err(format!("[Json] serialize result: {e}"))),
            Err(e) => Err(PyRuntimeError::new_err(format!("[{}] {e}", e.error_type()))),
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

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, String>>(64);

        rt_spawn(async move {
            match stream_text_as_openai(&*model, prompt, opts, stream_options).await {
                Ok(stream_result) => {
                    use futures::StreamExt;
                    let mut stream = stream_result.stream;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(chunk) => {
                                let json = serde_json::to_string(&chunk)
                                    .unwrap_or_else(|_| "{}".to_string());
                                if tx.send(Ok(json)).await.is_err() {
                                    break;
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

        Ok(StreamIterator { rx })
    }
}

/// Python iterator that yields StreamPart JSON strings from a tokio channel.
#[pyclass]
struct StreamIterator {
    rx: tokio::sync::mpsc::Receiver<Result<String, String>>,
}

#[pymethods]
impl StreamIterator {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        // Block on the next channel item, allowing other Python threads to run.
        let item = py.allow_threads(|| {
            runtime().block_on(self.rx.recv())
        });

        match item {
            Some(Ok(json)) => Ok(Some(json.to_object(py).into())),
            Some(Err(e)) => Err(PyRuntimeError::new_err(e)),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
    let provider = AzureProvider::new(config)
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    let model = provider
        .language_model(deployment)
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
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
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            Some(serde_json::from_str(s).map_err(|e| {
                PyRuntimeError::new_err(format!("[Json] invalid config: {e}"))
            })?)
        }
        _ => None,
    };
    if let Some(url) = base_url {
        options.get_or_insert_with(Default::default).base_url = Some(url.to_string());
    }
    let model = aimux_providers::provider(name, api_key, model_id, options)
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
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
    let level = if level.trim().is_empty() { "warn" } else { level };
    aimux_providers::init_logging(level);
}

#[pymodule]
fn aimux(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Model>()?;
    m.add_class::<StreamIterator>()?;
    m.add_function(wrap_pyfunction!(init_logging, m)?)?;
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

    // Multimodal classes.
    m.add_class::<EmbeddingModel>()?;
    m.add_class::<SpeechModel>()?;
    m.add_class::<ImageModel>()?;
    m.add_class::<TranscriptionModel>()?;
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
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| PyRuntimeError::new_err(format!("[Json] invalid prompt JSON: {e}")))?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
        .map_err(|e| PyRuntimeError::new_err(format!("[Json] invalid prompt: {e}")))
}

fn parse_opts(json: Option<&str>) -> PyResult<GenerateTextOptions> {
    match json {
        None => Ok(GenerateTextOptions::default()),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return Ok(GenerateTextOptions::default());
            }
            serde_json::from_str(s)
                .map_err(|e| PyRuntimeError::new_err(format!("[Json] invalid options JSON: {e}")))
        }
    }
}
