//! The factories of the single-modality vendor packages.
//!
//! Every package follows the shape of the AI SDK's `createXxx`: settings are
//! validated once, the credential is evaluated on every request (never when the
//! provider is created, never from a stale copy), headers layer provider ->
//! call with `None` removing, the transport is the one the settings name, and
//! the `provider()` strings are `"{name}.{method}"`. The wire behavior of each
//! package is covered by its own test file; this file covers the factory
//! contract across all 28 packages through a scripted `Fetch` transport.

// The settings macros below name only the fields every package shares; the
// packages with more fields are completed by `..Default::default()`, which is a
// no-op for the ones that have none.
#![allow(clippy::needless_update)]

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::BoxFuture;
use serde_json::json;
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::provider::Provider;
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};
use aimux_core::search_model::{SearchCallOptions, SearchModel};
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_core::video_model::{VideoCallOptions, VideoModel};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable};

use aimux_providers::assemblyai::{AssemblyAIProviderSettings, assemblyai, create_assemblyai};
use aimux_providers::aws_polly::{AwsPollyProviderSettings, aws_polly, create_aws_polly};
use aimux_providers::black_forest_labs::{
    BlackForestLabsProviderSettings, black_forest_labs, create_black_forest_labs,
};
use aimux_providers::cartesia::{CartesiaProviderSettings, cartesia, create_cartesia};
use aimux_providers::dataforseo::{DataforseoProviderSettings, create_dataforseo, dataforseo};
use aimux_providers::deepgram::{DeepgramProviderSettings, create_deepgram, deepgram};
use aimux_providers::exa_ai::{ExaAiProviderSettings, create_exa_ai, exa_ai};
use aimux_providers::fal::{FalProviderSettings, create_fal, fal};
use aimux_providers::firecrawl::{FirecrawlProviderSettings, create_firecrawl, firecrawl};
use aimux_providers::gladia::{GladiaProviderSettings, create_gladia, gladia};
use aimux_providers::google_pse::{GooglePseProviderSettings, create_google_pse, google_pse};
use aimux_providers::hume::{HumeProviderSettings, create_hume, hume};
use aimux_providers::jina_ai::{JinaAiProviderSettings, create_jina_ai, jina_ai};
use aimux_providers::klingai::{KlingAIProviderSettings, create_klingai, klingai};
use aimux_providers::linkup::{LinkupProviderSettings, create_linkup, linkup};
use aimux_providers::lmnt::{LMNTProviderSettings, create_lmnt, lmnt};
use aimux_providers::luma::{LumaProviderSettings, create_luma, luma};
use aimux_providers::parallel_ai::{ParallelAiProviderSettings, create_parallel_ai, parallel_ai};
use aimux_providers::prodia::{ProdiaProviderSettings, create_prodia, prodia};
use aimux_providers::recraft::{RecraftProviderSettings, create_recraft, recraft};
use aimux_providers::replicate::{ReplicateProviderSettings, create_replicate, replicate};
use aimux_providers::revai::{RevaiProviderSettings, create_revai, revai};
use aimux_providers::runwayml::{RunwaymlProviderSettings, create_runwayml, runwayml};
use aimux_providers::searxng::{SearxngProviderSettings, create_searxng, searxng};
use aimux_providers::serper::{SerperProviderSettings, create_serper, serper};
use aimux_providers::stability::{StabilityProviderSettings, create_stability, stability};
use aimux_providers::tavily::{TavilyProviderSettings, create_tavily, tavily};
use aimux_providers::tinyfish::{TinyfishProviderSettings, create_tinyfish, tinyfish};
use aimux_providers::you_com::{YouComProviderSettings, create_you_com, you_com};

use mock_fetch::{Canned, EnvVar, MockFetch, RouteFetch};

