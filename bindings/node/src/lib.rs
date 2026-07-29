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

#[napi]
impl Model {
    /// Generate text (non-streaming).
    ///
    /// `prompt` — a JSON string: bare prompt (`"text"` or `[{...}]`) or `{"prompt": ...}`.
    /// `options` — optional JSON-serialized `GenerateTextOptions`.
    /// Returns a JSON-serialized `GenerateTextResult`.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate_text(
        &self,
        prompt: String,
        options: Option<String>,
    ) -> Result<String> {
        let parsed_prompt = parse_prompt(&prompt)?;
        let opts = parse_opts(options.as_deref())?;

        let result = generate_text(&*self.inner, parsed_prompt, opts)
            .await
            .map_err(|e| Error::from_reason(format!("{e}")))?;

        serde_json::to_string(&result)
            .map_err(|e| Error::from_reason(format!("serialize result: {e}")))
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
    ) -> Result<StreamTextGenerator> {
        let model = self.inner.clone();

        let (tx, rx) = tokio::sync::mpsc::channel::<std::result::Result<String, String>>(64);

        // Spawn the stream-driving task immediately on napi's tokio runtime.
        napi::tokio::spawn(async move {
            let prompt = match parse_prompt(&prompt) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Err(format!("invalid prompt: {e}"))).await;
                    return;
                }
            };
            let opts = match parse_opts(options.as_deref()) {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(Err(format!("invalid options: {e}"))).await;
                    return;
                }
            };

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
                                let _ = tx.send(Err(format!("{e}"))).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("{e}"))).await;
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

/// Create an OpenAI model instance.
#[napi]
pub async fn openai(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};

    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("{e}")))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create an Anthropic model instance.
#[napi]
pub async fn anthropic(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};

    let mut config = AnthropicConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = AnthropicProvider::new(config);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("{e}")))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

/// Create a DeepSeek model instance.
#[napi]
pub async fn deepseek(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> Result<Model> {
    use aimux_core::provider::Provider;
    use aimux_providers::deepseek::{DeepSeekConfig, DeepSeekProvider};

    let mut config = DeepSeekConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = DeepSeekProvider::new(config);
    let model = provider
        .language_model(&model_id)
        .map_err(|e| Error::from_reason(format!("{e}")))?;
    Ok(Model {
        inner: Arc::from(model),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_prompt(json: &str) -> Result<ModelPrompt> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::from_reason(format!("invalid prompt JSON: {e}")))?;
    let inner = match &value {
        serde_json::Value::Object(obj) if obj.len() == 1 && obj.contains_key("prompt") => {
            obj.get("prompt").expect("checked by guard")
        }
        _ => &value,
    };
    serde_json::from_value(inner.clone())
        .map_err(|e| Error::from_reason(format!("invalid prompt: {e}")))
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
                .map_err(|e| Error::from_reason(format!("invalid options JSON: {e}")))
        }
    }
}
