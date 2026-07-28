//! # aimux-providers
//!
//! LLM provider implementations for aimux.
//!
//! Each provider implements the `LanguageModel` trait from `aimux-core`.

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

pub mod cerebras;
pub mod deepseek;
pub mod fireworks;
pub mod groq;
pub mod moonshotai;
pub mod openrouter;
pub mod perplexity;
pub mod togetherai;
pub mod xai;

// OpenAI-compatible thin wrappers (second batch).
pub mod alibaba;
pub mod baseten;
pub mod bytedance;
pub mod copilot;
pub mod deepinfra;
pub mod doubleword;
pub mod github;
pub mod huggingface;
pub mod llamafile;
pub mod lmstudio;
pub mod mistralrs;
pub mod ollama;
pub mod sambanova;
pub mod siliconflow;
pub mod vercel;
pub mod zai;

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
pub mod ai21;
pub mod ai302;
pub mod aibadgr;
pub mod aigc2d;
pub mod aihubmix;
pub mod ails;
pub mod aiml;
pub mod albert;
pub mod anyscale;
pub mod api2d;
pub mod api2gpt;
pub mod apiserpent;
pub mod atlascloud;
pub mod azure_ai;
pub mod baichuan;
pub mod baidu;
pub mod bigmodel;
pub mod byteplus;
pub mod bytez;
pub mod canopywave;
pub mod chatgpt;
pub mod clarifai;
pub mod cline_pass;
pub mod closeai;
pub mod codestral;
pub mod cometapi;
pub mod commandcode;
pub mod compactifai;
pub mod coze;
pub mod cybertron;
pub mod databricks;
pub mod datarobot;
pub mod deepbricks;
pub mod docker_model_runner;
pub mod embercloud;
pub mod fastcrw;
pub mod fastgpt;
pub mod fastrouter;
pub mod featherless_ai;
pub mod friendliai;
pub mod galadriel;
pub mod gaudi;
pub mod gdc;
pub mod gigachat;
pub mod gonka24;
pub mod gradient_ai;
pub mod helicone;
pub mod heroku;
pub mod hosted_vllm;
pub mod hyperbolic;
pub mod inception;
pub mod inference_net;
pub mod infinity;
pub mod jlama;
pub mod kilo;
pub mod kiro;
pub mod kluster_ai;
pub mod krutrim;
pub mod lambda_ai;
pub mod lemonfox_ai;
pub mod lingyiwanwu;
pub mod litellm_proxy;
pub mod llamacpp;
pub mod local;
pub mod localai;
pub mod longcat;
pub mod matterai;
pub mod meta_llama;
pub mod minimax;
pub mod mira;
pub mod mlx;
pub mod modal;
pub mod modelscope;
pub mod morph;
pub mod nanogpt;
pub mod ncompass;
pub mod nebius;
pub mod nextbit;
pub mod nlp_cloud;
pub mod nous_research;
pub mod novita;
pub mod nscale;
pub mod nvidia_nim;
pub mod ohmygpt;
pub mod ollama_cloud;
pub mod omlx;
pub mod onnx;
pub mod oobabooba;
pub mod openaimax;
pub mod openaisb;
pub mod opencode_go;
pub mod opencode_zen;
pub mod openvino;
pub mod orcarouter;
pub mod ovhcloud;
pub mod parasail;
pub mod perfxcloud;
pub mod petals;
pub mod pioneer;
pub mod portkey;
pub mod predibase;
pub mod qihoo360;
pub mod qiniu_ai;
pub mod reka_ai;
pub mod requesty;
pub mod reve;
pub mod sakana;
pub mod sarvam;
pub mod scaleway;
pub mod scx_ai;
pub mod sglang;
pub mod stepfun;
pub mod submodel;
pub mod tencent;
pub mod tokenpony;
pub mod tundra;
pub mod upstage;
pub mod v0;
pub mod vllm;
pub mod wafer;
pub mod xiaomimimo;
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

