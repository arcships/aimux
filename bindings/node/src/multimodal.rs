//! Multimodal API bindings for Node.js (napi-rs).
//!
//! Each modality is a napi class wrapping the Rust trait object.
//! All cross-boundary data uses JSON strings (base64 for binary).

use crate::error::{AimuxResult, MappedError};
use std::sync::Arc;

use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel as EmbeddingModelTrait};
use aimux_core::files_model::{Files as FilesTrait, UploadFileCallOptions};
use aimux_core::image_model::{ImageCallOptions, ImageModel as ImageModelTrait};
use aimux_core::reranking_model::{RerankingCallOptions, RerankingModel as RerankingModelTrait};
use aimux_core::search_model::SearchModel as SearchModelTrait;
use aimux_core::shared::FileBytes;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel as SpeechModelTrait};
use aimux_core::transcription_model::{
    AudioChunk, AudioInput, TranscriptionCallOptions,
    TranscriptionModel as TranscriptionModelTrait,
};
use aimux_core::video_model::{VideoCallOptions, VideoModel as VideoModelTrait};
use napi_derive::napi;

// ─────────────────────────────────────────────────────────────────────────────
// EmbeddingModel
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct EmbeddingModel {
    inner: Arc<dyn EmbeddingModelTrait>,
}

#[napi]
impl EmbeddingModel {
    /// Generate embeddings. `values_json` is a JSON array of strings.
    /// Returns JSON-serialized EmbeddingResult.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn embed(
        &self,
        values_json: String,
        opts_json: Option<String>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let mut opts: EmbeddingCallOptions = match opts_json.as_deref() {
                    Some(s) if !s.trim().is_empty() && s.trim() != "null" => {
                        serde_json::from_str(s).map_err(|e| {
                            MappedError::from(&AiMuxError::InvalidArgument(format!(
                                "invalid opts: {e}"
                            )))
                        })?
                    }
                    _ => EmbeddingCallOptions::new(""),
                };
                // Override values from the JSON array
                let values: Vec<String> = serde_json::from_str(&values_json).map_err(|e| {
                    MappedError::from(&AiMuxError::InvalidArgument(format!(
                        "invalid values_json: {e}"
                    )))
                })?;
                opts.values = values;

