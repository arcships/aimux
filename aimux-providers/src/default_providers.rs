//! The built-in providers.
//!
//! The AI SDK ships no vendor list: a caller creates providers with the
//! package factories and hands a map of them to `createProviderRegistry`.
//! aimux knows its vendors, so it does that assembly once, the way a
//! models.dev consumer does: a name selects a factory (a vendor package, or
//! the OpenAI-compatible factory with a row of `provider_registry.json`) and
//! the same few settings go to whichever factory it is.
//!
//! - [`create_provider`] is that one step for one name.
//! - [`default_providers`] is every name with default settings, ready for
//!   [`aimux_core::create_provider_registry`].

use std::collections::BTreeMap;
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;

use crate::preset::{self, PresetSettings};

/// The vendor packages, by the name of their default instance.
const PACKAGES: [&str; 45] = [
    "amazon_bedrock",
    "anthropic",
    "anthropic_aws",
    "assemblyai",
    "aws_polly",
    "azure",
    "black_forest_labs",
    "cartesia",
    "codex",
    "cohere",
    "dataforseo",
    "deepgram",
    "deepseek",
    "elevenlabs",
    "exa_ai",
    "fal",
    "firecrawl",
    "gladia",
    "google",
    "google_pse",
    "google_vertex",
    "groq",
    "huggingface",
    "hume",
    "jina_ai",
    "klingai",
    "linkup",
    "lmnt",
    "luma",
    "mistral",
    "openai",
    "parallel_ai",
    "prodia",
    "recraft",
    "replicate",
    "revai",
    "runwayml",
    "searxng",
    "serper",
    "stability",
    "tavily",
    "tinyfish",
    "voyage",
    "xai",
    "you_com",
];

/// The names [`create_provider`] accepts: the vendor packages and every
/// registry row, sorted.
#[must_use]
pub fn provider_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PACKAGES.to_vec();
    names.extend(preset::names().filter(|name| !PACKAGES.contains(name)));
    names.sort_unstable();
    names
}

