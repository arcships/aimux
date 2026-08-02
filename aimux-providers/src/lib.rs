//! # aimux-providers
//!
//! LLM provider implementations for aimux.
//!
//! Each provider implements the `LanguageModel` trait from `aimux-core`.

// Registry-backed provider construction (RFC-0017 phase 4): all built-in
// OpenAI-compatible providers are looked up by name from `provider_registry.json`.
// The 250 per-provider `XxxConfig`/`XxxProvider` shell types were retired in
// phase 4 — use [`provider`] / [`provider_from_env`] instead.
pub mod provider;
pub mod provider_name;
pub use provider::{ProviderOptions, provider, provider_from_env, provider_registry_entry};

pub mod anthropic;
pub mod anthropic_aws;
pub mod azure;
pub mod bedrock;
pub mod cohere;
pub mod google;
pub mod mistral;
pub mod openai;
pub mod vertex;
pub mod voyage;

pub mod openrouter;
pub mod xai;

// OpenAI-compatible thin wrappers (second batch).
pub mod huggingface;
pub mod llamafile;
pub mod lmstudio;
pub mod mistralrs;
pub mod ollama;

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

// Bulk-generated thin-wrapper providers.
pub mod cybertron;
pub mod docker_model_runner;
pub mod gaudi;
pub mod jlama;
pub mod litellm_proxy;
pub mod llamacpp;
pub mod local;
pub mod localai;
pub mod mlx;
pub mod omlx;
pub mod onnx;
pub mod oobabooba;
pub mod openvino;
pub mod sglang;
pub mod vllm;
pub mod xinference;

pub use anthropic::{AnthropicConfig, AnthropicProvider};
pub use anthropic_aws::{AnthropicAwsAuth, AnthropicAwsProvider, AnthropicAwsProviderConfig};
pub use azure::{
    AzureAuth, AzureConfig, AzureModel, AzureProvider, AzureResponsesModel, TokenProvider,
};
pub use bedrock::{
    BedrockAuth, BedrockEmbeddingModel, BedrockImageModel, BedrockProvider, BedrockProviderConfig,
};
pub use cohere::{CohereConfig, CohereEmbeddingModel, CohereProvider};
pub use google::{
    GoogleConfig, GoogleEmbeddingModel, GoogleImageModel, GoogleImageSettings, GoogleProvider,
    GoogleVideoModel,
};
pub use mistral::{MistralConfig, MistralEmbeddingModel, MistralProvider};
pub use openai::{
    OpenAIConfig, OpenAIEmbeddingModel, OpenAIImageModel, OpenAIProvider, OpenAIResponsesModel,
    OpenAISpeechModel, OpenAITranscriptionModel,
};
pub use vertex::{
    VertexAuth, VertexEmbeddingModel, VertexImageModel, VertexProvider, VertexProviderConfig,
    VertexTranscriptionModel, VertexVideoModel,
};
pub use voyage::{VoyageConfig, VoyageEmbeddingModel, VoyageProvider};

pub use openrouter::{OpenRouterConfig, OpenRouterProvider};
pub use xai::{XAIConfig, XAIProvider};

pub use cartesia::{
    CartesiaConfig, CartesiaProvider, CartesiaSpeechModel, CartesiaTranscriptionModel,
};
pub use elevenlabs::{
    ElevenLabsConfig, ElevenLabsProvider, ElevenLabsSpeechModel, ElevenLabsTranscriptionModel,
};
pub use huggingface::{HuggingFaceConfig, HuggingFaceProvider};
pub use hume::{HumeConfig, HumeProvider, HumeSpeechModel};
pub use llamafile::{LlamafileConfig, LlamafileProvider};
pub use lmnt::{LMNTConfig, LMNTProvider, LMNTSpeechModel};
pub use mistralrs::{MistralrsConfig, MistralrsProvider};

pub use lmstudio::{LmStudioConfig, LmStudioProvider};
pub use ollama::{OllamaConfig, OllamaProvider};

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

// Bulk-generated provider re-exports.
pub use cybertron::{CybertronConfig, CybertronProvider};
pub use docker_model_runner::{DockerModelRunnerConfig, DockerModelRunnerProvider};
pub use gaudi::{GaudiConfig, GaudiProvider};
pub use jlama::{JlamaConfig, JlamaProvider};
pub use litellm_proxy::{LitellmProxyConfig, LitellmProxyProvider};
pub use llamacpp::{LlamacppConfig, LlamacppProvider};
pub use local::{LocalConfig, LocalProvider};
pub use localai::{LocalaiConfig, LocalaiProvider};
pub use mlx::{MlxConfig, MlxProvider};
pub use omlx::{OmlxConfig, OmlxProvider};
pub use onnx::{OnnxConfig, OnnxProvider};
pub use oobabooba::{OobaboobaConfig, OobaboobaProvider};
pub use openvino::{OpenvinoConfig, OpenvinoProvider};
pub use sglang::{SglangConfig, SglangProvider};
pub use vllm::{VllmConfig, VllmProvider};
pub use xinference::{XinferenceConfig, XinferenceProvider};

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

// P1 thin-wrapper providers (provider-research batch).
pub mod bedrock_mantle;

pub use bedrock_mantle::{BedrockMantleConfig, BedrockMantleProvider};

// Vertex AI MaaS partner-model providers (OpenAI-compatible thin wrappers).
// Each wraps the shared OpenAIProvider against the Vertex AI MaaS OpenAPI
// endpoint, authenticating with a Google Cloud Bearer token.
pub mod vertex_ai_ai21_models;
pub mod vertex_ai_anthropic_models;
pub mod vertex_ai_deepseek_models;
pub mod vertex_ai_llama_models;
pub mod vertex_ai_minimax_models;
pub mod vertex_ai_mistral_models;
pub mod vertex_ai_moonshot_models;
pub mod vertex_ai_openai_models;
pub mod vertex_ai_qwen_models;
pub mod vertex_ai_zai_models;

pub use vertex_ai_ai21_models::{VertexAiAi21ModelsConfig, VertexAiAi21ModelsProvider};
pub use vertex_ai_anthropic_models::{
    VertexAiAnthropicModelsConfig, VertexAiAnthropicModelsProvider,
};
pub use vertex_ai_deepseek_models::{VertexAiDeepseekModelsConfig, VertexAiDeepseekModelsProvider};
pub use vertex_ai_llama_models::{VertexAiLlamaModelsConfig, VertexAiLlamaModelsProvider};
pub use vertex_ai_minimax_models::{VertexAiMinimaxModelsConfig, VertexAiMinimaxModelsProvider};
pub use vertex_ai_mistral_models::{VertexAiMistralModelsConfig, VertexAiMistralModelsProvider};
pub use vertex_ai_moonshot_models::{VertexAiMoonshotModelsConfig, VertexAiMoonshotModelsProvider};
pub use vertex_ai_openai_models::{VertexAiOpenaiModelsConfig, VertexAiOpenaiModelsProvider};
pub use vertex_ai_qwen_models::{VertexAiQwenModelsConfig, VertexAiQwenModelsProvider};
pub use vertex_ai_zai_models::{VertexAiZaiModelsConfig, VertexAiZaiModelsProvider};

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