pub use cerebras::{CerebrasConfig, CerebrasProvider};
pub use deepseek::{DeepSeekConfig, DeepSeekProvider};
pub use fireworks::{FireworksConfig, FireworksProvider};
pub use groq::{GroqConfig, GroqProvider};
pub use moonshotai::{MoonshotAIConfig, MoonshotAIProvider};
pub use openrouter::{OpenRouterConfig, OpenRouterProvider};
pub use perplexity::{PerplexityConfig, PerplexityProvider};
pub use togetherai::{TogetherAIConfig, TogetherAIProvider};
pub use xai::{XAIConfig, XAIProvider};

pub use alibaba::{AlibabaConfig, AlibabaProvider};
pub use baseten::{BasetenConfig, BasetenProvider};
pub use bytedance::{ByteDanceConfig, ByteDanceProvider};
pub use cartesia::{
    CartesiaConfig, CartesiaProvider, CartesiaSpeechModel, CartesiaTranscriptionModel,
};
pub use copilot::{CopilotConfig, CopilotProvider};
pub use deepinfra::{DeepInfraConfig, DeepInfraProvider};
pub use doubleword::{DoublewordConfig, DoublewordProvider};
pub use elevenlabs::{
    ElevenLabsConfig, ElevenLabsProvider, ElevenLabsSpeechModel, ElevenLabsTranscriptionModel,
};
pub use huggingface::{HuggingFaceConfig, HuggingFaceProvider};
pub use hume::{HumeConfig, HumeProvider, HumeSpeechModel};
pub use llamafile::{LlamafileConfig, LlamafileProvider};
pub use lmnt::{LMNTConfig, LMNTProvider, LMNTSpeechModel};
pub use mistralrs::{MistralrsConfig, MistralrsProvider};
pub use vercel::{VercelConfig, VercelProvider};

