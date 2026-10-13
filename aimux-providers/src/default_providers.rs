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
        // Checked here, not left to `preset::create`: its error lists only the
        // registry rows, and a misspelled package name must list the packages.
        if preset::lookup(name).is_none() {
            return Err(AiMuxError::NoSuchProvider {
                provider_id: name.to_string(),
                model_id: name.to_string(),
                model_type: String::new(),
                available_providers: provider_names().into_iter().map(str::to_owned).collect(),
            });
        }
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
                #[allow(clippy::needless_update)]
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
    macro_rules! package_with_string_key {
        ($module:ident, $create:ident, $settings:ident $(, $defaults:expr)?) => {{
            if transform_request_body.is_some() {
                return unsupported("transform_request_body");
            }
            let api_key = match api_key {
                Some(aimux_provider_utils::Resolvable::Value(key)) => Some(key),
                None => None,
                Some(_) => return unsupported("resolvable api_key"),
            };
            Ok(Arc::new(crate::$module::$create(
                #[allow(clippy::needless_update)]
                crate::$module::$settings {
                    api_key,
                    base_url,
                    headers,
                    fetch,
                    $(..$defaults)?
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
        "anthropic" => {
            package_with_string_key!(
                anthropic,
                create_anthropic,
                AnthropicProviderSettings,
                Default::default()
            )
        }
        "azure" => {
            package_without_transform!(azure, create_azure, AzureOpenAIProviderSettings, api_key)
        }
        "cohere" => package_with_string_key!(
            cohere,
            create_cohere,
            CohereProviderSettings,
            Default::default()
        ),
        "deepseek" => package_with_string_key!(deepseek, create_deepseek, DeepSeekProviderSettings),
        "elevenlabs" => package!(elevenlabs, create_elevenlabs, ElevenLabsProviderSettings),
        "google" => package_with_string_key!(
            google,
            create_google,
            GoogleProviderSettings,
            Default::default()
        ),
        "groq" => package_with_string_key!(groq, create_groq, GroqProviderSettings),
        "huggingface" => package!(huggingface, create_huggingface, HuggingFaceProviderSettings),
        "mistral" => {
            package_with_string_key!(
                mistral,
                create_mistral,
                MistralProviderSettings,
                Default::default()
            )
        }
        "openai" => {
            package_with_string_key!(
                openai,
                create_openai,
                OpenAIProviderSettings,
                Default::default()
            )
        }
        "voyage" => package!(voyage, create_voyage, VoyageProviderSettings),
        "xai" => {
            package_with_string_key!(xai, create_xai, XAIProviderSettings, Default::default())
        }
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

/// The environment variable `name`'s API key is read from when
/// [`create_provider`] gets no `api_key`: the vendor package's key variable,
/// or the registry row's `env_var`. `None` for an unknown name, for a
/// registry row without a variable, and for the packages whose `api_key`
/// setting has no variable (`searxng`) or that take no `api_key` at all
/// (`anthropic_aws`, `aws_polly`, `codex`, `dataforseo`).
///
/// Hosts that let a caller pick a key by variable name (the aimux-web
/// console's `env:VAR`) use it to allow only the provider's own variable.
#[must_use]
pub fn provider_api_key_env(name: &str) -> Option<&'static str> {
    use crate::{
        anthropic, assemblyai, azure, bedrock, black_forest_labs, cartesia, cohere, deepgram,
        deepseek, elevenlabs, exa_ai, fal, firecrawl, gladia, google, google_pse, groq,
        huggingface, hume, jina_ai, klingai, linkup, lmnt, luma, mistral, openai, parallel_ai,
        prodia, recraft, replicate, revai, runwayml, serper, stability, tavily, tinyfish, vertex,
        voyage, xai, you_com,
    };
    Some(match name {
        "amazon_bedrock" => bedrock::BEARER_TOKEN_ENV_VAR,
        "anthropic" => anthropic::API_KEY_ENV_VAR,
        "assemblyai" => assemblyai::API_KEY_ENV_VAR,
        "azure" => azure::API_KEY_ENV_VAR,
        "black_forest_labs" => black_forest_labs::API_KEY_ENV_VAR,
        "cartesia" => cartesia::API_KEY_ENV_VAR,
        "cohere" => cohere::API_KEY_ENV_VAR,
        "deepgram" => deepgram::API_KEY_ENV_VAR,
        "deepseek" => deepseek::API_KEY_ENV_VAR,
        "elevenlabs" => elevenlabs::API_KEY_ENV_VAR,
        "exa_ai" => exa_ai::API_KEY_ENV_VAR,
        "fal" => fal::API_KEY_ENV_VAR,
        "firecrawl" => firecrawl::API_KEY_ENV_VAR,
        "gladia" => gladia::API_KEY_ENV_VAR,
        "google" => google::API_KEY_ENV_VAR,
        "google_pse" => google_pse::API_KEY_ENV_VAR,
        "google_vertex" => vertex::API_KEY_ENV_VAR,
        "groq" => groq::API_KEY_ENV_VAR,
        "huggingface" => huggingface::API_KEY_ENV_VAR,
        "hume" => hume::API_KEY_ENV_VAR,
        "jina_ai" => jina_ai::API_KEY_ENV_VAR,
        "klingai" => klingai::API_KEY_ENV_VAR,
        "linkup" => linkup::API_KEY_ENV_VAR,
        "lmnt" => lmnt::API_KEY_ENV_VAR,
        "luma" => luma::API_KEY_ENV_VAR,
        "mistral" => mistral::API_KEY_ENV_VAR,
        "openai" => openai::API_KEY_ENV_VAR,
        "parallel_ai" => parallel_ai::API_KEY_ENV_VAR,
        "prodia" => prodia::API_KEY_ENV_VAR,
        "recraft" => recraft::API_KEY_ENV_VAR,
        "replicate" => replicate::API_KEY_ENV_VAR,
        "revai" => revai::API_KEY_ENV_VAR,
        "runwayml" => runwayml::API_KEY_ENV_VAR,
        "serper" => serper::API_KEY_ENV_VAR,
        "stability" => stability::API_KEY_ENV_VAR,
        "tavily" => tavily::API_KEY_ENV_VAR,
        "tinyfish" => tinyfish::API_KEY_ENV_VAR,
        "voyage" => voyage::API_KEY_ENV_VAR,
        "xai" => xai::API_KEY_ENV_VAR,
        "you_com" => you_com::API_KEY_ENV_VAR,
        _ if PACKAGES.contains(&name) => return None,
        _ => preset::lookup(name)
            .map(|entry| entry.descriptor.env_var.as_str())
            .filter(|var| !var.is_empty())?,
    })
}

/// The endpoint `name`'s factory calls when [`create_provider`] gets no
/// `base_url` and no base-URL variable is set: the vendor package's default
/// base URL, or the registry row's `base_url`. `None` for an unknown name and
/// where the endpoint is not one fixed URL: built from settings (a region,
/// resource, project or a registry row's template parameters) or read only
/// from the environment.
///
/// Hosts that let a caller choose the base URL (the aimux-web console) use it
/// to tell the provider's own endpoint from one the caller picked.
#[must_use]
pub fn provider_default_base_url(name: &str) -> Option<&'static str> {
    use crate::{
        anthropic, assemblyai, black_forest_labs, cartesia, cohere, dataforseo, deepgram, deepseek,
        elevenlabs, exa_ai, fal, firecrawl, gladia, google, google_pse, groq, huggingface, hume,
        jina_ai, klingai, linkup, lmnt, luma, mistral, openai, parallel_ai, prodia, recraft,
        replicate, revai, runwayml, serper, stability, tavily, tinyfish, voyage, xai, you_com,
    };
    Some(match name {
        "anthropic" => anthropic::DEFAULT_BASE_URL,
        "assemblyai" => assemblyai::DEFAULT_BASE_URL,
        "black_forest_labs" => black_forest_labs::DEFAULT_BASE_URL,
        "cartesia" => cartesia::DEFAULT_BASE_URL,
        "cohere" => cohere::DEFAULT_BASE_URL,
        "dataforseo" => dataforseo::DEFAULT_BASE_URL,
        "deepgram" => deepgram::DEFAULT_BASE_URL,
        "deepseek" => deepseek::DEFAULT_BASE_URL,
        "elevenlabs" => elevenlabs::DEFAULT_BASE_URL,
        "exa_ai" => exa_ai::DEFAULT_BASE_URL,
        "fal" => fal::DEFAULT_BASE_URL,
        "firecrawl" => firecrawl::DEFAULT_BASE_URL,
        "gladia" => gladia::DEFAULT_BASE_URL,
        "google" => google::DEFAULT_BASE_URL,
        "google_pse" => google_pse::DEFAULT_BASE_URL,
        "groq" => groq::DEFAULT_BASE_URL,
        "huggingface" => huggingface::DEFAULT_BASE_URL,
        "hume" => hume::DEFAULT_BASE_URL,
        "jina_ai" => jina_ai::DEFAULT_BASE_URL,
        "klingai" => klingai::DEFAULT_BASE_URL,
        "linkup" => linkup::DEFAULT_BASE_URL,
        "lmnt" => lmnt::DEFAULT_BASE_URL,
        "luma" => luma::DEFAULT_BASE_URL,
        "mistral" => mistral::DEFAULT_BASE_URL,
        "openai" => openai::DEFAULT_BASE_URL,
        "parallel_ai" => parallel_ai::DEFAULT_BASE_URL,
        "prodia" => prodia::DEFAULT_BASE_URL,
        "recraft" => recraft::DEFAULT_BASE_URL,
        "replicate" => replicate::DEFAULT_BASE_URL,
        "revai" => revai::DEFAULT_BASE_URL,
        "runwayml" => runwayml::DEFAULT_BASE_URL,
        "serper" => serper::DEFAULT_BASE_URL,
        "stability" => stability::DEFAULT_BASE_URL,
        "tavily" => tavily::DEFAULT_BASE_URL,
        "tinyfish" => tinyfish::DEFAULT_BASE_URL,
        "voyage" => voyage::DEFAULT_BASE_URL,
        "xai" => xai::DEFAULT_BASE_URL,
        "you_com" => you_com::DEFAULT_BASE_URL,
        // amazon_bedrock, anthropic_aws, aws_polly, azure, codex,
        // google_vertex and searxng.
        _ if PACKAGES.contains(&name) => return None,
        _ => preset::lookup(name)
            .filter(|entry| entry.descriptor.params.is_empty())
            .map(|entry| entry.descriptor.base_url.as_str())?,
    })
}

/// Every built-in provider with default settings, keyed by name: pass it to
/// [`aimux_core::create_provider_registry`], after adding or replacing entries
/// as needed.
///
/// No entry can fail to be created. Keys are read per request, so a missing
/// key fails the call that needs it. The `openai` and `anthropic` packages
/// read `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` once, here, as the AI SDK
/// does when a provider is created; an invalid value fails each call of that
/// provider with `InvalidArgument` naming the variable, not this function.
/// Registry rows read their base-URL variables per request.
#[must_use]
pub fn default_providers() -> BTreeMap<String, Arc<dyn Provider>> {
    provider_names()
        .into_iter()
        .map(|name| {
            let provider = create_provider(name, PresetSettings::default())
                .expect("default settings have nothing a factory can reject");
            (name.to_string(), provider)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sets an environment variable for the test's duration and restores
    /// the previous value on drop.
    struct EnvVar(&'static str, Option<std::ffi::OsString>);

    impl EnvVar {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(name);
            // SAFETY: the tests touching these variables are `#[serial]`.
            unsafe { std::env::set_var(name, value) };
            Self(name, previous)
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            // SAFETY: as in `set`.
            unsafe {
                match &self.1 {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
        }
    }

    /// An invalid `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` does not fail
    /// creating the default providers. Each call of that provider reports it
    /// instead, as the AI SDK's factories read the variable without failing
    /// (openai@4.0.83 `openai-provider.ts:207-215`, anthropic@4.0.71
    /// `anthropic-provider.ts:127-133`) and the first request rejects the URL.
    #[tokio::test]
    #[serial_test::serial]
    async fn an_invalid_env_base_url_fails_each_call_not_creation() {
        let _openai = EnvVar::set("OPENAI_BASE_URL", "not a url");
        let _anthropic = EnvVar::set("ANTHROPIC_BASE_URL", "not a url");
        let _openai_key = EnvVar::set("OPENAI_API_KEY", "test-key");
        let _anthropic_key = EnvVar::set("ANTHROPIC_API_KEY", "test-key");
        let providers = default_providers();
        for (name, var) in [
            ("openai", "OPENAI_BASE_URL"),
            ("anthropic", "ANTHROPIC_BASE_URL"),
        ] {
            let model = providers[name].language_model("some-model").unwrap();
            let options = aimux_core::options::CallOptions::new(vec![
                aimux_core::language_model_message::LanguageModelMessage::user_text("hi"),
            ]);
            let error = model.do_generate(&options).await.expect_err(name);
            assert!(
                matches!(&error, AiMuxError::InvalidArgument(message) if message.starts_with(var)),
                "{name}: {error:?}"
            );
        }
        let _ = create_provider("openai", PresetSettings::default()).unwrap();
    }

    /// A misspelled name lists every name `create_provider` accepts, the
    /// vendor packages included, not only the registry rows.
    #[test]
    fn an_unknown_name_lists_every_built_in_provider() {
        let Err(AiMuxError::NoSuchProvider {
            provider_id,
            available_providers,
            ..
        }) = create_provider("opeanai", PresetSettings::default())
        else {
            panic!("expected NoSuchProvider");
        };
        assert_eq!(provider_id, "opeanai");
        assert_eq!(available_providers, provider_names());
        assert!(available_providers.iter().any(|name| name == "openai"));
    }

    /// The key variable is the one each factory reads with no `api_key`:
    /// the package's own constant, else the registry row's `env_var`.
    #[test]
    fn provider_api_key_env_names_the_variable_the_factory_reads() {
        assert_eq!(provider_api_key_env("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(provider_api_key_env("groq"), Some("GROQ_API_KEY"));
        assert_eq!(
            provider_api_key_env("google"),
            Some("GOOGLE_GENERATIVE_AI_API_KEY")
        );
        assert_eq!(
            provider_api_key_env("amazon_bedrock"),
            Some("AWS_BEARER_TOKEN_BEDROCK")
        );
        assert_eq!(provider_api_key_env("fal"), Some("FAL_KEY"));
        // Registry rows.
        assert_eq!(
            provider_api_key_env("openrouter"),
            Some("OPENROUTER_API_KEY")
        );
        assert_eq!(provider_api_key_env("ollama"), Some("OLLAMA_API_KEY"));
        // Every package that takes a key names its variable; the rest none.
        let keyless = [
            "anthropic_aws",
            "aws_polly",
            "codex",
            "dataforseo",
            "searxng",
        ];
        for name in PACKAGES {
            assert_eq!(
                provider_api_key_env(name).is_some(),
                !keyless.contains(&name),
                "{name}"
            );
        }
        // A registry row without a variable, and an unknown name.
        let without = preset::entries()
            .find(|entry| entry.descriptor.env_var.is_empty())
            .map(|entry| entry.descriptor.name.as_str());
        if let Some(name) = without {
            assert_eq!(provider_api_key_env(name), None, "{name}");
        }
        assert_eq!(provider_api_key_env("no-such-provider"), None);
    }

    /// The default endpoint is the package constant or the registry row's
    /// URL; a URL built from settings has none.
    #[test]
    fn provider_default_base_url_is_the_factorys_fixed_endpoint() {
        assert_eq!(
            provider_default_base_url("openai"),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            provider_default_base_url("groq"),
            Some("https://api.groq.com/openai/v1")
        );
        assert_eq!(
            provider_default_base_url("openrouter"),
            Some("https://openrouter.ai/api/v1")
        );
        for name in [
            "amazon_bedrock",
            "anthropic_aws",
            "aws_polly",
            "azure",
            "codex",
            "google_vertex",
            "searxng",
            // A registry row with template parameters.
            "bedrock_mantle",
            "no-such-provider",
        ] {
            assert_eq!(provider_default_base_url(name), None, "{name}");
        }
    }

    /// Providers and their models report the V4 specification version, as
    /// the AI SDK's `ProviderV4` and model V4 interfaces do: a vendor package
    /// and a registry row from `create_provider`, and the same providers
    /// reached through a provider registry.
    #[test]
    fn providers_and_models_report_specification_version_v4() {
        for (name, model_id) in [("openai", "gpt-4o"), ("fireworks", "some-model")] {
            let provider = create_provider(name, PresetSettings::default()).unwrap();
            assert_eq!(provider.specification_version(), "v4", "{name}");
            let model = provider.language_model(model_id).unwrap();
            assert_eq!(model.specification_version(), "v4", "{name}");
        }
        let openai = create_provider("openai", PresetSettings::default()).unwrap();
        let files = openai.files().expect("openai offers files");
        assert_eq!(files.specification_version(), "v4");

        let registry = aimux_core::create_provider_registry(
            default_providers(),
            aimux_core::ProviderRegistryOptions::default(),
        );
        assert_eq!(
            registry.provider("openai").unwrap().specification_version(),
            "v4"
        );
        let model = registry.language_model("openai:gpt-4o").unwrap();
        assert_eq!(model.specification_version(), "v4");
    }
}
