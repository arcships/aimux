//! Multimodal API bindings for Python (PyO3).
//!
//! Each modality is a PyO3 class wrapping the Rust trait object.
//! All cross-boundary data uses JSON strings (base64 for binary).
//!
//! Mirrors the Node (napi-rs) multimodal bindings, but uses PyO3
//! `#[pyclass]` + `#[pymethods]` and the crate-global tokio runtime
//! (see `runtime()` in `lib.rs`).

// pyo3 0.22 macros generate unsafe-op-in-unsafe-fn calls that trigger
// edition-2024 lint. Suppress until pyo3 0.23+ lands.
#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;

use crate::error::to_py_err;
use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel as EmbeddingModelTrait};
use aimux_core::files_model::{Files as FilesTrait, UploadFileCallOptions};
use aimux_core::image_model::{ImageCallOptions, ImageModel as ImageModelTrait};
use aimux_core::reranking_model::{RerankingCallOptions, RerankingModel as RerankingModelTrait};
use aimux_core::search_model::{SearchCallOptions, SearchModel as SearchModelTrait};
use aimux_core::shared::FileBytes;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel as SpeechModelTrait};
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, TranscriptionCallOptions,
    TranscriptionModel as TranscriptionModelTrait,
};
use aimux_core::video_model::{VideoCallOptions, VideoModel as VideoModelTrait};
use pyo3::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct EmbeddingModel {
    inner: Arc<dyn EmbeddingModelTrait>,
}