pub use github::{GithubConfig, GithubProvider};
pub use lmstudio::{LmStudioConfig, LmStudioProvider};
pub use ollama::{OllamaConfig, OllamaProvider};
pub use sambanova::{SambaNovaConfig, SambaNovaProvider};
pub use siliconflow::{SiliconFlowConfig, SiliconFlowProvider};
pub use zai::{ZaiConfig, ZaiProvider};

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
pub use ai21::{Ai21Config, Ai21Provider};
pub use ai302::{Ai302Config, Ai302Provider};
pub use aibadgr::{AibadgrConfig, AibadgrProvider};
pub use aigc2d::{Aigc2dConfig, Aigc2dProvider};
pub use aihubmix::{AihubmixConfig, AihubmixProvider};
pub use ails::{AilsConfig, AilsProvider};
pub use aiml::{AimlConfig, AimlProvider};
pub use albert::{AlbertConfig, AlbertProvider};
pub use anyscale::{AnyscaleConfig, AnyscaleProvider};
pub use api2d::{Api2dConfig, Api2dProvider};
pub use api2gpt::{Api2gptConfig, Api2gptProvider};
pub use apiserpent::{ApiserpentConfig, ApiserpentProvider};
pub use atlascloud::{AtlascloudConfig, AtlascloudProvider};
pub use azure_ai::{AzureAiConfig, AzureAiProvider};
pub use baichuan::{BaichuanConfig, BaichuanProvider};
pub use baidu::{BaiduConfig, BaiduProvider};
pub use bigmodel::{BigmodelConfig, BigmodelProvider};
pub use byteplus::{ByteplusConfig, ByteplusProvider};
pub use bytez::{BytezConfig, BytezProvider};
pub use canopywave::{CanopywaveConfig, CanopywaveProvider};
pub use chatgpt::{ChatgptConfig, ChatgptProvider};
pub use clarifai::{ClarifaiConfig, ClarifaiProvider};
pub use cline_pass::{ClinePassConfig, ClinePassProvider};
pub use closeai::{CloseaiConfig, CloseaiProvider};
pub use codestral::{CodestralConfig, CodestralProvider};
pub use cometapi::{CometapiConfig, CometapiProvider};
pub use commandcode::{CommandcodeConfig, CommandcodeProvider};
pub use compactifai::{CompactifaiConfig, CompactifaiProvider};
pub use coze::{CozeConfig, CozeProvider};
pub use cybertron::{CybertronConfig, CybertronProvider};
pub use databricks::{DatabricksConfig, DatabricksProvider};
pub use datarobot::{DatarobotConfig, DatarobotProvider};
pub use deepbricks::{DeepbricksConfig, DeepbricksProvider};
pub use docker_model_runner::{DockerModelRunnerConfig, DockerModelRunnerProvider};
pub use embercloud::{EmbercloudConfig, EmbercloudProvider};
pub use fastcrw::{FastcrwConfig, FastcrwProvider};
pub use fastgpt::{FastgptConfig, FastgptProvider};
pub use fastrouter::{FastrouterConfig, FastrouterProvider};
pub use featherless_ai::{FeatherlessAiConfig, FeatherlessAiProvider};
pub use friendliai::{FriendliAIConfig, FriendliAIProvider};
pub use galadriel::{GaladrielConfig, GaladrielProvider};
pub use gaudi::{GaudiConfig, GaudiProvider};
pub use gdc::{GdcConfig, GdcProvider};
pub use gigachat::{GigaChatConfig, GigaChatProvider};
pub use gonka24::{Gonka24Config, Gonka24Provider};
pub use gradient_ai::{GradientAiConfig, GradientAiProvider};
pub use helicone::{HeliconeConfig, HeliconeProvider};
pub use heroku::{HerokuConfig, HerokuProvider};
pub use hosted_vllm::{HostedVllmConfig, HostedVllmProvider};
pub use hyperbolic::{HyperbolicConfig, HyperbolicProvider};
pub use inception::{InceptionConfig, InceptionProvider};
pub use inference_net::{InferenceNetConfig, InferenceNetProvider};
pub use infinity::{InfinityConfig, InfinityProvider};
pub use jlama::{JlamaConfig, JlamaProvider};
pub use kilo::{KiloConfig, KiloProvider};
pub use kiro::{KiroConfig, KiroProvider};
pub use kluster_ai::{KlusterAiConfig, KlusterAiProvider};
pub use krutrim::{KrutrimConfig, KrutrimProvider};
pub use lambda_ai::{LambdaAiConfig, LambdaAiProvider};
pub use lemonfox_ai::{LemonfoxAiConfig, LemonfoxAiProvider};
pub use lingyiwanwu::{LingyiwanwuConfig, LingyiwanwuProvider};
pub use litellm_proxy::{LitellmProxyConfig, LitellmProxyProvider};
pub use llamacpp::{LlamacppConfig, LlamacppProvider};
pub use local::{LocalConfig, LocalProvider};
pub use localai::{LocalaiConfig, LocalaiProvider};
pub use longcat::{LongcatConfig, LongcatProvider};
pub use matterai::{MatteraiConfig, MatteraiProvider};
pub use meta_llama::{MetaLlamaConfig, MetaLlamaProvider};
pub use minimax::{MinimaxConfig, MinimaxProvider};
pub use mira::{MiraConfig, MiraProvider};
pub use mlx::{MlxConfig, MlxProvider};
pub use modal::{ModalConfig, ModalProvider};
pub use modelscope::{ModelscopeConfig, ModelscopeProvider};
pub use morph::{MorphConfig, MorphProvider};
pub use nanogpt::{NanogptConfig, NanogptProvider};
pub use ncompass::{NcompassConfig, NcompassProvider};
pub use nebius::{NebiusConfig, NebiusProvider};
pub use nextbit::{NextbitConfig, NextbitProvider};
pub use nlp_cloud::{NlpCloudConfig, NlpCloudProvider};
pub use nous_research::{NousResearchConfig, NousResearchProvider};
pub use novita::{NovitaConfig, NovitaProvider};
pub use nscale::{NscaleConfig, NscaleProvider};
pub use nvidia_nim::{NvidiaNimConfig, NvidiaNimProvider};
pub use ohmygpt::{OhmygptConfig, OhmygptProvider};
pub use ollama_cloud::{OllamaCloudConfig, OllamaCloudProvider};
pub use omlx::{OmlxConfig, OmlxProvider};
pub use onnx::{OnnxConfig, OnnxProvider};
pub use oobabooba::{OobaboobaConfig, OobaboobaProvider};
pub use openaimax::{OpenaimaxConfig, OpenaimaxProvider};
pub use openaisb::{OpenaisbConfig, OpenaisbProvider};
pub use opencode_go::{OpencodeGoConfig, OpencodeGoProvider};
pub use opencode_zen::{OpencodeZenConfig, OpencodeZenProvider};
pub use openvino::{OpenvinoConfig, OpenvinoProvider};
pub use orcarouter::{OrcarouterConfig, OrcarouterProvider};
pub use ovhcloud::{OvhcloudConfig, OvhcloudProvider};
pub use parasail::{ParasailConfig, ParasailProvider};
pub use perfxcloud::{PerfxcloudConfig, PerfxcloudProvider};
pub use petals::{PetalsConfig, PetalsProvider};
pub use pioneer::{PioneerConfig, PioneerProvider};
pub use portkey::{PortkeyConfig, PortkeyProvider};
pub use predibase::{PredibaseConfig, PredibaseProvider};
pub use qihoo360::{Qihoo360Config, Qihoo360Provider};
pub use qiniu_ai::{QiniuAiConfig, QiniuAiProvider};
pub use reka_ai::{RekaAiConfig, RekaAiProvider};
pub use requesty::{RequestyConfig, RequestyProvider};
pub use reve::{ReveConfig, ReveProvider};
pub use sakana::{SakanaConfig, SakanaProvider};
pub use sarvam::{SarvamConfig, SarvamProvider};
pub use scaleway::{ScalewayConfig, ScalewayProvider};
pub use scx_ai::{ScxAiConfig, ScxAiProvider};
pub use sglang::{SglangConfig, SglangProvider};
pub use stepfun::{StepfunConfig, StepfunProvider};
pub use submodel::{SubmodelConfig, SubmodelProvider};
pub use tencent::{TencentConfig, TencentProvider};
pub use tokenpony::{TokenponyConfig, TokenponyProvider};
pub use tundra::{TundraConfig, TundraProvider};
pub use upstage::{UpstageConfig, UpstageProvider};
pub use v0::{V0Config, V0Provider};
pub use vllm::{VllmConfig, VllmProvider};
pub use wafer::{WaferConfig, WaferProvider};
pub use xiaomimimo::{XiaomimimoConfig, XiaomimimoProvider};
pub use xinference::{XinferenceConfig, XinferenceProvider};

