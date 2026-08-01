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

use aimux_core::generate::{generate_text, stream_text, GenerateTextOptions};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::ModelPrompt;
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

/// Create a DeepSeek model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
fn deepseek(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::deepseek::{DeepSeekConfig, DeepSeekProvider};

    let mut config = DeepSeekConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = DeepSeekProvider::new(config);
    let model = provider
        .language_model(model_id)
        .map_err(|e| PyRuntimeError::new_err(format!("[{}] {e}", e.error_type())))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Module
// ─────────────────────────────────────────────────────────────────────────────

#[pymodule]
fn aimux(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Model>()?;
    m.add_class::<StreamIterator>()?;
    m.add_function(wrap_pyfunction!(openai, m)?)?;
    m.add_function(wrap_pyfunction!(anthropic, m)?)?;
    m.add_function(wrap_pyfunction!(deepseek, m)?)?;

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
