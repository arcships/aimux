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

// Registry-backed provider construction (RFC-0017 phase 4, RFC-0036): every
// OpenAI-compatible vendor is a row of `provider_registry.json`, compiled by
// `scripts/gen_presets.py` into the explicit factories of `presets`
// (`presets::create_<name>(PresetSettings)`, `presets::<name>()`) and looked up
// by name through [`provider`] / [`provider_from_env`]. The hand-written thin
// wrapper types (`OllamaConfig`, `VllmProvider`, ...) are gone.
#[doc(hidden)]
pub mod body_merge;
pub mod provider;
pub mod replay;
pub use preset::{AuthMode, PresetDescriptor, PresetEntry, PresetFamily, PresetSettings};
pub use provider::{
    ExternalProviderEntry, ProviderOptions, ProviderProfile, is_external_provider,
    load_providers_from_json, provider, provider_discovery, provider_from_env, provider_handle,
    provider_names, provider_registry_entry, register_provider, reject_removed_provider_options,
};
pub use replay::rebuild_provider;

pub mod catalogue;
pub use catalogue::{Catalogue, get_model_specs};

pub mod anthropic;
pub mod anthropic_aws;
pub mod azure;
pub mod bedrock;
pub mod cohere;
pub mod deepseek;
pub mod google;
pub mod groq;
pub mod mistral;
pub mod openai;
pub mod openai_compatible;
pub mod preset;
pub mod presets;
pub(crate) mod shared;
pub mod vertex;
pub mod voyage;

pub mod codex;
pub mod xai;

// OpenAI-compatible wrapper that has not moved to its own package yet.
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
    AzureAuth, AzureConfig, AzureModel, AzureProvider, AzureResponsesModel, TokenProvider,
};
pub use bedrock::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, BedrockEmbeddingModel, BedrockImageModel,
    BedrockModel, BedrockRerankingModel, amazon_bedrock, create_amazon_bedrock,
};
pub use cohere::{CohereConfig, CohereEmbeddingModel, CohereProvider};
pub use google::{
    GoogleEmbeddingModel, GoogleFiles, GoogleImageModel, GoogleImageSettings, GoogleModel,
    GoogleProvider, GoogleProviderSettings, GoogleVideoModel, create_google, google,
};
pub use mistral::{MistralConfig, MistralEmbeddingModel, MistralProvider};
pub use openai::{
    OpenAIEmbeddingModel, OpenAIImageModel, OpenAIProvider, OpenAIProviderSettings,
    OpenAIResponsesModel, OpenAISpeechModel, OpenAITranscriptionModel, create_openai,
};
pub use vertex::{
    VertexAnthropicModel, VertexEmbeddingModel, VertexImageModel, VertexModel, VertexProvider,
    VertexProviderSettings, VertexTranscriptionModel, VertexVideoModel, create_google_vertex,
    google_vertex,
};
pub use voyage::{VoyageConfig, VoyageEmbeddingModel, VoyageProvider};

pub use codex::{
    CODEX_API_BASE_URL, CODEX_API_KEY_ENV_VAR, CODEX_OAUTH_TOKEN_URL, CODEX_SUBSCRIPTION_BASE_URL,
    CodexConfig, CodexMode, CodexModel, CodexProvider, CodexTokens, codex_refresh,
    codex_refresh_at,
};
pub use xai::{XAIConfig, XAIProvider};

pub use cartesia::{
    CartesiaConfig, CartesiaProvider, CartesiaSpeechModel, CartesiaTranscriptionModel,
};
pub use elevenlabs::{
    ElevenLabsConfig, ElevenLabsProvider, ElevenLabsSpeechModel, ElevenLabsTranscriptionModel,
};
pub use huggingface::{HuggingFaceConfig, HuggingFaceProvider};
pub use hume::{HumeConfig, HumeProvider, HumeSpeechModel};
pub use lmnt::{LMNTConfig, LMNTProvider, LMNTSpeechModel};

pub use assemblyai::{AssemblyAIConfig, AssemblyAIProvider, AssemblyAITranscriptionModel};
pub use deepgram::{DeepgramConfig, DeepgramProvider, DeepgramTranscriptionModel};
pub use fal::{FalConfig, FalImageModel, FalProvider, FalTranscriptionModel, FalVideoModel};

// Image-only provider re-exports.
pub use black_forest_labs::{
    BlackForestLabsConfig, BlackForestLabsImageModel, BlackForestLabsProvider,
};
pub use gladia::{GladiaConfig, GladiaProvider, GladiaTranscriptionModel};
pub use klingai::{KlingAIConfig, KlingAIProvider, KlingAIVideoModel};
pub use luma::{LumaConfig, LumaImageModel, LumaProvider};
pub use prodia::{ProdiaConfig, ProdiaImageModel, ProdiaProvider, ProdiaVideoModel};
pub use replicate::{ReplicateConfig, ReplicateImageModel, ReplicateProvider, ReplicateVideoModel};
pub use revai::{RevaiConfig, RevaiProvider, RevaiTranscriptionModel};

pub use open_responses::{OpenResponsesConfig, OpenResponsesModel, OpenResponsesProvider};

// Modality-specific providers (non-language, e.g. rerank-only).
pub mod jina_ai;
pub use jina_ai::{JinaAiConfig, JinaAiProvider, JinaAiRerankingModel};

// AWS Polly speech (TTS) provider — SigV4 authenticated, speech modality only.
pub mod aws_polly;
pub use aws_polly::{AwsPollyConfig, AwsPollyProvider, AwsPollySpeechModel};

// Recraft image provider (OpenAI Images-compatible + Recraft extension fields).
pub mod recraft;
pub use recraft::{RecraftConfig, RecraftImageModel, RecraftProvider};

// Stability image provider (image modality only).
pub mod stability;
pub use stability::{StabilityConfig, StabilityImageModel, StabilityProvider};

// Video-only provider (runwayml).
pub mod runwayml;
pub use runwayml::{RunwaymlConfig, RunwaymlProvider, RunwaymlVideoModel};

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

pub use dataforseo::{DataforseoConfig, DataforseoProvider, DataforseoSearchModel};
pub use exa_ai::{ExaAiConfig, ExaAiProvider, ExaAiSearchModel};
pub use firecrawl::{FirecrawlConfig, FirecrawlProvider, FirecrawlSearchModel};
pub use google_pse::{GooglePseConfig, GooglePseProvider, GooglePseSearchModel};
pub use linkup::{LinkupConfig, LinkupProvider, LinkupSearchModel};
pub use parallel_ai::{ParallelAiConfig, ParallelAiProvider, ParallelAiSearchModel};
pub use searxng::{SearxngConfig, SearxngProvider, SearxngSearchModel};
pub use serper::{SerperConfig, SerperProvider, SerperSearchModel};
pub use tavily::{TavilyConfig, TavilyProvider, TavilySearchModel};
pub use tinyfish::{TinyfishConfig, TinyfishProvider, TinyfishSearchModel};
pub use you_com::{YouComConfig, YouComProvider, YouComSearchModel};
