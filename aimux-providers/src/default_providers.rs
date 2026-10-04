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
const PACKAGES: [&str; 16] = [
    "amazon_bedrock",
    "anthropic",
    "anthropic_aws",
    "azure",
    "codex",
    "cohere",
    "deepseek",
    "elevenlabs",
    "google",
    "google_vertex",
    "groq",
    "huggingface",
    "mistral",
    "openai",
    "voyage",
    "xai",
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
    if transform_request_body.is_some() {
        return unsupported("transform_request_body");
    }
    // The packages whose settings carry the same four fields.
    macro_rules! package {
        ($module:ident, $create:ident, $settings:ident) => {
            Ok(Arc::new(crate::$module::$create(
                crate::$module::$settings {
                    api_key,
                    base_url,
                    headers,
                    fetch,
                    ..Default::default()
                },
            )?))
        };
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
        "google_vertex" => {
            if headers.is_some() {
                return unsupported("headers");
            }
            Ok(Arc::new(crate::vertex::create_google_vertex(
                crate::vertex::VertexProviderSettings {
                    api_key,
                    base_url,
                    fetch,
                    ..Default::default()
                },
            )?))
        }
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
                        ..Default::default()
                    },
                )?))
            } else {
                Ok(Arc::new(crate::anthropic_aws::create_anthropic_aws(
                    crate::anthropic_aws::AnthropicAwsProviderSettings {
                        base_url,
                        headers,
                        fetch,
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
