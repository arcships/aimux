//! # aimux-providers
//!
//! LLM provider implementations for aimux.
//!
//! Each provider implements the `LanguageModel` trait from `aimux-core`.

/// One required `Provider` method that this vendor does not offer: returns
/// `NoSuchModel` with the AI SDK `modelType`.
#[doc(hidden)]
#[macro_export]
macro_rules! __unsupported_required_model {
    ($method:ident, $trait:ident, $model_type:literal) => {
        fn $method(
            &self,
            id: &str,
        ) -> ::std::result::Result<
            ::std::sync::Arc<dyn ::aimux_core::$trait>,
            ::aimux_core::AiMuxError,
        > {
            ::std::result::Result::Err(::aimux_core::AiMuxError::no_such_model(id, $model_type))
        }
    };
}

/// Emit `impl Provider for $ty` for a vendor that offers exactly one modality.
///
/// The first argument after the type names the `Provider` method to wire
/// (`language_model`, `embedding_model`, `image_model`, `transcription_model`,
/// `speech_model`, `reranking_model`, `video_model` or `search_model`); the
/// closure-like tail binds the provider and the model id and evaluates to the
/// (infallible) concrete model. The two other required methods return
/// `NoSuchModel`; the other optional ones keep their `None` default.
///
/// ```ignore
/// impl_single_modality_provider!(VllmProvider, language_model, |p, id| p.model(id));
/// impl_single_modality_provider!(SerperProvider, search_model, |p, _id| p.search_model());
/// ```
#[macro_export]
macro_rules! impl_single_modality_provider {
    ($ty:ty, language_model, |$p:ident, $id:ident| $build:expr) => {
        impl ::aimux_core::Provider for $ty {
            fn language_model(
                &self,
                $id: &str,
            ) -> ::std::result::Result<
                ::std::sync::Arc<dyn ::aimux_core::LanguageModel>,
                ::aimux_core::AiMuxError,
            > {
                let $p = self;
                ::std::result::Result::Ok(::std::sync::Arc::new($build))
            }
            $crate::__unsupported_required_model!(
                embedding_model,
                EmbeddingModel,
                "embeddingModel"
            );
            $crate::__unsupported_required_model!(image_model, ImageModel, "imageModel");
        }
    };
    ($ty:ty, embedding_model, |$p:ident, $id:ident| $build:expr) => {
        impl ::aimux_core::Provider for $ty {
            $crate::__unsupported_required_model!(language_model, LanguageModel, "languageModel");
            fn embedding_model(
                &self,
                $id: &str,
            ) -> ::std::result::Result<
                ::std::sync::Arc<dyn ::aimux_core::EmbeddingModel>,
                ::aimux_core::AiMuxError,
            > {
                let $p = self;
                ::std::result::Result::Ok(::std::sync::Arc::new($build))
            }
            $crate::__unsupported_required_model!(image_model, ImageModel, "imageModel");
        }
    };
    ($ty:ty, image_model, |$p:ident, $id:ident| $build:expr) => {
        impl ::aimux_core::Provider for $ty {
            $crate::__unsupported_required_model!(language_model, LanguageModel, "languageModel");
            $crate::__unsupported_required_model!(
                embedding_model,
                EmbeddingModel,
                "embeddingModel"
            );
            fn image_model(
                &self,
                $id: &str,
            ) -> ::std::result::Result<
                ::std::sync::Arc<dyn ::aimux_core::ImageModel>,
                ::aimux_core::AiMuxError,
            > {
                let $p = self;
                ::std::result::Result::Ok(::std::sync::Arc::new($build))
            }
        }
    };
    ($ty:ty, $method:ident, |$p:ident, $id:ident| $build:expr) => {
        impl ::aimux_core::Provider for $ty {
            $crate::__unsupported_required_model!(language_model, LanguageModel, "languageModel");
            $crate::__unsupported_required_model!(
                embedding_model,
                EmbeddingModel,
                "embeddingModel"
            );
            $crate::__unsupported_required_model!(image_model, ImageModel, "imageModel");
            $crate::__optional_model!($method, |$p, $id| $build);
        }
    };
}

/// One optional `Provider` method wired to a concrete model (see
/// [`impl_single_modality_provider!`]).
#[doc(hidden)]
#[macro_export]
macro_rules! __optional_model {
    (transcription_model, |$p:ident, $id:ident| $build:expr) => {
        $crate::__optional_model!(@emit transcription_model, TranscriptionModel, |$p, $id| $build);
    };
    (speech_model, |$p:ident, $id:ident| $build:expr) => {
        $crate::__optional_model!(@emit speech_model, SpeechModel, |$p, $id| $build);
    };
    (reranking_model, |$p:ident, $id:ident| $build:expr) => {
        $crate::__optional_model!(@emit reranking_model, RerankingModel, |$p, $id| $build);
    };
    (video_model, |$p:ident, $id:ident| $build:expr) => {
        $crate::__optional_model!(@emit video_model, VideoModel, |$p, $id| $build);
    };
    (search_model, |$p:ident, $id:ident| $build:expr) => {
        $crate::__optional_model!(@emit search_model, SearchModel, |$p, $id| $build);
    };
    (@emit $method:ident, $trait:ident, |$p:ident, $id:ident| $build:expr) => {
        fn $method(
            &self,
            $id: &str,
        ) -> ::std::option::Option<
            ::std::result::Result<
                ::std::sync::Arc<dyn ::aimux_core::$trait>,
                ::aimux_core::AiMuxError,
            >,
        > {
            let $p = self;
            ::std::option::Option::Some(::std::result::Result::Ok(::std::sync::Arc::new($build)))
        }
    };
}