/// Create the built-in provider `name` with `settings` (key, base URL, extra
/// headers, transport) applied to its factory. A vendor package wins over a
/// registry row of the same name.
///
/// # Errors
///
/// [`AiMuxError::NoSuchProvider`] for an unknown name (there is no fallback),
/// `InvalidArgument` for a setting the selected factory does not have, and
/// whatever the factory itself reports.
pub fn create_provider(
    name: &str,
    settings: PresetSettings,
) -> Result<Arc<dyn Provider>, AiMuxError> {
    if !PACKAGES.contains(&name) {
        return Ok(Arc::new(preset::create(name, settings)?));
    }
    let PresetSettings {
        api_key,
        base_url,
        headers,
        fetch,
        params,
        transform_request_body,
    } = settings;
    let unsupported = |field: &str| {
        Err(AiMuxError::InvalidArgument(format!(
            "provider '{name}' has no `{field}` setting"
        )))
    };
    if !params.is_empty() {
        return unsupported("params");
    }
    // The packages whose settings also support a body transform.
    macro_rules! package {
        ($module:ident, $create:ident, $settings:ident) => {
            Ok(Arc::new(crate::$module::$create(
                crate::$module::$settings {
                    api_key,
                    base_url,
                    headers,
                    fetch,
                    transform_request_body,
                    ..Default::default()
                },
            )?))
        };
    }
    macro_rules! package_without_transform {
        ($module:ident, $create:ident, $settings:ident $(, $api_key:ident)?) => {{
            if transform_request_body.is_some() {
                return unsupported("transform_request_body");
            }
            Ok(Arc::new(crate::$module::$create(
                crate::$module::$settings {
                    $($api_key,)?
                    base_url,
                    headers,
                    fetch,
                    ..Default::default()
                },
            )?))
        }};
    }
    match name {
        "amazon_bedrock" => package!(
            bedrock,
            create_amazon_bedrock,
            AmazonBedrockProviderSettings
        ),
        "anthropic" => package!(anthropic, create_anthropic, AnthropicProviderSettings),
        "azure" => package!(azure, create_azure, AzureOpenAIProviderSettings),
        "cohere" => package!(cohere, create_cohere, CohereProviderSettings),
        "deepseek" => package!(deepseek, create_deepseek, DeepSeekProviderSettings),
        "elevenlabs" => package!(elevenlabs, create_elevenlabs, ElevenLabsProviderSettings),
        "google" => package!(google, create_google, GoogleProviderSettings),
        "groq" => package!(groq, create_groq, GroqProviderSettings),
        "huggingface" => package!(huggingface, create_huggingface, HuggingFaceProviderSettings),
        "mistral" => package!(mistral, create_mistral, MistralProviderSettings),
        "openai" => package!(openai, create_openai, OpenAIProviderSettings),
        "voyage" => package!(voyage, create_voyage, VoyageProviderSettings),
        "xai" => package!(xai, create_xai, XAIProviderSettings),
        "assemblyai" => package_without_transform!(
            assemblyai,
            create_assemblyai,
            AssemblyAIProviderSettings,
            api_key
        ),
        "black_forest_labs" => package_without_transform!(
            black_forest_labs,
            create_black_forest_labs,
            BlackForestLabsProviderSettings,
            api_key
        ),
        "cartesia" => {
            package_without_transform!(cartesia, create_cartesia, CartesiaProviderSettings, api_key)
        }
        "deepgram" => {
            package_without_transform!(deepgram, create_deepgram, DeepgramProviderSettings, api_key)
        }
        "exa_ai" => {
            package_without_transform!(exa_ai, create_exa_ai, ExaAiProviderSettings, api_key)
        }
        "fal" => package_without_transform!(fal, create_fal, FalProviderSettings, api_key),
        "firecrawl" => package_without_transform!(
            firecrawl,
            create_firecrawl,
            FirecrawlProviderSettings,
            api_key
        ),
        "gladia" => {
            package_without_transform!(gladia, create_gladia, GladiaProviderSettings, api_key)
        }
        "google_pse" => package_without_transform!(
            google_pse,
            create_google_pse,
            GooglePseProviderSettings,
            api_key
        ),
        "hume" => package_without_transform!(hume, create_hume, HumeProviderSettings, api_key),
        "jina_ai" => {
            package_without_transform!(jina_ai, create_jina_ai, JinaAiProviderSettings, api_key)
        }
        "klingai" => {
            package_without_transform!(klingai, create_klingai, KlingAIProviderSettings, api_key)
        }
        "linkup" => {
            package_without_transform!(linkup, create_linkup, LinkupProviderSettings, api_key)
        }
        "lmnt" => package_without_transform!(lmnt, create_lmnt, LMNTProviderSettings, api_key),
        "luma" => package_without_transform!(luma, create_luma, LumaProviderSettings, api_key),
        "parallel_ai" => package_without_transform!(
            parallel_ai,
            create_parallel_ai,
            ParallelAiProviderSettings,
            api_key
        ),
        "prodia" => {
            package_without_transform!(prodia, create_prodia, ProdiaProviderSettings, api_key)
        }
        "recraft" => {
            package_without_transform!(recraft, create_recraft, RecraftProviderSettings, api_key)
        }
        "replicate" => package_without_transform!(
            replicate,
            create_replicate,
            ReplicateProviderSettings,
            api_key
        ),
        "revai" => package_without_transform!(revai, create_revai, RevaiProviderSettings, api_key),
        "runwayml" => {
            package_without_transform!(runwayml, create_runwayml, RunwaymlProviderSettings, api_key)
        }
        "searxng" => {
            package_without_transform!(searxng, create_searxng, SearxngProviderSettings, api_key)
        }
        "serper" => {
            package_without_transform!(serper, create_serper, SerperProviderSettings, api_key)
        }
        "stability" => package_without_transform!(
            stability,
            create_stability,
            StabilityProviderSettings,
            api_key
        ),
        "tavily" => {
            package_without_transform!(tavily, create_tavily, TavilyProviderSettings, api_key)
        }
        "tinyfish" => {
            package_without_transform!(tinyfish, create_tinyfish, TinyfishProviderSettings, api_key)
        }
        "you_com" => {
            package_without_transform!(you_com, create_you_com, YouComProviderSettings, api_key)
        }
        "aws_polly" => {
            if api_key.is_some() {
                return unsupported("api_key");
            }
            package_without_transform!(aws_polly, create_aws_polly, AwsPollyProviderSettings)
        }
        "dataforseo" => {
            if api_key.is_some() {
                return unsupported("api_key");
            }
            package_without_transform!(dataforseo, create_dataforseo, DataforseoProviderSettings)
        }
        "google_vertex" => Ok(Arc::new(crate::vertex::create_google_vertex(
            crate::vertex::VertexProviderSettings {
                api_key,
                base_url,
                headers: headers.map(Into::into),
                fetch,
                transform_request_body,
                ..Default::default()
            },
        )?)),
        "anthropic_aws" | "codex" => {
            if api_key.is_some() {
                return unsupported("api_key");
            }
            if name == "codex" {
                Ok(Arc::new(crate::codex::create_codex(
                    crate::codex::CodexProviderSettings {
                        base_url,
                        headers,
                        fetch,
                        transform_request_body,
                        ..Default::default()
                    },
                )?))
            } else {
                Ok(Arc::new(crate::anthropic_aws::create_anthropic_aws(
                    crate::anthropic_aws::AnthropicAwsProviderSettings {
                        base_url,
                        headers,
                        fetch,
                        transform_request_body,
                        ..Default::default()
                    },
                )?))
            }
        }
        _ => unreachable!("every name in PACKAGES has an arm"),
    }
}

/// Every built-in provider with default settings, keyed by name: pass it to
/// [`aimux_core::create_provider_registry`], after adding or replacing entries
/// as needed.
///
/// Nothing is read from the environment here and no entry can fail to be
/// created: keys and environment-sourced base URLs are evaluated per request,
/// so a missing key fails the call that needs it.
#[must_use]
pub fn default_providers() -> BTreeMap<String, Arc<dyn Provider>> {
    provider_names()
        .into_iter()
        .map(|name| {
            let provider = create_provider(name, PresetSettings::default())
                .expect("a built-in provider with default settings is always valid");
            (name.to_string(), provider)
        })
        .collect()
}