/// Every environment variable the 28 packages read.
const ENV_VARS: &[&str] = &[
    "ASSEMBLYAI_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_REGION",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "BFL_API_KEY",
    "CARTESIA_API_KEY",
    "DATAFORSEO_LOGIN",
    "DATAFORSEO_PASSWORD",
    "DEEPGRAM_API_KEY",
    "EXA_API_KEY",
    "FAL_KEY",
    "FIRECRAWL_API_KEY",
    "GLADIA_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_CSE_ID",
    "HUME_API_KEY",
    "JINA_AI_API_KEY",
    "KLINGAI_API_KEY",
    "LINKUP_API_KEY",
    "LMNT_API_KEY",
    "LUMA_API_KEY",
    "PARALLEL_API_KEY",
    "PRODIA_API_KEY",
    "RECRAFT_API_TOKEN",
    "REPLICATE_API_TOKEN",
    "REVAI_API_KEY",
    "RUNWAYML_API_SECRET",
    "SEARXNG_URL",
    "SERPER_API_KEY",
    "STABILITY_API_KEY",
    "TAVILY_API_KEY",
    "TINYFISH_API_KEY",
    "YDC_API_KEY",
];

/// The environment of a test: every variable above cleared, restored on drop.
struct CleanEnv(#[allow(dead_code)] Vec<EnvVar>);

fn clean_env() -> CleanEnv {
    CleanEnv(
        ENV_VARS
            .iter()
            .map(|name| EnvVar::set(name, None))
            .collect(),
    )
}

fn headers(pairs: &[(&str, Option<&str>)]) -> HeaderMapOpt {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.map(str::to_string)))
        .collect()
}

/// The knobs every package's settings share.
#[derive(Clone)]
struct Knobs {
    key: Option<Resolvable<String>>,
    base_url: Option<String>,
    headers: Option<HeaderMapOpt>,
    name: Option<String>,
    fetch: FetchFunction,
}

impl Knobs {
    fn new(fetch: FetchFunction) -> Self {
        Self {
            key: None,
            base_url: Some("http://vendor.test".to_string()),
            headers: None,
            name: None,
            fetch,
        }
    }

    fn with_key(mut self, key: &str) -> Self {
        self.key = Some(Resolvable::Value(key.to_string()));
        self
    }

    /// The key as the plain-string setting of the AWS package.
    fn text(&self) -> Option<String> {
        match &self.key {
            Some(Resolvable::Value(text)) => Some(text.clone()),
            _ => None,
        }
    }
}

/// The settings of a package from the shared knobs (plus fields only it has).
macro_rules! settings {
    ($ty:ident, $k:expr $(, $field:ident : $value:expr)* $(,)?) => {
        $ty {
            api_key: $k.key.clone(),
            base_url: $k.base_url.clone(),
            headers: $k.headers.clone(),
            name: $k.name.clone(),
            fetch: Some($k.fetch.clone()),
            $($field: $value,)*
            ..Default::default()
        }
    };
}

/// The same for a package whose credential is not a single `api_key`.
macro_rules! settings_with {
    ($ty:ident, $k:expr $(, $field:ident : $value:expr)* $(,)?) => {
        $ty {
            base_url: $k.base_url.clone(),
            headers: $k.headers.clone(),
            name: $k.name.clone(),
            fetch: Some($k.fetch.clone()),
            $($field: $value,)*
            ..Default::default()
        }
    };
}

async fn call_search(m: impl SearchModel) -> Result<(), AiMuxError> {
    m.do_search(&SearchCallOptions::new("q")).await.map(|_| ())
}

async fn call_speech(m: impl SpeechModel) -> Result<(), AiMuxError> {
    let mut options = SpeechCallOptions::new("hi");
    options.voice = Some("v".to_string());
    m.do_generate(&options).await.map(|_| ())
}

async fn call_transcription(m: impl TranscriptionModel) -> Result<(), AiMuxError> {
    let options = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
    m.do_generate(&options).await.map(|_| ())
}

async fn call_image(m: impl ImageModel) -> Result<(), AiMuxError> {
    m.do_generate(&ImageCallOptions::new("p".to_string()))
        .await
        .map(|_| ())
}

async fn call_video(m: impl VideoModel) -> Result<(), AiMuxError> {
    m.do_start(&VideoCallOptions::new("p")).await.map(|_| ())
}

async fn call_reranking(m: impl RerankingModel) -> Result<(), AiMuxError> {
    let documents = RerankingDocuments::Text {
        values: vec!["a".to_string()],
    };
    m.do_rerank(&RerankingCallOptions::new("q", documents))
        .await
        .map(|_| ())
}

/// Builds a provider from a base URL.
type Create = fn(Option<String>) -> Result<Box<dyn Provider>, AiMuxError>;

