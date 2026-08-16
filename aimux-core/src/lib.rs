//! # aimux-core
//!
//! Core abstractions for the aimux toolkit.
//!
//! Architecture (aligned with Vercel AI SDK V4):
//!
//! ```text
//!   User-facing API          Provider-facing trait
//!   ──────────────           ─────────────────────
//!   generate_text()  ──►     LanguageModel::do_generate()
//!   stream_text()    ──►     LanguageModel::do_stream()
//! ```
//!
//! The user calls `generate_text` / `stream_text` (free functions).
//! These convert user messages into `LanguageModelPrompt`, build `CallOptions`,
//! and call `LanguageModel::do_generate` / `do_stream` on the provider implementation.

pub mod composite;
pub mod content;
pub mod embedding_model;
pub mod error;
pub mod files_model;
pub mod generate;
pub mod image_model;
pub mod json_repair;
pub mod language_model;
pub mod language_model_message;
pub mod math;
pub mod message;
pub mod moa;
pub mod model_catalogue;
pub mod model_id;
pub mod openai_output;
pub mod options;
pub mod provider;
pub mod recording;
pub mod replay;
pub mod reranking_model;
pub mod result;
pub mod router;
pub mod search_model;
pub mod session;
pub mod shared;
pub mod speech_model;
pub mod stream_part;
pub mod tool;
pub mod trace;
pub mod transcription_model;
pub mod types;
pub mod util;
pub mod video_model;

/// Convenience re-exports.
pub mod prelude {
    pub use crate::composite::ChildModel;
    pub use crate::content::ContentPart;
    pub use crate::embedding_model::{EmbeddingCallOptions, EmbeddingModel, EmbeddingResult};
    pub use crate::error::{AiMuxError, ApiCallError};
    pub use crate::files_model::{Files, UploadFileCallOptions, UploadFileResult};
    pub use crate::generate::{
        GenerateObjectResult, GenerateTextOptions, GenerateTextResult, StreamTextResult,
        generate_object, generate_text, generate_text_as_openai, stream_text,
        stream_text_as_openai,
    };
    pub use crate::image_model::{ImageCallOptions, ImageModel, ImageResult};
    pub use crate::language_model::LanguageModel;
    pub use crate::language_model_message::LanguageModelPrompt;
    pub use crate::message::{MessageContent, ModelMessage, ModelPrompt, Role};
    pub use crate::moa::{MoaConfig, MoaFailMode, MoaModel};
    pub use crate::model_catalogue::{
        CatalogueSource, Modality, ModelCapabilities, ModelCost, ModelLimits, ModelModalities,
        ModelSpec, ModelType, ReasoningMode, ReasoningSpec, ReasoningVisibility, RuntimeModel,
    };
    pub use crate::model_id::ModelId;
    pub use crate::openai_output::{
        ChatCompletion, ChatCompletionChunk, ChatCompletionStream, DONE_FRAME, OpenAiStreamOptions,
        encode_chunk_sse, to_chat_completion, to_chat_completion_stream,
    };
    pub use crate::options::{CallOptions, ResponseFormat, ToolChoice};
    pub use crate::provider::Provider;
    pub use crate::reranking_model::{RerankingCallOptions, RerankingModel, RerankingResult};
    pub use crate::result::{GenerateResult, StreamResult};
    pub use crate::router::{
        FallbackPolicy, Router, RouterConfig, RouterModel, RuleRouter, WeightedRouter,
    };
    pub use crate::search_model::{
        SearchCallOptions, SearchModel, SearchResponse, SearchResult, SearchResultItem,
    };
    pub use crate::session::{
        SessionCall, SessionInferer, SessionSource, SessionStore, SessionView, init_session_infer,
        init_session_store, list_sessions, session_calls,
    };
    pub use crate::shared::{
        AbortSignal, AspectRatio, FileBytes, FileData, SharedHeaders, SharedProviderMetadata,
        SharedProviderOptions, SharedProviderReference, Size,
    };
    pub use crate::speech_model::{SpeechCallOptions, SpeechModel, SpeechResult};
    pub use crate::stream_part::StreamPart;
    pub use crate::tool::{FunctionTool, ProviderTool, Tool, ToolCall, ToolResult};
    pub use crate::transcription_model::{
        TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
    };
    pub use crate::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Usage, Warning};
    pub use crate::video_model::{VideoCallOptions, VideoModel, VideoResult};
}

// Root-level re-exports for convenience.
pub use embedding_model::EmbeddingModel;
pub use error::{AiMuxError, ApiCallError};
pub use files_model::Files;
pub use image_model::ImageModel;
pub use language_model::LanguageModel;
pub use provider::Provider;
pub use reranking_model::RerankingModel;
pub use search_model::SearchModel;
pub use speech_model::SpeechModel;
pub use transcription_model::TranscriptionModel;
pub use video_model::VideoModel;