                let result = self
                    .inner
                    .do_embed(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpeechModel (TTS)
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct SpeechModel {
    inner: Arc<dyn SpeechModelTrait>,
}

#[napi]
impl SpeechModel {
    /// Generate speech audio. `opts_json` is JSON-serialized SpeechCallOptions.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized SpeechResult (audio as base64 in JSON).
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate(
        &self,
        opts_json: String,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let mut opts: SpeechCallOptions =
                    serde_json::from_str(&opts_json).map_err(|e| {
                        MappedError::from(&AiMuxError::InvalidArgument(format!(
                            "invalid opts: {e}"
                        )))
                    })?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_generate(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ImageModel
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct ImageModel {
    inner: Arc<dyn ImageModelTrait>,
}

#[napi]
impl ImageModel {
    /// Generate images. `opts_json` is JSON-serialized ImageCallOptions.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized ImageResult (images as base64 in JSON).
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate(
        &self,
        opts_json: String,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let mut opts: ImageCallOptions = serde_json::from_str(&opts_json).map_err(|e| {
                    MappedError::from(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_generate(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionModel (STT, non-streaming only)
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct TranscriptionModel {
    inner: Arc<dyn TranscriptionModelTrait>,
}

#[napi]
impl TranscriptionModel {
    /// Transcribe audio. `audio_base64` is base64-encoded audio data.
    /// `media_type` is e.g. "audio/mp3". `opts_json` is optional JSON options.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized TranscriptionResult.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate(
        &self,
        audio_base64: String,
        media_type: String,
        opts_json: Option<String>,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let mut opts =
                    TranscriptionCallOptions::new(AudioInput::Base64(audio_base64), media_type);
                if let Some(s) = opts_json.as_deref() {
                    if !s.trim().is_empty() && s.trim() != "null" {
                        let parsed: TranscriptionCallOptions =
                            serde_json::from_str(s).map_err(|e| {
                                MappedError::from(&AiMuxError::InvalidArgument(format!(
                                    "invalid opts: {e}"
                                )))
                            })?;
                        // Keep audio and media_type from our explicit args
                        parsed
                            .provider_options
                            .inspect(|p| opts.provider_options = Some(p.clone()));
                    }
                }
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_generate(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RerankingModel
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct RerankingModel {
    inner: Arc<dyn RerankingModelTrait>,
}

#[napi]
impl RerankingModel {
    /// Rerank documents. `query` is the search query, `docs_json` is a JSON
    /// array of documents, `opts_json` is optional JSON options.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized RerankingResult.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn rerank(
        &self,
        query: String,
        docs_json: String,
        opts_json: Option<String>,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                use aimux_core::reranking_model::RerankingDocuments;
                let docs: RerankingDocuments = serde_json::from_str(&docs_json).map_err(|e| {
                    MappedError::from(&AiMuxError::InvalidArgument(format!(
                        "invalid docs_json: {e}"
                    )))
                })?;

                let mut opts = RerankingCallOptions::new(query, docs);
                if let Some(s) = opts_json.as_deref() {
                    if !s.trim().is_empty() && s.trim() != "null" {
                        let parsed: RerankingCallOptions =
                            serde_json::from_str(s).map_err(|e| {
                                MappedError::from(&AiMuxError::InvalidArgument(format!(
                                    "invalid opts: {e}"
                                )))
                            })?;
                        opts.provider_options = parsed.provider_options;
                        opts.top_n = parsed.top_n;
                    }
                }
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_rerank(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VideoModel
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct VideoModel {
    inner: Arc<dyn VideoModelTrait>,
}

#[napi]
impl VideoModel {
    /// Generate video. `opts_json` is JSON-serialized VideoCallOptions.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized VideoResult (typically contains a URL).
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn generate(
        &self,
        opts_json: String,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                let mut opts: VideoCallOptions = serde_json::from_str(&opts_json).map_err(|e| {
                    MappedError::from(&AiMuxError::InvalidArgument(format!("invalid opts: {e}")))
                })?;
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_generate(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SearchModel
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct SearchModel {
    inner: Arc<dyn SearchModelTrait>,
}

#[napi]
impl SearchModel {
    /// Search. `query` is the search query, `opts_json` is optional JSON options.
    /// `bridge` — optional `AbortBridge`; aborting the wrapped signal cancels
    /// the call. Returns JSON-serialized SearchResult.
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn search(
        &self,
        query: String,
        opts_json: Option<String>,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                use aimux_core::search_model::SearchCallOptions;
                let mut opts = SearchCallOptions::new(query);
                if let Some(s) = opts_json.as_deref() {
                    if !s.trim().is_empty() && s.trim() != "null" {
                        let parsed: SearchCallOptions = serde_json::from_str(s).map_err(|e| {
                            MappedError::from(&AiMuxError::InvalidArgument(format!(
                                "invalid opts: {e}"
                            )))
                        })?;
                        opts = parsed;
                    }
                }
                opts.abort_signal = bridge.map(|b| b.core_signal());

                let result = self
                    .inner
                    .do_search(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Files
// ─────────────────────────────────────────────────────────────────────────────

#[napi]
pub struct Files {
    inner: Arc<dyn FilesTrait>,
}

#[napi]
impl Files {
    /// Upload a file. `data_base64` is base64-encoded file content,
    /// `media_type` is e.g. "application/pdf", `opts_json` is optional
    /// (may contain filename, provider_options).
    /// Returns JSON-serialized UploadFileResult (contains provider file ID).
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn upload_file(
        &self,
        data_base64: String,
        media_type: String,
        opts_json: Option<String>,
    ) -> AimuxResult<String> {
        AimuxResult({
            let __r: crate::error::MResult<String> = async {
                use aimux_core::files_model::UploadFileData;
                let mut opts = UploadFileCallOptions::new(
                    UploadFileData::Data {
                        data: FileBytes::Base64(data_base64),
                    },
                    media_type,
                );
                if let Some(s) = opts_json.as_deref() {
                    if !s.trim().is_empty() && s.trim() != "null" {
                        let parsed: UploadFileCallOptions =
                            serde_json::from_str(s).map_err(|e| {
                                MappedError::from(&AiMuxError::InvalidArgument(format!(
                                    "invalid opts: {e}"
                                )))
                            })?;
                        opts.filename = parsed.filename;
                        opts.provider_options = parsed.provider_options;
                    }
                }

                let result = self
                    .inner
                    .upload_file(&opts)
                    .await
                    .map_err(|e| MappedError::from(&e))?;
                serde_json::to_string(&result)
                    .map_err(|e| MappedError::from(&AiMuxError::JsonParse(format!("serialize: {e}"))))
            }
            .await;
            __r
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory functions — OpenAI multimodal
// ─────────────────────────────────────────────────────────────────────────────

/// Create an OpenAI embedding model instance.
#[napi]
pub async fn openai_embedding(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<EmbeddingModel> {
    AimuxResult({
        let __r: crate::error::MResult<EmbeddingModel> = async {
            use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
            let mut config = OpenAIConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = OpenAIProvider::new(config);
            let model = provider.embedding_model(&model_id);
            Ok(EmbeddingModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create an OpenAI speech (TTS) model instance.
#[napi]
pub async fn openai_speech(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<SpeechModel> {
    AimuxResult({
        let __r: crate::error::MResult<SpeechModel> = async {
            use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
            let mut config = OpenAIConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = OpenAIProvider::new(config);
            let model = provider.speech(&model_id);
            Ok(SpeechModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create an OpenAI image model instance.
#[napi]
pub async fn openai_image(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<ImageModel> {
    AimuxResult({
        let __r: crate::error::MResult<ImageModel> = async {
            use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
            let mut config = OpenAIConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = OpenAIProvider::new(config);
            let model = provider.image(&model_id);
            Ok(ImageModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create an OpenAI transcription model instance.
#[napi]
pub async fn openai_transcription(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<TranscriptionModel> {
    AimuxResult({
        let __r: crate::error::MResult<TranscriptionModel> = async {
            use aimux_providers::openai::{OpenAIConfig, OpenAIProvider};
            let mut config = OpenAIConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = OpenAIProvider::new(config);
            let model = provider.transcription(&model_id);
            Ok(TranscriptionModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create OpenAI files manager.
#[napi]
pub async fn openai_files(api_key: String, base_url: Option<String>) -> AimuxResult<Files> {
    AimuxResult({
        let __r: crate::error::MResult<Files> = async {
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
        .await;
        __r
    })
}

/// Create a Cohere embedding model instance.
#[napi]
pub async fn cohere_embedding(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<EmbeddingModel> {
    AimuxResult({
        let __r: crate::error::MResult<EmbeddingModel> = async {
            use aimux_providers::cohere::{CohereConfig, CohereProvider};
            let mut config = CohereConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = CohereProvider::new(config);
            let model = provider.embedding_model(&model_id);
            Ok(EmbeddingModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create a Cohere reranking model instance.
#[napi]
pub async fn cohere_reranking(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<RerankingModel> {
    AimuxResult({
        let __r: crate::error::MResult<RerankingModel> = async {
            use aimux_providers::cohere::{CohereConfig, CohereProvider};
            let mut config = CohereConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = CohereProvider::new(config);
            let model = provider.reranking_model(&model_id);
            Ok(RerankingModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create a Google embedding model instance.
#[napi]
pub async fn google_embedding(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<EmbeddingModel> {
    AimuxResult({
        let __r: crate::error::MResult<EmbeddingModel> = async {
            use aimux_providers::google::{GoogleConfig, GoogleProvider};
            let mut config = GoogleConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = GoogleProvider::new(config);
            let model = provider.embedding_model(&model_id);
            Ok(EmbeddingModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create a Google image model instance.
#[napi]
pub async fn google_image(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<ImageModel> {
    AimuxResult({
        let __r: crate::error::MResult<ImageModel> = async {
            use aimux_providers::google::{GoogleConfig, GoogleProvider};
            let mut config = GoogleConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = GoogleProvider::new(config);
            let model = provider.image(&model_id);
            Ok(ImageModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create a Google video model instance.
#[napi]
pub async fn google_video(
    api_key: String,
    model_id: String,
    base_url: Option<String>,
) -> AimuxResult<VideoModel> {
    AimuxResult({
        let __r: crate::error::MResult<VideoModel> = async {
            use aimux_providers::google::{GoogleConfig, GoogleProvider};
            let mut config = GoogleConfig::new(api_key);
            if let Some(url) = base_url {
                config = config.with_base_url(url);
            }
            let provider = GoogleProvider::new(config);
            let model = provider.video(&model_id);
            Ok(VideoModel {
                inner: Arc::new(model),
            })
        }
        .await;
        __r
    })
}

/// Create a Tavily search model instance.
#[napi]
pub async fn tavily_search(api_key: String, base_url: Option<String>) -> AimuxResult<SearchModel> {
    AimuxResult({
        let __r: crate::error::MResult<SearchModel> = async {
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
        .await;
        __r
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionSession (RFC-0028 streaming, native path)
// ─────────────────────────────────────────────────────────────────────────────

/// A live streaming-transcription session (RFC-0028). Push audio chunks with
/// `pushAudio`, mark end-of-audio with `inputDone`, then pull transcription
/// parts with `nextPart`. Mirrors the C-ABI session shape, built natively on
/// core channels.
#[napi]
pub struct TranscriptionSession {
    audio_tx: std::sync::Mutex<Option<futures::channel::mpsc::Sender<AudioChunk>>>,
    parts_rx: tokio::sync::Mutex<
        tokio::sync::mpsc::Receiver<std::result::Result<String, MappedError>>,
    >,
    token: aimux_core::shared::AbortSignal,
}

/// Start a streaming transcription session. `opts_json` (optional):
/// `{ "input_audio_format": {"format_type","rate"}, "provider_options",
/// "headers", "include_raw_chunks" }`. `bridge` — optional AbortSignal; firing
/// it aborts the session. The returned parts are JSON-serialized
/// `TranscriptionStreamPart`s.
#[napi]
pub async fn start_transcription_session(
    model: &TranscriptionModel,
    opts_json: Option<String>,
    bridge: Option<&crate::AbortBridge>,
) -> AimuxResult<TranscriptionSession> {
    AimuxResult({
        let __r: crate::error::MResult<TranscriptionSession> = async {
            #[derive(serde::Deserialize, Default)]
            struct SessionOpts {
                input_audio_format: Option<aimux_core::transcription_model::InputAudioFormat>,
                provider_options:
                    Option<std::collections::HashMap<String, serde_json::Value>>,
                headers: Option<std::collections::HashMap<String, String>>,
                include_raw_chunks: Option<bool>,
                timeout: Option<aimux_core::options::TimeoutConfiguration>,
            }
            let opts: SessionOpts = match opts_json.as_deref() {
                Some(json) if !json.trim().is_empty() && json.trim() != "null" => {
                    serde_json::from_str(json).map_err(|e| {
                        MappedError::from(&AiMuxError::InvalidArgument(format!(
                            "invalid opts_json: {e}"
                        )))
                    })?
                }
                _ => SessionOpts::default(),
            };

            // Effective abort = user bridge OR close token (linked).
            let token = aimux_core::shared::AbortSignal::new();
            let effective = aimux_core::shared::AbortSignal::new();
            let mut sources = vec![token.clone()];
            if let Some(b) = bridge {
                sources.push(b.core_signal());
            }
            for source in sources {
                let linked = effective.clone();
                napi::tokio::spawn(async move {
                    source.cancelled().await;
                    linked.abort();
                });
            }

            let (audio_tx, audio_rx) =
                futures::channel::mpsc::channel::<AudioChunk>(64);
            let (tx, rx) =
                tokio::sync::mpsc::channel::<std::result::Result<String, MappedError>>(256);

            let model = model.inner.clone();
            napi::tokio::spawn(async move {
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
                        // Immediate delivery when capacity allows (terminal
                        // errors are never preempted); abort only unblocks a
                        // full channel.
                        while let Some(item) = stream.next().await {
                            // In-stream errors reject from `nextPart` (issue
                            // #145, option A) instead of being smuggled through
                            // the data channel as a serialized `{"Err": ...}`
                            // part — matching the FFI session (Err items) and
                            // the other six bindings' exception sentinels.
                            let part = match item {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = tx.send(Err(MappedError::from(&e))).await;
                                    return;
                                }
                            };
                            let json = match serde_json::to_string(&part) {
                                Ok(j) => j,
                                Err(e) => {
                                    let _ = tx
                                        .send(Err(MappedError::from(&AiMuxError::JsonParse(
                                            format!("serialize part: {e}"),
                                        ))))
                                        .await;
                                    return;
                                }
                            };
                            loop {
                                match tx.try_send(Ok(json.clone())) {
                                    Ok(()) => break,
                                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                        // Full: block on send (waits for
                                        // capacity) unless aborted.
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
                        // Connect failure: deliver as the first channel item.
                        // try_send + abort-select (a full channel must not
                        // stall; abort covers the session-drop path).
                        loop {
                            match tx.try_send(Err(MappedError::from(&e))) {
                                Ok(()) => break,
                                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                    tokio::select! {
                                        _ = effective.cancelled() => return,
                                        res = tx.send(Err(MappedError::from(&e))) => {
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
        .await;
        __r
    })
}

#[napi]
impl TranscriptionSession {
    /// Push one binary audio chunk. Awaits while the internal channel is
    /// full (backpressure).
    #[napi]
    pub async fn push_audio(&self, data: napi::bindgen_prelude::Buffer) -> napi::Result<()> {
        use futures::SinkExt;
        let bytes = data.to_vec();
        // Clone the sender and drop the guard BEFORE awaiting: the mutex is
        // shared with the SYNC inputDone()/close() methods on the JS thread —
        // holding it across a backpressured send would wedge the event loop.
        let mut tx = {
            let guard = self
                .audio_tx
                .lock()
                .map_err(|_| napi::Error::new(napi::Status::GenericFailure, "session poisoned"))?;
            match guard.as_ref() {
                None => {
                    return Err(napi::Error::new(
                        napi::Status::GenericFailure,
                        "audio input already finished",
                    ));
                }
                Some(tx) => tx.clone(),
            }
        };
        tx.send(AudioChunk::Binary(bytes))
            .await
            .map_err(|_| napi::Error::new(napi::Status::GenericFailure, "session ended"))
    }

    /// Signal end-of-audio (idempotent).
    #[napi]
    pub fn input_done(&self) -> napi::Result<()> {
        self.audio_tx
            .lock()
            .map_err(|_| napi::Error::new(napi::Status::GenericFailure, "session poisoned"))?
            .take();
        Ok(())
    }

    /// Pull the next transcription part (JSON string). Resolves `null` when
    /// the stream ended normally. Rejects on error — including a timeout
    /// (no part within `timeoutMs`; the session stays live, call again).
    /// `timeoutMs`: >0 wait at most; 0 immediate poll; negative OR omitted
    /// = wait indefinitely.
    #[napi]
    pub async fn next_part(
        &self,
        timeout_ms: Option<i64>,
    ) -> AimuxResult<Option<String>> {
        AimuxResult({
            let __r: crate::error::MResult<Option<String>> = async {
                let mut rx = self.parts_rx.lock().await;
                let timeout = match timeout_ms {
                    Some(ms) if ms >= 0 => {
                        Some(std::time::Duration::from_millis(ms as u64))
                    }
                    _ => None,
                };
                let recv = rx.recv();
                let part = match timeout {
                    Some(d) => match tokio::time::timeout(d, recv).await {
                        Ok(p) => p,
                        Err(_) => {
                            return Err(MappedError::from(&AiMuxError::Timeout(
                                "no transcription part within timeout".into(),
                            )));
                        }
                    },
                    None => recv.await,
                };
                match part {
                    Some(Ok(json)) => Ok(Some(json)),
                    Some(Err(e)) => Err(e),
                    None => Ok(None), // ended normally
                }
            }
            .await;
            __r
        })
    }

    /// Terminate the session (aborts the driver). The object becomes inert;
    /// further `pushAudio`/`nextPart` fail. Call this promptly — GC teardown
    /// drops the channels (which eventually tears the driver down) but never
    /// fires the abort token.
    #[napi]
    pub fn close(&self) {
        // End audio + abort; the driver unblocks and exits.
        if let Ok(mut guard) = self.audio_tx.lock() {
            guard.take();
        }
        self.token.abort();
    }
}