/// The `provider()` string of a model of a provider with the given `name`.
type ProviderString = fn(Option<String>) -> Result<String, AiMuxError>;

type Call = fn(Knobs) -> BoxFuture<'static, Result<(), AiMuxError>>;

/// How a missing credential shows.
#[derive(Clone, Copy)]
enum Missing {
    /// `AiMuxError::LoadApiKey { env_var }`.
    ApiKey,
    /// `AiMuxError::LoadSetting { env_var }`.
    Setting,
}

struct Case {
    id: &'static str,
    env_var: &'static str,
    missing: Missing,
    /// Wire header expectations for the key `"k"`: `~` marks a prefix.
    auth: &'static [(&'static str, &'static str)],
    /// The default `name` of the package (the prefix of `provider()`).
    name: &'static str,
    /// The modalities the package offers (the model method names).
    offers: &'static [&'static str],
    call: Call,
    /// A provider from the given base URL, nothing else set.
    create: Create,
    /// The `provider()` string of the model, for the given `name` setting.
    provider_string: ProviderString,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            id: "serper.search",
            env_var: "SERPER_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "serper",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(create_serper(settings!(SerperProviderSettings, k))?.search_model())
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_serper(SerperProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_serper(SerperProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "prodia.image",
            env_var: "PRODIA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[
                ("x-prodia-key", "k"),
                ("accept", "multipart/form-data; image/png"),
            ],
            name: "prodia",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(create_prodia(settings!(ProdiaProviderSettings, k))?.image("m"))
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_prodia(ProdiaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_prodia(ProdiaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "prodia.video",
            env_var: "PRODIA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-prodia-key", "k")],
            name: "prodia",
            offers: &["video_model"],
            call: |k| {
                Box::pin(async move {
                    call_video(create_prodia(settings!(ProdiaProviderSettings, k))?.video("m"))
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_prodia(ProdiaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_prodia(ProdiaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .video("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "deepgram.transcription",
            env_var: "DEEPGRAM_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Token k")],
            name: "deepgram",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_deepgram(settings!(DeepgramProviderSettings, k))?.transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_deepgram(DeepgramProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_deepgram(DeepgramProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "you_com.search",
            env_var: "YDC_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "you_com",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_you_com(settings!(YouComProviderSettings, k))?.search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_you_com(YouComProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_you_com(YouComProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "luma.image",
            env_var: "LUMA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "luma",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(create_luma(settings!(LumaProviderSettings, k))?.image("m")).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_luma(LumaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_luma(LumaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "lmnt.speech",
            env_var: "LMNT_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "lmnt",
            offers: &["speech_model"],
            call: |k| {
                Box::pin(async move {
                    call_speech(create_lmnt(settings!(LMNTProviderSettings, k))?.speech("m")).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_lmnt(LMNTProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_lmnt(LMNTProviderSettings {
                    name,
                    ..Default::default()
                })?
                .speech("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "klingai.video",
            env_var: "KLINGAI_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "klingai",
            offers: &["video_model"],
            call: |k| {
                Box::pin(async move {
                    call_video(create_klingai(settings!(KlingAIProviderSettings, k))?.video("m"))
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_klingai(KlingAIProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_klingai(KlingAIProviderSettings {
                    name,
                    ..Default::default()
                })?
                .video("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "dataforseo.search",
            env_var: "DATAFORSEO_LOGIN",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Basic azpr")],
            name: "dataforseo",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(create_dataforseo(settings_with!(DataforseoProviderSettings, k, login: k.key.clone(), password: k.key.clone()))?.search_model()).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_dataforseo(DataforseoProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_dataforseo(DataforseoProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "replicate.image",
            env_var: "REPLICATE_API_TOKEN",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "replicate",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(
                        create_replicate(settings!(ReplicateProviderSettings, k))?.image("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_replicate(ReplicateProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_replicate(ReplicateProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "replicate.video",
            env_var: "REPLICATE_API_TOKEN",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Token k")],
            name: "replicate",
            offers: &["video_model"],
            call: |k| {
                Box::pin(async move {
                    call_video(
                        create_replicate(settings!(ReplicateProviderSettings, k))?.video("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_replicate(ReplicateProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_replicate(ReplicateProviderSettings {
                    name,
                    ..Default::default()
                })?
                .video("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "fal.image",
            env_var: "FAL_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Key k")],
            name: "fal",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(create_fal(settings!(FalProviderSettings, k))?.image("m")).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_fal(FalProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_fal(FalProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "fal.transcription",
            env_var: "FAL_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Key k")],
            name: "fal",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_fal(settings!(FalProviderSettings, k))?.transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_fal(FalProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_fal(FalProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "fal.video",
            env_var: "FAL_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Key k")],
            name: "fal",
            offers: &["video_model"],
            call: |k| {
                Box::pin(async move {
                    call_video(create_fal(settings!(FalProviderSettings, k))?.video("m")).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_fal(FalProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_fal(FalProviderSettings {
                    name,
                    ..Default::default()
                })?
                .video("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "recraft.image",
            env_var: "RECRAFT_API_TOKEN",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "recraft",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(create_recraft(settings!(RecraftProviderSettings, k))?.image("m"))
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_recraft(RecraftProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_recraft(RecraftProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "aws_polly.speech",
            env_var: "AWS_ACCESS_KEY_ID",
            missing: Missing::Setting,
            auth: &[("authorization", "~AWS4-HMAC-SHA256 Credential=k/")],
            name: "amazon-polly",
            offers: &["speech_model"],
            call: |k| {
                Box::pin(async move {
                    call_speech(create_aws_polly(settings_with!(AwsPollyProviderSettings, k, access_key_id: k.text(), secret_access_key: k.text(), region: Some("us-east-1".to_string())))?.speech("m")).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_aws_polly(AwsPollyProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_aws_polly(AwsPollyProviderSettings {
                    name,
                    ..Default::default()
                })?
                .speech("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "jina_ai.reranking",
            env_var: "JINA_AI_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "jina_ai",
            offers: &["reranking_model"],
            call: |k| {
                Box::pin(async move {
                    call_reranking(
                        create_jina_ai(settings!(JinaAiProviderSettings, k))?.reranking_model("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_jina_ai(JinaAiProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_jina_ai(JinaAiProviderSettings {
                    name,
                    ..Default::default()
                })?
                .reranking_model("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "gladia.transcription",
            env_var: "GLADIA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "gladia",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_gladia(settings!(GladiaProviderSettings, k))?.transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_gladia(GladiaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_gladia(GladiaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "tavily.search",
            env_var: "TAVILY_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "tavily",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(create_tavily(settings!(TavilyProviderSettings, k))?.search_model())
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_tavily(TavilyProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_tavily(TavilyProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "linkup.search",
            env_var: "LINKUP_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "linkup",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(create_linkup(settings!(LinkupProviderSettings, k))?.search_model())
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_linkup(LinkupProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_linkup(LinkupProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "tinyfish.search",
            env_var: "TINYFISH_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "tinyfish",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_tinyfish(settings!(TinyfishProviderSettings, k))?.search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_tinyfish(TinyfishProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_tinyfish(TinyfishProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "black_forest_labs.image",
            env_var: "BFL_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "blackForestLabs",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(
                        create_black_forest_labs(settings!(BlackForestLabsProviderSettings, k))?
                            .image("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_black_forest_labs(
                    BlackForestLabsProviderSettings {
                        base_url,
                        ..Default::default()
                    },
                )?))
            },
            provider_string: |name| {
                Ok(create_black_forest_labs(BlackForestLabsProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "assemblyai.transcription",
            env_var: "ASSEMBLYAI_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "k")],
            name: "assemblyai",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_assemblyai(settings!(AssemblyAIProviderSettings, k))?
                            .transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_assemblyai(AssemblyAIProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_assemblyai(AssemblyAIProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "revai.transcription",
            env_var: "REVAI_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "revai",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_revai(settings!(RevaiProviderSettings, k))?.transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_revai(RevaiProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_revai(RevaiProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "hume.speech",
            env_var: "HUME_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-hume-api-key", "k")],
            name: "hume",
            offers: &["speech_model"],
            call: |k| {
                Box::pin(async move {
                    call_speech(create_hume(settings!(HumeProviderSettings, k))?.speech()).await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_hume(HumeProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_hume(HumeProviderSettings {
                    name,
                    ..Default::default()
                })?
                .speech()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "cartesia.speech",
            env_var: "CARTESIA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[
                ("authorization", "Bearer k"),
                ("cartesia-version", "2026-03-01"),
            ],
            name: "cartesia",
            offers: &["speech_model"],
            call: |k| {
                Box::pin(async move {
                    call_speech(
                        create_cartesia(settings!(CartesiaProviderSettings, k))?.speech("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_cartesia(CartesiaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_cartesia(CartesiaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .speech("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "cartesia.transcription",
            env_var: "CARTESIA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[
                ("authorization", "Bearer k"),
                ("cartesia-version", "2026-03-01"),
            ],
            name: "cartesia",
            offers: &["transcription_model"],
            call: |k| {
                Box::pin(async move {
                    call_transcription(
                        create_cartesia(settings!(CartesiaProviderSettings, k))?.transcription("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_cartesia(CartesiaProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_cartesia(CartesiaProviderSettings {
                    name,
                    ..Default::default()
                })?
                .transcription("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "searxng.search",
            env_var: "SEARXNG_URL",
            missing: Missing::Setting,
            auth: &[("authorization", "Bearer k")],
            name: "searxng",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_searxng(settings!(SearxngProviderSettings, k))?.search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_searxng(SearxngProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_searxng(SearxngProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "parallel_ai.search",
            env_var: "PARALLEL_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "parallel_ai",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_parallel_ai(settings!(ParallelAiProviderSettings, k))?
                            .search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_parallel_ai(ParallelAiProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_parallel_ai(ParallelAiProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "firecrawl.search",
            env_var: "FIRECRAWL_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k")],
            name: "firecrawl",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_firecrawl(settings!(FirecrawlProviderSettings, k))?.search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_firecrawl(FirecrawlProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_firecrawl(FirecrawlProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "runwayml.video",
            env_var: "RUNWAYML_API_SECRET",
            missing: Missing::ApiKey,
            auth: &[
                ("authorization", "Bearer k"),
                ("x-runway-version", "2024-11-06"),
            ],
            name: "runwayml",
            offers: &["video_model"],
            call: |k| {
                Box::pin(async move {
                    call_video(create_runwayml(settings!(RunwaymlProviderSettings, k))?.video("m"))
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_runwayml(RunwaymlProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_runwayml(RunwaymlProviderSettings {
                    name,
                    ..Default::default()
                })?
                .video("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "exa_ai.search",
            env_var: "EXA_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("x-api-key", "k")],
            name: "exa_ai",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(create_exa_ai(settings!(ExaAiProviderSettings, k))?.search_model())
                        .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_exa_ai(ExaAiProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_exa_ai(ExaAiProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
        Case {
            id: "stability.image",
            env_var: "STABILITY_API_KEY",
            missing: Missing::ApiKey,
            auth: &[("authorization", "Bearer k"), ("accept", "image/*")],
            name: "stability",
            offers: &["image_model"],
            call: |k| {
                Box::pin(async move {
                    call_image(
                        create_stability(settings!(StabilityProviderSettings, k))?.image("m"),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_stability(StabilityProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_stability(StabilityProviderSettings {
                    name,
                    ..Default::default()
                })?
                .image("m")
                .provider()
                .to_string())
            },
        },
        Case {
            id: "google_pse.search",
            env_var: "GOOGLE_API_KEY",
            missing: Missing::ApiKey,
            auth: &[],
            name: "google_pse",
            offers: &["search_model"],
            call: |k| {
                Box::pin(async move {
                    call_search(
                        create_google_pse(
                            settings!(GooglePseProviderSettings, k, cx: Some("cx".to_string())),
                        )?
                        .search_model(),
                    )
                    .await
                })
            },
            create: |base_url| {
                Ok(Box::new(create_google_pse(GooglePseProviderSettings {
                    base_url,
                    ..Default::default()
                })?))
            },
            provider_string: |name| {
                Ok(create_google_pse(GooglePseProviderSettings {
                    name,
                    ..Default::default()
                })?
                .search_model()
                .provider()
                .to_string())
            },
        },
    ]
}

// ── provider strings, modalities, creation ───────────────────────────────────

#[test]
fn provider_strings_are_name_dot_method() {
    for case in cases() {
        let method = case.id.rsplit('.').next().unwrap();
        assert_eq!(
            (case.provider_string)(None).unwrap(),
            format!("{}.{method}", case.name),
            "{}",
            case.id
        );
        assert_eq!(
            (case.provider_string)(Some("proxy".to_string())).unwrap(),
            format!("proxy.{method}"),
            "{}",
            case.id
        );
    }
}

#[test]
fn a_vendor_serves_its_modality_and_says_no_such_model_for_the_others() {
    fn model_type<T: ?Sized>(result: Result<Arc<T>, AiMuxError>) -> String {
        match result {
            Err(AiMuxError::NoSuchModel { model_type, .. }) => model_type,
            Err(other) => panic!("expected NoSuchModel, got {other:?}"),
            Ok(_) => panic!("expected NoSuchModel"),
        }
    }
    for case in cases() {
        let provider = (case.create)(None).unwrap();
        let offered = |method: &str| {
            cases().iter().any(|other| {
                other.id.split('.').next() == case.id.split('.').next()
                    && other.offers.contains(&method)
            })
        };
        let id = case.id;
        assert_eq!(
            model_type(provider.language_model("m")),
            "languageModel",
            "{id}"
        );
        assert_eq!(
            model_type(provider.embedding_model("m")),
            "embeddingModel",
            "{id}"
        );
        assert_eq!(
            provider.image_model("m").is_ok(),
            offered("image_model"),
            "{id}"
        );
        if !offered("image_model") {
            assert_eq!(model_type(provider.image_model("m")), "imageModel", "{id}");
        }
        assert_eq!(
            provider.transcription_model("m").is_some(),
            offered("transcription_model"),
            "{id}"
        );
        assert_eq!(
            provider.speech_model("m").is_some(),
            offered("speech_model"),
            "{id}"
        );
        assert_eq!(
            provider.reranking_model("m").is_some(),
            offered("reranking_model"),
            "{id}"
        );
        assert_eq!(
            provider.video_model("m").is_some(),
            offered("video_model"),
            "{id}"
        );
        assert_eq!(
            provider.search_model("m").is_some(),
            offered("search_model"),
            "{id}"
        );
    }
}

#[test]
#[serial]
fn default_instances_read_no_environment_and_never_fail() {
    let _env = clean_env();
    for case in cases() {
        (case.create)(None).expect("creation never needs the environment");
    }
    let _ = serper();
    let _ = prodia();
    let _ = deepgram();
    let _ = you_com();
    let _ = luma();
    let _ = lmnt();
    let _ = klingai();
    let _ = dataforseo();
    let _ = replicate();
    let _ = fal();
    let _ = recraft();
    let _ = aws_polly();
    let _ = jina_ai();
    let _ = gladia();
    let _ = tavily();
    let _ = linkup();
    let _ = tinyfish();
    let _ = black_forest_labs();
    let _ = assemblyai();
    let _ = revai();
    let _ = hume();
    let _ = cartesia();
    let _ = searxng();
    let _ = parallel_ai();
    let _ = firecrawl();
    let _ = runwayml();
    let _ = exa_ai();
    let _ = stability();
    let _ = google_pse();
}

#[test]
fn invalid_base_urls_fail_when_the_provider_is_created() {
    for case in cases() {
        assert!(
            (case.create)(Some("not a url".to_string())).is_err(),
            "{}",
            case.id
        );
    }
}

#[tokio::test]
#[serial]
async fn a_missing_key_fails_the_call_with_the_env_var_name_and_sends_nothing() {
    let _env = clean_env();
    for case in cases() {
        let mock = MockFetch::new(vec![]);
        let mut knobs = Knobs::new(mock.transport());
        if case.id == "searxng.search" {
            knobs.base_url = None;
        }
        let error = (case.call)(knobs).await.unwrap_err();
        match (case.missing, &error) {
            (Missing::ApiKey, AiMuxError::LoadApiKey { env_var, .. })
            | (Missing::Setting, AiMuxError::LoadSetting { env_var, .. }) => {
                assert_eq!(env_var, case.env_var, "{}", case.id);
            }
            _ => panic!("{}: unexpected error {error:?}", case.id),
        }
        assert!(mock.seen().is_empty(), "{}: a request was sent", case.id);
    }
}

#[tokio::test]
#[serial]
async fn the_key_goes_on_the_wire_the_way_the_vendor_wants_it() {
    let _env = clean_env();
    for case in cases() {
        let route = RouteFetch::new(|_| Canned::json(&json!({})));
        let knobs = Knobs::new(route.transport()).with_key("k");
        let _ = (case.call)(knobs).await;
        let seen = route.seen();
        assert!(!seen.is_empty(), "{}: nothing was sent", case.id);
        let first = &seen[0];
        for (name, expected) in case.auth {
            let actual = first.headers.get(*name).unwrap_or_else(|| {
                panic!(
                    "{}: header {name} missing, got {:?}",
                    case.id, first.headers
                )
            });
            match expected.strip_prefix('~') {
                Some(prefix) => {
                    assert!(actual.starts_with(prefix), "{}: {name}: {actual}", case.id)
                }
                None => assert_eq!(actual, expected, "{}: {name}", case.id),
            }
        }
        if case.id == "google_pse.search" {
            assert!(!first.headers.contains_key("authorization"));
            assert!(first.url.contains("key=k"), "{}", first.url);
        }
    }
}

#[tokio::test]
#[serial]
async fn an_empty_key_is_sent_as_given_and_never_falls_back() {
    let _env = clean_env();
    let _decoy = EnvVar::set("TAVILY_API_KEY", Some("from-env"));
    let route = RouteFetch::new(|_| Canned::json(&json!({ "results": [] })));
    let mut knobs = Knobs::new(route.transport());
    knobs.key = Some(Resolvable::Value(String::new()));
    let case = cases()
        .into_iter()
        .find(|c| c.id == "tavily.search")
        .unwrap();
    (case.call)(knobs).await.unwrap();
    assert_eq!(route.seen()[0].headers["authorization"], "Bearer ");
}

#[tokio::test]
#[serial]
async fn the_key_is_evaluated_on_every_request() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "results": [] })));
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = create_tavily(TavilyProviderSettings {
        api_key: Some(Resolvable::from_async_fn({
            let counter = counter.clone();
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(format!("key-{n}")) }
            }
        })),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.search_model();
    model.do_search(&SearchCallOptions::new("a")).await.unwrap();
    model.do_search(&SearchCallOptions::new("b")).await.unwrap();
    let seen = route.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer key-1");
    assert_eq!(seen[1].headers["authorization"], "Bearer key-2");
}

#[tokio::test]
#[serial]
async fn the_environment_is_read_per_request_not_when_the_provider_is_created() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "results": [] })));
    let provider = create_tavily(TavilyProviderSettings {
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.search_model();
    let missing = model
        .do_search(&SearchCallOptions::new("a"))
        .await
        .unwrap_err();
    assert!(matches!(missing, AiMuxError::LoadApiKey { .. }));
    let _key = EnvVar::set("TAVILY_API_KEY", Some("late"));
    model.do_search(&SearchCallOptions::new("b")).await.unwrap();
    assert_eq!(route.seen()[0].headers["authorization"], "Bearer late");
}

// ── headers ──────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn headers_layer_provider_then_call_and_none_removes_the_credential() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "organic": [] })));
    let provider = create_serper(SerperProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        headers: Some(headers(&[
            ("X-Provider", Some("p")),
            ("X-Both", Some("provider")),
        ])),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options = SearchCallOptions::new("q");
    options.headers = Some(
        [("x-both".to_string(), "call".to_string())]
            .into_iter()
            .collect(),
    );
    provider.search_model().do_search(&options).await.unwrap();
    let seen = &route.seen()[0];
    assert_eq!(seen.headers["x-provider"], "p");
    assert_eq!(seen.headers["x-both"], "call");
    assert_eq!(seen.headers["x-api-key"], "k");

    let route = RouteFetch::new(|_| Canned::json(&json!({ "organic": [] })));
    let provider = create_serper(SerperProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        headers: Some(headers(&[("X-API-KEY", None)])),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .search_model()
        .do_search(&SearchCallOptions::new("q"))
        .await
        .unwrap();
    assert!(!route.seen()[0].headers.contains_key("x-api-key"));
}

#[tokio::test]
#[serial]
async fn an_unauthenticated_searxng_sends_no_credential() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "results": [] })));
    let provider = create_searxng(SearxngProviderSettings {
        base_url: Some("http://searx.test/".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    provider
        .search_model()
        .do_search(&SearchCallOptions::new("q"))
        .await
        .unwrap();
    let seen = &route.seen()[0];
    assert!(
        seen.url.starts_with("http://searx.test/search?"),
        "{}",
        seen.url
    );
    assert!(!seen.headers.contains_key("authorization"));
}

#[tokio::test]
#[serial]
async fn searxng_reads_its_instance_url_from_the_environment_per_request() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "results": [] })));
    let provider = create_searxng(SearxngProviderSettings {
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let model = provider.search_model();
    let _url = EnvVar::set("SEARXNG_URL", Some("http://env.searx.test"));
    model.do_search(&SearchCallOptions::new("q")).await.unwrap();
    assert!(
        route.seen()[0]
            .url
            .starts_with("http://env.searx.test/search?")
    );
}

#[tokio::test]
#[serial]
async fn google_pse_resolves_the_search_engine_id_setting_then_options_then_env() {
    let _env = clean_env();
    let run = |settings_cx: Option<&str>, option_cx: Option<&str>| {
        let settings_cx = settings_cx.map(str::to_string);
        let option_cx = option_cx.map(str::to_string);
        async move {
            let route = RouteFetch::new(|_| Canned::json(&json!({ "items": [] })));
            let provider = create_google_pse(GooglePseProviderSettings {
                api_key: Some(Resolvable::Value("k".to_string())),
                cx: settings_cx,
                fetch: Some(route.transport()),
                ..Default::default()
            })
            .unwrap();
            let mut options = SearchCallOptions::new("q");
            if let Some(cx) = option_cx {
                options.provider_options = Some(
                    [("google_pse".to_string(), json!({ "cx": cx }))]
                        .into_iter()
                        .collect(),
                );
            }
            let result = provider.search_model().do_search(&options).await;
            (result, route.seen())
        }
    };
    let (result, seen) = run(Some("setting"), Some("option")).await;
    result.unwrap();
    assert!(seen[0].url.contains("cx=setting"), "{}", seen[0].url);
    let (result, seen) = run(None, Some("option")).await;
    result.unwrap();
    assert!(seen[0].url.contains("cx=option"), "{}", seen[0].url);
    let _cx = EnvVar::set("GOOGLE_CSE_ID", Some("env"));
    let (result, seen) = run(None, None).await;
    result.unwrap();
    assert!(seen[0].url.contains("cx=env"), "{}", seen[0].url);
    drop(_cx);
    let (result, seen) = run(None, None).await;
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
    assert!(seen.is_empty());
}

#[tokio::test]
#[serial]
async fn dataforseo_needs_both_halves_of_the_basic_credential() {
    let _env = clean_env();
    let route = RouteFetch::new(|_| Canned::json(&json!({ "tasks": [] })));
    let provider = create_dataforseo(DataforseoProviderSettings {
        login: Some(Resolvable::Value("user".to_string())),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let error = provider
        .search_model()
        .do_search(&SearchCallOptions::new("q"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiMuxError::LoadApiKey { ref env_var, .. } if env_var == "DATAFORSEO_PASSWORD"),
        "{error:?}"
    );
    assert!(route.seen().is_empty());
}

#[tokio::test]
#[serial]
async fn polly_signs_with_the_credential_chain_and_the_regional_default_endpoint() {
    let _env = clean_env();
    let _id = EnvVar::set("AWS_ACCESS_KEY_ID", Some("AKIDENV"));
    let _secret = EnvVar::set("AWS_SECRET_ACCESS_KEY", Some("secret"));
    let _region = EnvVar::set("AWS_REGION", Some("eu-west-1"));
    let route = RouteFetch::new(|_| Canned::json(&json!({})));
    let provider = create_aws_polly(AwsPollyProviderSettings {
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap();
    let _ = provider
        .speech("neural")
        .do_generate(&SpeechCallOptions::new("hi"))
        .await;
    let seen = &route.seen()[0];
    assert!(
        seen.url
            .starts_with("https://polly.eu-west-1.amazonaws.com/v1/speech"),
        "{}",
        seen.url
    );
    assert!(seen.headers["authorization"].contains("Credential=AKIDENV/"));
    assert!(seen.headers["authorization"].contains("/eu-west-1/polly/aws4_request"));
}