// Modality-specific providers (non-language, e.g. rerank-only).
pub mod jina_ai;
pub use jina_ai::{JinaAiConfig, JinaAiProvider, JinaAiRerankingModel};

// P0 thin-wrapper providers (provider-research batch).
pub mod abacus;
pub mod abliteration_ai;
pub mod aiand;
pub mod ambient;
pub mod umans_ai;
pub mod venice;

pub use abacus::{AbacusConfig, AbacusProvider};
pub use abliteration_ai::{AbliterationAiConfig, AbliterationAiProvider};
pub use aiand::{AiandConfig, AiandProvider};
pub use ambient::{AmbientConfig, AmbientProvider};
pub use umans_ai::{UmansAiConfig, UmansAiProvider};
pub use venice::{VeniceConfig, VeniceProvider};

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
pub mod ai_router;
pub mod aki_io;
pub mod alibaba_coding_plan;
pub mod alibaba_coding_plan_cn;
pub mod alibaba_token_plan;
pub mod alibaba_token_plan_cn;
pub mod anyapi;
pub mod auriko;
pub mod baidu_v2;
pub mod bailing;
pub mod bedrock_mantle;
pub mod berget;
pub mod cherryin;
pub mod chutes;
pub mod claudinio;
pub mod cloudferro_sherlock;
pub mod cloudflare_workers_ai;
pub mod cortecs;
pub mod crof;
pub mod crossmodel;
pub mod crusoe;
pub mod daoxe;
pub mod digitalocean;
pub mod dinference;
pub mod doubao;
pub mod drun;
pub mod ebcloud;
pub mod empiriolabs;
pub mod evroc;
pub mod frogbot;
pub mod gmicloud;
pub mod hpc_ai;
pub mod inceptron;
pub mod inferx;
pub mod io_net;
pub mod jiekou;
pub mod kenari;
pub mod kimi;
pub mod kimi_for_coding;
pub mod lilac;
pub mod llama;
pub mod llamagate;
pub mod llmgateway;
pub mod llmtr;
pub mod lucidquery;
pub mod meganova;
pub mod merge_gateway;
pub mod meta;
pub mod mimo;
pub mod minimax_cn;
pub mod mixlayer;
pub mod moark;
pub mod model_oracle_ai;
pub mod nearai;
pub mod neon;
pub mod neuralwatt;
pub mod oci;
pub mod ofox;
pub mod perplexity_agent;
pub mod poe;
pub mod poolside;
pub mod ppinfra;
pub mod qihang_ai;
pub mod regolo_ai;
pub mod routing_run;
pub mod snowflake_cortex;
pub mod stackit;
pub mod stepfun_ai_step_plan;
pub mod stepfun_step_plan;
pub mod subconscious;
pub mod tencent_tokenhub;
pub mod the_grid_ai;
pub mod tokenflux;
pub mod trustedrouter;
pub mod unorouter;
pub mod vivgrid;
pub mod volc_engine;
pub mod vultr;
pub mod wandb;
pub mod xunfei;
pub mod zai_coding_plan;
pub mod zenmux;
pub mod zhipu_v4;
pub mod zhipuai_coding_plan;