// Registry-backed provider construction: the embedded `provider_registry.json`
// is parsed once into `preset` descriptors and used by the by-name entry points.
pub mod provider;
pub mod replay;
pub use preset::{AuthMode, PresetDescriptor, PresetEntry, PresetSettings};
pub use replay::rebuild_provider;

pub mod catalogue;
pub use catalogue::{Catalogue, get_model_specs};

pub mod anthropic;
pub mod anthropic_aws;
pub mod azure;
pub mod bedrock;
pub mod cohere;
pub mod deepseek;
mod default_providers;
pub mod google;
pub mod groq;
pub mod mistral;
pub mod openai;
pub mod openai_compatible;
pub use default_providers::{create_provider, default_providers, provider_names};
pub mod preset;
pub(crate) mod shared;
pub mod vertex;
pub mod voyage;

pub mod codex;
pub mod xai;

pub mod huggingface;

// Speech-only providers (TTS).
pub mod cartesia;
pub mod elevenlabs;
pub mod hume;
pub mod lmnt;

// Transcription-only providers (STT).
pub mod assemblyai;
pub mod deepgram;
pub mod fal;
pub mod gladia;
pub mod revai;

// Image-only providers.
pub mod black_forest_labs;
pub mod luma;
pub mod prodia;
pub mod replicate;

// Video-only providers.
pub mod klingai;

// Generic Responses API wrapper.
pub mod open_responses;

// Unified logging entry point (RFC-0014) — re-export from provider-utils so
// Rust consumers and the FFI layer share one implementation.
pub use aimux_provider_utils::logging::init_logging;

pub use anthropic::{
    AnthropicMessagesModel, AnthropicProvider, AnthropicProviderSettings, create_anthropic,
};
pub use anthropic_aws::{
    AnthropicAwsAuth, AnthropicAwsProvider, AnthropicAwsProviderSettings, create_anthropic_aws,
};
pub use azure::{
    AzureChatModel, AzureOpenAIProvider, AzureOpenAIProviderSettings, AzureResponsesModel, azure,
    create_azure,
};
pub use bedrock::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, BedrockEmbeddingModel, BedrockImageModel,
    BedrockModel, BedrockRerankingModel, amazon_bedrock, create_amazon_bedrock,
};
pub use cohere::{
    CohereEmbeddingModel, CohereModel, CohereProvider, CohereProviderSettings,
    CohereRerankingModel, cohere, create_cohere,
};
pub use google::{
    GoogleEmbeddingModel, GoogleFiles, GoogleImageModel, GoogleImageSettings, GoogleModel,
    GoogleProvider, GoogleProviderSettings, GoogleVideoModel, create_google, google,
};
pub use mistral::{
    MistralEmbeddingModel, MistralModel, MistralProvider, MistralProviderSettings, create_mistral,
    mistral,
};
pub use openai::{
    OpenAIEmbeddingModel, OpenAIImageModel, OpenAIProvider, OpenAIProviderSettings,
    OpenAIResponsesModel, OpenAISpeechModel, OpenAITranscriptionModel, create_openai,
};
pub use vertex::{
    GoogleAuthOptions, GoogleAuthScopes, VertexAnthropicModel, VertexAnthropicProvider,
    VertexAnthropicProviderSettings, VertexEmbeddingModel, VertexImageModel, VertexModel,
    VertexProvider, VertexProviderSettings, VertexTranscriptionModel, VertexVideoModel,
    create_google_vertex, create_google_vertex_anthropic, google_vertex, google_vertex_anthropic,
};
pub use voyage::{
    VoyageEmbeddingModel, VoyageProvider, VoyageProviderSettings, VoyageRerankingModel,
    create_voyage, voyage,
};

pub use codex::{
    CODEX_API_BASE_URL, CODEX_API_KEY_ENV_VAR, CODEX_OAUTH_TOKEN_URL, CODEX_SUBSCRIPTION_BASE_URL,
    CodexMode, CodexModel, CodexProvider, CodexProviderSettings, CodexTokens, codex, codex_refresh,
    codex_refresh_at, create_codex,
};
pub use xai::{XAIProvider, XAIProviderSettings, XaiResponsesModel, create_xai, xai};

pub use cartesia::{
    CartesiaProvider, CartesiaProviderSettings, CartesiaSpeechModel, CartesiaTranscriptionModel,
    cartesia, create_cartesia,
};
pub use elevenlabs::{
    ElevenLabsProvider, ElevenLabsProviderSettings, ElevenLabsSpeechModel,
    ElevenLabsTranscriptionModel, create_elevenlabs, elevenlabs,
};
pub use huggingface::{
    HuggingFaceProvider, HuggingFaceProviderSettings, HuggingFaceResponsesModel,
    create_huggingface, huggingface,
};
pub use hume::{HumeProvider, HumeProviderSettings, HumeSpeechModel, create_hume, hume};
pub use lmnt::{LMNTProvider, LMNTProviderSettings, LMNTSpeechModel, create_lmnt, lmnt};

