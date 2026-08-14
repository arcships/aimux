//! Provider construction for the console (RFC-0029 §8.2): native single-key
//! providers and the registry-backed OpenAI-compatible family.
//!
//! Adapted from `aimux-cli` `probe::provider::build_model` so the console and
//! the CLI share the same provider construction semantics.

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_providers::provider::ProviderOptions;

/// Build a model from a provider name + model id + optional key / base URL.
///
/// - Native providers (openai/anthropic/google/mistral/xai/cohere) need a key:
///   explicit `api_key` or the provider's standard env var.
/// - Everything else goes through the registry (`provider(name, …)`), where a
///   `None` key reads the provider's registered env var.
pub fn build_model(
    provider: &str,
    api_key: Option<String>,
    model_id: &str,
    base_url: Option<&str>,
) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
    macro_rules! native {
        ($provider_mod:ident, $config:ident, $provider_type:ident, $env:literal) => {{
            let key = native_key(provider, $env, api_key.clone())?;
            let mut cfg = aimux_providers::$provider_mod::$config::new(key);
            if let Some(url) = base_url {
                cfg = cfg.with_base_url(url);
            }
            let p = aimux_providers::$provider_mod::$provider_type::new(cfg);
            Ok(Arc::from(p.model(model_id)))
        }};
    }

    match provider {
        "openai" => native!(openai, OpenAIConfig, OpenAIProvider, "OPENAI_API_KEY"),
        "anthropic" => native!(
            anthropic,
            AnthropicConfig,
            AnthropicProvider,
            "ANTHROPIC_API_KEY"
        ),
        "google" => native!(
            google,
            GoogleConfig,
            GoogleProvider,
            "GOOGLE_GENERATIVE_AI_API_KEY"
        ),
        "mistral" => native!(mistral, MistralConfig, MistralProvider, "MISTRAL_API_KEY"),
        "xai" => native!(xai, XAIConfig, XAIProvider, "XAI_API_KEY"),
        "cohere" => native!(cohere, CohereConfig, CohereProvider, "COHERE_API_KEY"),
        _ => {
            let mut options = ProviderOptions::default();
            if let Some(url) = base_url {
                options.base_url = Some(url.to_string());
            }
            let model = aimux_providers::provider(provider, api_key, model_id, Some(options))
                .map_err(|e| AiMuxError::InvalidArgument(format!("provider '{provider}': {e}")))?;
            Ok(Arc::from(model))
        }
    }
}

/// Resolve the key for a native provider: explicit key, else the standard env var.
fn native_key(provider: &str, env: &str, api_key: Option<String>) -> Result<String, AiMuxError> {
    match api_key {
        Some(k) => Ok(k),
        None => std::env::var(env).map_err(|_| {
            AiMuxError::InvalidArgument(format!(
                "provider '{provider}' needs an API key — set `{env}` or pass api_key=\"env:{env}\""
            ))
        }),
    }
}