pub use ai_router::{AiRouterConfig, AiRouterProvider};
pub use aki_io::{AkiIoConfig, AkiIoProvider};
pub use alibaba_coding_plan::{AlibabaCodingPlanConfig, AlibabaCodingPlanProvider};
pub use alibaba_coding_plan_cn::{AlibabaCodingPlanCnConfig, AlibabaCodingPlanCnProvider};
pub use alibaba_token_plan::{AlibabaTokenPlanConfig, AlibabaTokenPlanProvider};
pub use alibaba_token_plan_cn::{AlibabaTokenPlanCnConfig, AlibabaTokenPlanCnProvider};
pub use anyapi::{AnyapiConfig, AnyapiProvider};
pub use auriko::{AurikoConfig, AurikoProvider};
pub use baidu_v2::{BaiduV2Config, BaiduV2Provider};
pub use bailing::{BailingConfig, BailingProvider};
pub use bedrock_mantle::{BedrockMantleConfig, BedrockMantleProvider};
pub use berget::{BergetConfig, BergetProvider};
pub use cherryin::{CherryinConfig, CherryinProvider};
pub use chutes::{ChutesConfig, ChutesProvider};
pub use claudinio::{ClaudinioConfig, ClaudinioProvider};
pub use cloudferro_sherlock::{CloudferroSherlockConfig, CloudferroSherlockProvider};
pub use cloudflare_workers_ai::{CloudflareWorkersAiConfig, CloudflareWorkersAiProvider};
pub use cortecs::{CortecsConfig, CortecsProvider};
pub use crof::{CrofConfig, CrofProvider};
pub use crossmodel::{CrossmodelConfig, CrossmodelProvider};
pub use crusoe::{CrusoeConfig, CrusoeProvider};
pub use daoxe::{DaoxeConfig, DaoxeProvider};
pub use digitalocean::{DigitaloceanConfig, DigitaloceanProvider};
pub use dinference::{DinferenceConfig, DinferenceProvider};
pub use doubao::{DoubaoConfig, DoubaoProvider};
pub use drun::{DrunConfig, DrunProvider};
pub use ebcloud::{EbcloudConfig, EbcloudProvider};
pub use empiriolabs::{EmpiriolabsConfig, EmpiriolabsProvider};
pub use evroc::{EvrocConfig, EvrocProvider};
pub use frogbot::{FrogbotConfig, FrogbotProvider};
pub use gmicloud::{GmicloudConfig, GmicloudProvider};
pub use hpc_ai::{HpcAiConfig, HpcAiProvider};
pub use inceptron::{InceptronConfig, InceptronProvider};
pub use inferx::{InferxConfig, InferxProvider};
pub use io_net::{IoNetConfig, IoNetProvider};
pub use jiekou::{JiekouConfig, JiekouProvider};
pub use kenari::{KenariConfig, KenariProvider};
pub use kimi::{KimiConfig, KimiProvider};
pub use kimi_for_coding::{KimiForCodingConfig, KimiForCodingProvider};
pub use lilac::{LilacConfig, LilacProvider};
pub use llama::{LlamaConfig, LlamaProvider};
pub use llamagate::{LlamagateConfig, LlamagateProvider};
pub use llmgateway::{LlmgatewayConfig, LlmgatewayProvider};
pub use llmtr::{LlmtrConfig, LlmtrProvider};
pub use lucidquery::{LucidqueryConfig, LucidqueryProvider};
pub use meganova::{MeganovaConfig, MeganovaProvider};
pub use merge_gateway::{MergeGatewayConfig, MergeGatewayProvider};
pub use meta::{MetaConfig, MetaProvider};
pub use mimo::{MimoConfig, MimoProvider};
pub use minimax_cn::{MinimaxCnConfig, MinimaxCnProvider};
pub use mixlayer::{MixlayerConfig, MixlayerProvider};
pub use moark::{MoarkConfig, MoarkProvider};
pub use model_oracle_ai::{ModelOracleAiConfig, ModelOracleAiProvider};
pub use nearai::{NearaiConfig, NearaiProvider};
pub use neon::{NeonConfig, NeonProvider};
pub use neuralwatt::{NeuralwattConfig, NeuralwattProvider};
pub use oci::{OciConfig, OciProvider};
pub use ofox::{OfoxConfig, OfoxProvider};
pub use perplexity_agent::{PerplexityAgentConfig, PerplexityAgentProvider};
pub use poe::{PoeConfig, PoeProvider};
pub use poolside::{PoolsideConfig, PoolsideProvider};
pub use ppinfra::{PpinfraConfig, PpinfraProvider};
pub use qihang_ai::{QihangAiConfig, QihangAiProvider};
pub use regolo_ai::{RegoloAiConfig, RegoloAiProvider};
pub use routing_run::{RoutingRunConfig, RoutingRunProvider};
pub use snowflake_cortex::{SnowflakeCortexConfig, SnowflakeCortexProvider};
pub use stackit::{StackitConfig, StackitProvider};
pub use stepfun_ai_step_plan::{StepfunAiStepPlanConfig, StepfunAiStepPlanProvider};
pub use stepfun_step_plan::{StepfunStepPlanConfig, StepfunStepPlanProvider};
pub use subconscious::{SubconsciousConfig, SubconsciousProvider};
pub use tencent_tokenhub::{TencentTokenhubConfig, TencentTokenhubProvider};
pub use the_grid_ai::{TheGridAiConfig, TheGridAiProvider};
pub use tokenflux::{TokenfluxConfig, TokenfluxProvider};
pub use trustedrouter::{TrustedrouterConfig, TrustedrouterProvider};
pub use unorouter::{UnorouterConfig, UnorouterProvider};
pub use vivgrid::{VivgridConfig, VivgridProvider};
pub use volc_engine::{VolcEngineConfig, VolcEngineProvider};
pub use vultr::{VultrConfig, VultrProvider};
pub use wandb::{WandbConfig, WandbProvider};
pub use xunfei::{XunfeiConfig, XunfeiProvider};
pub use zai_coding_plan::{ZaiCodingPlanConfig, ZaiCodingPlanProvider};
pub use zenmux::{ZenmuxConfig, ZenmuxProvider};
pub use zhipu_v4::{ZhipuV4Config, ZhipuV4Provider};
pub use zhipuai_coding_plan::{ZhipuaiCodingPlanConfig, ZhipuaiCodingPlanProvider};