#[pymethods]
impl EmbeddingModel {
    /// Generate embeddings. `values_json` is a JSON array of strings.
    /// `opts_json` is optional JSON-serialized EmbeddingCallOptions.
    /// Returns JSON-serialized EmbeddingResult.
    #[pyo3(signature = (values_json, opts_json=None))]
    pub fn embed(&self, values_json: &str, opts_json: Option<&str>) -> PyResult<String> {
        let mut opts: EmbeddingCallOptions = match opts_json {
            Some(s) if !s.trim().is_empty() && s.trim() != "null" => serde_json::from_str(s)
                .map_err(|e| {
                    to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?,
            _ => EmbeddingCallOptions::new(""),
        };
        // Override values from the JSON array.
        let values: Vec<String> = serde_json::from_str(values_json).map_err(|e| {
            to_py_err(&AiMuxError::InvalidArgument(format!(
                "invalid values_json: {e}"
            )))
        })?;
        opts.values = values;

        let result = crate::runtime().block_on(async move { self.inner.do_embed(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpeechModel (TTS)
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct SpeechModel {
    inner: Arc<dyn SpeechModelTrait>,
}

#[pymethods]
impl SpeechModel {
    /// Generate speech audio. `opts_json` is JSON-serialized SpeechCallOptions.
    /// Returns JSON-serialized SpeechResult (audio as base64 in JSON).
    pub fn generate(&self, opts_json: &str) -> PyResult<String> {
        let opts: SpeechCallOptions = serde_json::from_str(opts_json)
            .map_err(|e| to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}"))))?;

        let result = crate::runtime().block_on(async move { self.inner.do_generate(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageModel
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct ImageModel {
    inner: Arc<dyn ImageModelTrait>,
}

#[pymethods]
impl ImageModel {
    /// Generate images. `opts_json` is JSON-serialized ImageCallOptions.
    /// Returns JSON-serialized ImageResult (images as base64 in JSON).
    pub fn generate(&self, opts_json: &str) -> PyResult<String> {
        let opts: ImageCallOptions = serde_json::from_str(opts_json)
            .map_err(|e| to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}"))))?;

        let result = crate::runtime().block_on(async move { self.inner.do_generate(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionModel (STT, non-streaming only)
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct TranscriptionModel {
    inner: Arc<dyn TranscriptionModelTrait>,
}

#[pymethods]
impl TranscriptionModel {
    /// Transcribe audio. `audio_base64` is base64-encoded audio data.
    /// `media_type` is e.g. "audio/mp3". `opts_json` is optional JSON options.
    /// Returns JSON-serialized TranscriptionResult.
    #[pyo3(signature = (audio_base64, media_type, opts_json=None))]
    pub fn generate(
        &self,
        audio_base64: &str,
        media_type: &str,
        opts_json: Option<&str>,
    ) -> PyResult<String> {
        let mut opts = TranscriptionCallOptions::new(
            AudioInput::Base64(audio_base64.to_string()),
            media_type.to_string(),
        );
        if let Some(s) = opts_json {
            if !s.trim().is_empty() && s.trim() != "null" {
                let parsed: TranscriptionCallOptions = serde_json::from_str(s).map_err(|e| {
                    to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                // Keep audio and media_type from our explicit args.
                if let Some(p) = &parsed.provider_options {
                    opts.provider_options = Some(p.clone());
                }
            }
        }

        let result = crate::runtime().block_on(async move { self.inner.do_generate(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RerankingModel
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct RerankingModel {
    inner: Arc<dyn RerankingModelTrait>,
}

#[pymethods]
impl RerankingModel {
    /// Rerank documents. `query` is the search query, `docs_json` is a JSON
    /// array of documents, `opts_json` is optional JSON options.
    /// Returns JSON-serialized RerankingResult.
    #[pyo3(signature = (query, docs_json, opts_json=None))]
    pub fn rerank(
        &self,
        query: &str,
        docs_json: &str,
        opts_json: Option<&str>,
    ) -> PyResult<String> {
        use aimux_core::reranking_model::RerankingDocuments;
        let docs: RerankingDocuments = serde_json::from_str(docs_json).map_err(|e| {
            to_py_err(&AiMuxError::InvalidArgument(format!(
                "invalid docs_json: {e}"
            )))
        })?;

        let mut opts = RerankingCallOptions::new(query.to_string(), docs);
        if let Some(s) = opts_json {
            if !s.trim().is_empty() && s.trim() != "null" {
                let parsed: RerankingCallOptions = serde_json::from_str(s).map_err(|e| {
                    to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                opts.provider_options = parsed.provider_options;
                opts.top_n = parsed.top_n;
            }
        }

        let result = crate::runtime().block_on(async move { self.inner.do_rerank(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VideoModel
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct VideoModel {
    inner: Arc<dyn VideoModelTrait>,
}

#[pymethods]
impl VideoModel {
    /// Generate video. `opts_json` is JSON-serialized VideoCallOptions.
    /// Returns JSON-serialized VideoResult (typically contains a URL).
    pub fn generate(&self, opts_json: &str) -> PyResult<String> {
        let opts: VideoCallOptions = serde_json::from_str(opts_json)
            .map_err(|e| to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}"))))?;

        let result = crate::runtime().block_on(async move { self.inner.do_generate(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchModel
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct SearchModel {
    inner: Arc<dyn SearchModelTrait>,
}

#[pymethods]
impl SearchModel {
    /// Search. `query` is the search query, `opts_json` is optional JSON options.
    /// Returns JSON-serialized SearchResult.
    #[pyo3(signature = (query, opts_json=None))]
    pub fn search(&self, query: &str, opts_json: Option<&str>) -> PyResult<String> {
        let mut opts = SearchCallOptions::new(query.to_string());
        if let Some(s) = opts_json {
            if !s.trim().is_empty() && s.trim() != "null" {
                let parsed: SearchCallOptions = serde_json::from_str(s).map_err(|e| {
                    to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                opts = parsed;
            }
        }

        let result = crate::runtime().block_on(async move { self.inner.do_search(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct Files {
    inner: Arc<dyn FilesTrait>,
}

#[pymethods]
impl Files {
    /// Upload a file. `data_base64` is base64-encoded file content,
    /// `media_type` is e.g. "application/pdf", `opts_json` is optional
    /// (may contain filename, provider_options).
    /// Returns JSON-serialized UploadFileResult (contains provider file ID).
    #[pyo3(signature = (data_base64, media_type, opts_json=None))]
    pub fn upload_file(
        &self,
        data_base64: &str,
        media_type: &str,
        opts_json: Option<&str>,
    ) -> PyResult<String> {
        use aimux_core::files_model::UploadFileData;
        let mut opts = UploadFileCallOptions::new(
            UploadFileData::Data {
                data: FileBytes::Base64(data_base64.to_string()),
            },
            media_type.to_string(),
        );
        if let Some(s) = opts_json {
            if !s.trim().is_empty() && s.trim() != "null" {
                let parsed: UploadFileCallOptions = serde_json::from_str(s).map_err(|e| {
                    to_py_err(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                opts.filename = parsed.filename;
                opts.provider_options = parsed.provider_options;
            }
        }

        let result = crate::runtime().block_on(async move { self.inner.upload_file(&opts).await });

        match result {
            Ok(r) => serde_json::to_string(&r)
                .map_err(|e| to_py_err(&AiMuxError::JsonParse(format!("serialize result: {e}")))),
            Err(e) => Err(to_py_err(&e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory functions — OpenAI multimodal
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI embedding model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn openai_embedding(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<EmbeddingModel> {
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider.embedding_model(model_id);
    Ok(EmbeddingModel {
        inner: Arc::new(model),
    })
}

/// Create an OpenAI speech (TTS) model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn openai_speech(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<SpeechModel> {
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider.speech(model_id);
    Ok(SpeechModel {
        inner: Arc::new(model),
    })
}

/// Create an OpenAI image model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn openai_image(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<ImageModel> {
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider.image(model_id);
    Ok(ImageModel {
        inner: Arc::new(model),
    })
}

/// Create an OpenAI transcription model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn openai_transcription(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<TranscriptionModel> {
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription(model_id);
    Ok(TranscriptionModel {
        inner: Arc::new(model),
    })
}

/// Create OpenAI files manager.
#[pyfunction]
#[pyo3(signature = (api_key, base_url=None))]
pub fn openai_files(api_key: &str, base_url: Option<&str>) -> PyResult<Files> {
    use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
    let mut config = OpenAIConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = OpenAIProvider::new(config);
    let files = provider.files();
    Ok(Files {
        inner: Arc::new(files),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory functions — Cohere multimodal
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Cohere embedding model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn cohere_embedding(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<EmbeddingModel> {
    use aimux_providers::cohere::{CohereConfig, CohereProvider};
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = CohereProvider::new(config);
    let model = provider.embedding_model(model_id);
    Ok(EmbeddingModel {
        inner: Arc::new(model),
    })
}

/// Create a Cohere reranking model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn cohere_reranking(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<RerankingModel> {
    use aimux_providers::cohere::{CohereConfig, CohereProvider};
    let mut config = CohereConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = CohereProvider::new(config);
    let model = provider.reranking_model(model_id);
    Ok(RerankingModel {
        inner: Arc::new(model),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory functions — Google multimodal
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Google embedding model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn google_embedding(
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> PyResult<EmbeddingModel> {
    use aimux_providers::google::{GoogleConfig, GoogleProvider};
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = GoogleProvider::new(config);
    let model = provider.embedding_model(model_id);
    Ok(EmbeddingModel {
        inner: Arc::new(model),
    })
}

/// Create a Google image model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn google_image(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<ImageModel> {
    use aimux_providers::google::{GoogleConfig, GoogleProvider};
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = GoogleProvider::new(config);
    let model = provider.image(model_id);
    Ok(ImageModel {
        inner: Arc::new(model),
    })
}

/// Create a Google video model instance.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, base_url=None))]
pub fn google_video(api_key: &str, model_id: &str, base_url: Option<&str>) -> PyResult<VideoModel> {
    use aimux_providers::google::{GoogleConfig, GoogleProvider};
    let mut config = GoogleConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = GoogleProvider::new(config);
    let model = provider.video(model_id);
    Ok(VideoModel {
        inner: Arc::new(model),
    })
}

/// Create a Tavily search model instance.
#[pyfunction]
#[pyo3(signature = (api_key, base_url=None))]
pub fn tavily_search(api_key: &str, base_url: Option<&str>) -> PyResult<SearchModel> {
    use aimux_providers::tavily::{TavilyConfig, TavilyProvider};
    let mut config = TavilyConfig::new(api_key);
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }
    let provider = TavilyProvider::new(config);
    let model = provider.search_model();
    Ok(SearchModel {
        inner: Arc::new(model),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionSession (RFC-0028 streaming, native path)
// ─────────────────────────────────────────────────────────────────────────────

/// A live streaming-transcription session (RFC-0028). Push audio chunks with
/// `push_audio`, mark end-of-audio with `input_done`, then pull transcription
/// parts with `next_part`. Mirrors the C-ABI session shape, built natively on
/// core channels.
#[pyclass]
pub struct TranscriptionSession {
    audio_tx: std::sync::Mutex<Option<futures::channel::mpsc::Sender<AudioChunk>>>,
    parts_rx: tokio::sync::Mutex<
        tokio::sync::mpsc::Receiver<std::result::Result<String, AiMuxError>>,
    >,
    token: aimux_core::shared::AbortSignal,
}

/// Start a streaming transcription session. `opts_json` (optional):
/// `{ "input_audio_format": {"format_type","rate"}, "provider_options",
/// "headers", "include_raw_chunks" }`. Returned parts are JSON-serialized
/// `TranscriptionStreamPart`s.
#[pyfunction]
#[pyo3(signature = (model, opts_json=None))]
pub fn start_transcription_session(
    model: &TranscriptionModel,
    opts_json: Option<&str>,
) -> PyResult<TranscriptionSession> {
    #[derive(serde::Deserialize, Default)]
    struct SessionOpts {
        input_audio_format: Option<aimux_core::transcription_model::InputAudioFormat>,
        provider_options: Option<std::collections::HashMap<String, serde_json::Value>>,
        headers: Option<std::collections::HashMap<String, String>>,
        include_raw_chunks: Option<bool>,
        timeout: Option<aimux_core::options::TimeoutConfiguration>,
    }
    let opts: SessionOpts = match opts_json {
        Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
            serde_json::from_str(s).map_err(|e| {
                to_py_err(&AiMuxError::InvalidArgument(format!(
                    "invalid opts_json: {e}"
                )))
            })?
        }
        _ => SessionOpts::default(),
    };

    let token = aimux_core::shared::AbortSignal::new();
    let effective = aimux_core::shared::AbortSignal::new();
    {
        let linked = effective.clone();
        let source = token.clone();
        crate::runtime().spawn(async move {
            source.cancelled().await;
            linked.abort();
        });
    }

    let (audio_tx, audio_rx) = futures::channel::mpsc::channel::<AudioChunk>(64);
    let (tx, rx) =
        tokio::sync::mpsc::channel::<std::result::Result<String, AiMuxError>>(256);

    let model = model.inner.clone();
    crate::runtime().spawn(async move {
        let options = aimux_core::transcription_model::TranscriptionStreamOptions {
            audio: Box::pin(audio_rx),
            input_audio_format: opts
                .input_audio_format
                .unwrap_or(aimux_core::transcription_model::InputAudioFormat {
                    format_type: "audio/pcm".to_string(),
                    rate: None,
                }),
            provider_options: opts.provider_options,
            abort_signal: Some(effective.clone()),
            headers: opts.headers,
            include_raw_chunks: opts.include_raw_chunks.unwrap_or(false),
            timeout: opts.timeout,
        };
        let result = model.do_stream(options).await;
        match result {
            Ok(stream_result) => {
                use futures::StreamExt;
                let mut stream = stream_result.stream;
                // Immediate delivery when capacity allows (terminal errors are
                // never preempted); abort only unblocks a full channel.
                while let Some(item) = stream.next().await {
                    // In-stream errors raise from `next_part` (issue #145,
                    // option A) instead of being smuggled through the data
                    // channel as a serialized `{"Err": ...}` part — matching
                    // the FFI session, which surfaces them as Err items, and
                    // the other six bindings' exception sentinels.
                    let part = match item {
                        Ok(p) => p,
                        Err(e) => {
                            // Terminal by contract: providers end the stream
                            // after an Error part (openai transcription
                            // breaks right after), so nothing is dropped by
                            // returning here. New providers must uphold this.
                            tokio::select! {
                                res = tx.send(Err(e)) => { let _ = res; }
                                _ = effective.cancelled() => {}
                            }
                            return;
                        }
                    };
                    let json = match serde_json::to_string(&part) {
                        Ok(j) => j,
                        Err(e) => {
                            let _ = tx
                                .send(Err(AiMuxError::JsonParse(format!(
                                    "serialize part: {e}"
                                ))))
                                .await;
                            return;
                        }
                    };
                    loop {
                        match tx.try_send(Ok(json.clone())) {
                            Ok(()) => break,
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                tokio::select! {
                                    _ = effective.cancelled() => return,
                                    res = tx.send(Ok(json.clone())) => {
                                        if res.is_err() { return; }
                                        break;
                                    }
                                }
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                return;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // Connect failure: deliver as the first channel item
                // (try_send + abort-select; mirrors the FFI driver).
                loop {
                    match tx.try_send(Err(e.clone())) {
                        Ok(()) => break,
                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                            tokio::select! {
                                _ = effective.cancelled() => return,
                                res = tx.send(Err(e.clone())) => {
                                    if res.is_err() { return; }
                                    break;
                                }
                            }
                        }
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return,
                    }
                }
            }
        }
    });

    Ok(TranscriptionSession {
        audio_tx: std::sync::Mutex::new(Some(audio_tx)),
        parts_rx: tokio::sync::Mutex::new(rx),
        token,
    })
}

#[pymethods]
impl TranscriptionSession {
    /// Push one binary audio chunk. Blocks while the internal channel is
    /// full (backpressure).
    #[pyo3(signature = (data,))]
    pub fn push_audio(&self, py: pyo3::Python<'_>, data: &[u8]) -> PyResult<()> {
        use futures::SinkExt;
        // Clone the sender and drop the guard BEFORE blocking: the mutex is
        // shared with the sync input_done()/close(), and blocking with the
        // guard held (or with the GIL held) would freeze other Python
        // threads — including one calling close() to abort.
        let mut tx = {
            let guard = self
                .audio_tx
                .lock()
                .map_err(|_| to_py_err(&AiMuxError::Other("session poisoned".into())))?;
            match guard.as_ref() {
                None => {
                    return Err(to_py_err(&AiMuxError::Other(
                        "audio input already finished".into(),
                    )));
                }
                Some(tx) => tx.clone(),
            }
        };
        let chunk = AudioChunk::Binary(data.to_vec());
        py.allow_threads(|| {
            crate::runtime().block_on(async move { tx.send(chunk).await })
        })
        .map_err(|_| to_py_err(&AiMuxError::Other("session ended".into())))
    }

    /// Signal end-of-audio (idempotent).
    pub fn input_done(&self) -> PyResult<()> {
        self.audio_tx
            .lock()
            .map_err(|_| to_py_err(&AiMuxError::Other("session poisoned".into())))?
            .take();
        Ok(())
    }

    /// Pull the next transcription part (JSON string). Returns None when the
    /// stream ended normally. Raises on error — including APITimeoutError
    /// (an AimuxError subclass) when no part arrives within `timeout_ms`
    /// (the session stays live, call again). `timeout_ms`: >0 wait at most;
    /// 0 immediate poll; negative = wait indefinitely.
    #[pyo3(signature = (timeout_ms=-1))]
    pub fn next_part(
        &self,
        py: pyo3::Python<'_>,
        timeout_ms: i64,
    ) -> PyResult<Option<String>> {
        let timeout = if timeout_ms >= 0 {
            Some(std::time::Duration::from_millis(timeout_ms as u64))
        } else {
            None
        };
        enum Outcome {
            Part(Option<std::result::Result<String, AiMuxError>>),
            TimedOut,
        }
        let outcome = py.allow_threads(|| {
            crate::runtime().block_on(async {
                let mut rx = self.parts_rx.lock().await;
                let recv = rx.recv();
                match timeout {
                    Some(d) => match tokio::time::timeout(d, recv).await {
                        Ok(p) => Outcome::Part(p),
                        Err(_) => Outcome::TimedOut,
                    },
                    None => Outcome::Part(recv.await),
                }
            })
        });
        let part = match outcome {
            Outcome::Part(p) => p,
            Outcome::TimedOut => {
                return Err(to_py_err(&AiMuxError::Timeout(
                    "no transcription part within timeout".into(),
                )));
            }
        };
        match part {
            Some(Ok(json)) => Ok(Some(json)),
            Some(Err(e)) => Err(to_py_err(&e)),
            None => Ok(None), // ended normally
        }
    }

    /// Terminate the session (aborts the driver). Further push/next fail.
    pub fn close(&self) {
        if let Ok(mut guard) = self.audio_tx.lock() {
            guard.take();
        }
        self.token.abort();
    }
}