pub use assemblyai::{
    AssemblyAIProvider, AssemblyAIProviderSettings, AssemblyAITranscriptionModel, assemblyai,
    create_assemblyai,
};
pub use deepgram::{
    DeepgramProvider, DeepgramProviderSettings, DeepgramTranscriptionModel, create_deepgram,
    deepgram,
};
pub use fal::{
    FalImageModel, FalProvider, FalProviderSettings, FalTranscriptionModel, FalVideoModel,
    create_fal, fal,
};

// Image-only provider re-exports.
pub use black_forest_labs::{
    BlackForestLabsImageModel, BlackForestLabsProvider, BlackForestLabsProviderSettings,
    black_forest_labs, create_black_forest_labs,
};
pub use gladia::{
    GladiaProvider, GladiaProviderSettings, GladiaTranscriptionModel, create_gladia, gladia,
};
pub use klingai::{
    KlingAIProvider, KlingAIProviderSettings, KlingAIVideoModel, create_klingai, klingai,
};
pub use luma::{LumaImageModel, LumaProvider, LumaProviderSettings, create_luma, luma};
pub use prodia::{
    ProdiaImageModel, ProdiaProvider, ProdiaProviderSettings, ProdiaVideoModel, create_prodia,
    prodia,
};
pub use replicate::{
    ReplicateImageModel, ReplicateProvider, ReplicateProviderSettings, ReplicateVideoModel,
    create_replicate, replicate,
};
pub use revai::{
    RevaiProvider, RevaiProviderSettings, RevaiTranscriptionModel, create_revai, revai,
};

pub use open_responses::{
    OpenResponsesModel, OpenResponsesProvider, OpenResponsesProviderSettings, create_open_responses,
};

// Modality-specific providers (non-language, e.g. rerank-only).
pub mod jina_ai;
pub use jina_ai::{
    JinaAiProvider, JinaAiProviderSettings, JinaAiRerankingModel, create_jina_ai, jina_ai,
};

// AWS Polly speech (TTS) provider — SigV4 authenticated, speech modality only.
pub mod aws_polly;
pub use aws_polly::{
    AwsPollyProvider, AwsPollyProviderSettings, AwsPollySpeechModel, aws_polly, create_aws_polly,
};

// Recraft image provider (OpenAI Images-compatible + Recraft extension fields).
pub mod recraft;
pub use recraft::{
    RecraftImageModel, RecraftProvider, RecraftProviderSettings, create_recraft, recraft,
};

// Stability image provider (image modality only).
pub mod stability;
pub use stability::{
    StabilityImageModel, StabilityProvider, StabilityProviderSettings, create_stability, stability,
};

// Video-only provider (runwayml).
pub mod runwayml;
pub use runwayml::{
    RunwaymlProvider, RunwaymlProviderSettings, RunwaymlVideoModel, create_runwayml, runwayml,
};

// Search-only providers (web search modality).
pub mod dataforseo;
pub mod exa_ai;
pub mod firecrawl;
pub mod google_pse;
pub mod linkup;
pub mod parallel_ai;
pub mod searxng;
pub mod serper;
pub mod tavily;
pub mod tinyfish;
pub mod you_com;

pub use dataforseo::{
    DataforseoProvider, DataforseoProviderSettings, DataforseoSearchModel, create_dataforseo,
    dataforseo,
};
pub use exa_ai::{ExaAiProvider, ExaAiProviderSettings, ExaAiSearchModel, create_exa_ai, exa_ai};
pub use firecrawl::{
    FirecrawlProvider, FirecrawlProviderSettings, FirecrawlSearchModel, create_firecrawl, firecrawl,
};
pub use google_pse::{
    GooglePseProvider, GooglePseProviderSettings, GooglePseSearchModel, create_google_pse,
    google_pse,
};
pub use linkup::{
    LinkupProvider, LinkupProviderSettings, LinkupSearchModel, create_linkup, linkup,
};
pub use parallel_ai::{
    ParallelAiProvider, ParallelAiProviderSettings, ParallelAiSearchModel, create_parallel_ai,
    parallel_ai,
};
pub use searxng::{
    SearxngProvider, SearxngProviderSettings, SearxngSearchModel, create_searxng, searxng,
};
pub use serper::{
    SerperProvider, SerperProviderSettings, SerperSearchModel, create_serper, serper,
};
pub use tavily::{
    TavilyProvider, TavilyProviderSettings, TavilySearchModel, create_tavily, tavily,
};
pub use tinyfish::{
    TinyfishProvider, TinyfishProviderSettings, TinyfishSearchModel, create_tinyfish, tinyfish,
};
pub use you_com::{
    YouComProvider, YouComProviderSettings, YouComSearchModel, create_you_com, you_com,
};
