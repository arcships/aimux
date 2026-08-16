//! Meta Llama models on Vertex AI MaaS — a thin OpenAI-compatible wrapper.
//!
//! Vertex AI serves partner and open models (Anthropic, AI21, DeepSeek, Llama,
//! MiniMax, Mistral, Moonshot, OpenAI, Qwen, Z.AI) through an OpenAI-compatible
//! Chat Completions endpoint — the "Model as a Service" (MaaS) OpenAPI surface
//! — rather than the native `rawPredict` path:
//!
//! `https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi`
//!
//! The host is derived from the location: `global` uses
//! `aiplatform.googleapis.com`, `eu`/`us` use `aiplatform.{loc}.rep.googleapis.com`,
//! and any other location uses `{loc}-aiplatform.googleapis.com`. Authentication
//! uses a Google Cloud OAuth2 Bearer token (the same `GOOGLE_VERTEX_ACCESS_TOKEN`
//! used by the native Vertex provider), sent as `Authorization: Bearer <token>`.
//!
//! Because the endpoint is OpenAI-compatible, this provider is a thin wrapper
//! over [`OpenAIProvider`]: only the base URL,
//! the Bearer-token env var, and the provider name differ. The shared
//! `OpenAIProvider` appends `/chat/completions` to the configured base URL.
//! Sample model ids: `"meta/llama-4-scout-17b-16e-instruct-maas"`, `"meta/llama-4-maverick-17b-128e-instruct-maas"`.

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIModel, OpenAIProvider};

const PROVIDER_NAME: &str = "vertex_ai_llama_models";
const TOKEN_ENV_VAR: &str = "GOOGLE_VERTEX_ACCESS_TOKEN";
const PROJECT_ENV_VAR: &str = "GOOGLE_VERTEX_PROJECT";
const LOCATION_ENV_VAR: &str = "GOOGLE_VERTEX_LOCATION";
const DEFAULT_LOCATION: &str = "global";
/// Fallback project when `GOOGLE_VERTEX_PROJECT` is unset and no base URL is
/// supplied via [`VertexAiLlamaModelsConfig::with_base_url`]; prefer
/// [`VertexAiLlamaModelsConfig::from_env`] or set the project explicitly for real usage.
const DEFAULT_PROJECT: &str = "your-project";

/// Build the Vertex AI MaaS OpenAI-compatible base URL for a project/location.
///
/// - `global` → `https://aiplatform.googleapis.com/v1/projects/{p}/locations/global/endpoints/openapi`
/// - `eu`/`us` → `https://aiplatform.{loc}.rep.googleapis.com/v1/projects/{p}/locations/{loc}/endpoints/openapi`
/// - other → `https://{loc}-aiplatform.googleapis.com/v1/projects/{p}/locations/{loc}/endpoints/openapi`
fn build_maas_base_url(project: &str, location: &str) -> String {
    let host = match location {
        "global" => "aiplatform.googleapis.com".to_string(),
        "eu" | "us" => format!("aiplatform.{location}.rep.googleapis.com"),
        _ => format!("{location}-aiplatform.googleapis.com"),
    };
    format!("https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi")
}

/// Assemble the shared [`OpenAIConfig`] for the given token + project/location.
fn build_config(api_key: String, project: &str, location: &str) -> OpenAIConfig {
    OpenAIConfig::new(api_key)
        .with_base_url(build_maas_base_url(project, location))
        .with_provider(PROVIDER_NAME)
        .with_profile(OpenAICompatProfile::full())
}

/// Configuration for the Meta Llama Vertex AI MaaS provider (wraps [`OpenAIConfig`]).
pub struct VertexAiLlamaModelsConfig(OpenAIConfig);

impl VertexAiLlamaModelsConfig {
    /// Create from a Google Cloud Bearer access token, constructing the base
    /// URL from `GOOGLE_VERTEX_PROJECT` / `GOOGLE_VERTEX_LOCATION` (with
    /// `global` / `your-project` fallbacks). Override the URL with
    /// [`Self::with_base_url`] for tests or proxies.
    pub fn new(api_key: impl Into<String>) -> Self {
        let project =
            std::env::var(PROJECT_ENV_VAR).unwrap_or_else(|_| DEFAULT_PROJECT.to_string());
        let location =
            std::env::var(LOCATION_ENV_VAR).unwrap_or_else(|_| DEFAULT_LOCATION.to_string());
        Self(build_config(api_key.into(), &project, &location))
    }

    /// Create from `GOOGLE_VERTEX_ACCESS_TOKEN` + `GOOGLE_VERTEX_PROJECT` +
    /// `GOOGLE_VERTEX_LOCATION` (location defaults to `global`).
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `GOOGLE_VERTEX_ACCESS_TOKEN` or
    /// `GOOGLE_VERTEX_PROJECT` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let token = std::env::var(TOKEN_ENV_VAR).map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_ACCESS_TOKEN environment variable is required for Vertex AI MaaS"
                    .to_string(),
            )
        })?;
        let project = std::env::var(PROJECT_ENV_VAR).map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_PROJECT environment variable is required for Vertex AI MaaS"
                    .to_string(),
            )
        })?;
        let location =
            std::env::var(LOCATION_ENV_VAR).unwrap_or_else(|_| DEFAULT_LOCATION.to_string());
        Ok(Self(build_config(token, &project, &location)))
    }

    /// Override the base URL (useful for tests / proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }
}

/// Meta Llama Vertex AI MaaS provider — creates [`OpenAIModel`] instances pointed
/// at the Vertex AI MaaS OpenAPI endpoint.
pub struct VertexAiLlamaModelsProvider(OpenAIProvider);

impl VertexAiLlamaModelsProvider {
    #[must_use]
    pub fn new(config: VertexAiLlamaModelsConfig) -> Self {
        Self(OpenAIProvider::new(config.0))
    }

    /// Create a model instance for the given Vertex AI MaaS model id
    /// (e.g. `"meta/llama-4-scout-17b-16e-instruct-maas"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        self.0.model(model_id)
    }
}

impl Provider for VertexAiLlamaModelsProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    crate::delegate_list_models!();
}
