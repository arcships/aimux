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
    AudioInput, TranscriptionCallOptions, TranscriptionModel as TranscriptionModelTrait,
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
                    .map_err(|e| MappedError::from(&AiMuxError::Json(format!("serialize: {e}"))))
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
