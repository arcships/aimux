//! Hugging Face provider — a thin OpenAI-compatible wrapper.
//!
//! Hugging Face exposes an OpenAI-compatible Chat Completions API through its
//! router at `https://router.huggingface.co/v1`. The TS SDK configures this base
//! URL and the `HUGGINGFACE_API_KEY` environment variable. The Rust
//! [`OpenAIProvider`](crate::openai::OpenAIProvider) appends `/chat/completions`
//! to the configured base URL, yielding
//! `https://router.huggingface.co/v1/chat/completions`. Everything else is
//! delegated to the shared `OpenAIProvider`.
//!
//! In addition to the Chat Completions API, Hugging Face also exposes a
//! Responses API (the lightest Responses implementation — function tools only,
//! no built-in tools). See [`responses::HuggingFaceResponsesModel`].

pub mod responses;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::load_api_key;

use crate::openai::{OpenAIConfig, OpenAIModel};

const DEFAULT_BASE_URL: &str = "https://router.huggingface.co/v1";
const ENV_VAR: &str = "HUGGINGFACE_API_KEY";
const PROVIDER_NAME: &str = "huggingface";

/// Configuration for the Hugging Face provider (wraps [`OpenAIConfig`]).
#[derive(Debug, Clone)]
pub struct HuggingFaceConfig(OpenAIConfig);

impl HuggingFaceConfig {
    /// Create from an API key, using the default Hugging Face base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }

    /// Create from the `HUGGINGFACE_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `HUGGINGFACE_API_KEY` is not
    /// set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "Hugging Face")?;
        Ok(Self::new(key).with_api_key_source(Some("env:HUGGINGFACE_API_KEY")))
    }

    /// Override the base URL (useful for tests / self-hosted endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。透传到内部 `OpenAIConfig`。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.0 = self.0.with_api_key_source(source);
        self
    }

    /// 内部 `OpenAIConfig` 引用(config_snapshot 复用 OpenAI helper 用,M2b)。
    pub(crate) fn openai_config(&self) -> &OpenAIConfig {
        &self.0
    }
}

/// Hugging Face provider — creates [`OpenAIModel`] (chat) and
/// [`responses::HuggingFaceResponsesModel`] (responses) instances pointed at HF.
pub struct HuggingFaceProvider {
    config: HuggingFaceConfig,
}

impl HuggingFaceProvider {
    #[must_use]
    pub fn new(config: HuggingFaceConfig) -> Self {
        Self { config }
    }

    /// Create a chat model instance for the given Hugging Face model id
    /// (e.g. `"meta-llama/Llama-3.3-70B-Instruct"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        OpenAIModel::new(model_id.to_string(), self.config.0.clone())
    }

    /// Create a Responses model instance for the given Hugging Face model id.
    ///
    /// The Hugging Face Responses API is the lightest Responses implementation:
    /// it supports function tools only (no built-in tools), and uses the
    /// `text.format` field for structured output.
    #[must_use]
    pub fn responses_model(&self, model_id: &str) -> responses::HuggingFaceResponsesModel {
        responses::HuggingFaceResponsesModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for HuggingFaceProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        // HuggingFaceProvider holds an OpenAIConfig (not an OpenAIProvider
        // directly), so delegate via execute_list_models + catalogue resolve.
        let config = self.config.0.clone();
        Box::pin(async move {
            let headers = crate::openai::model::build_auth_headers(&config);
            let runtime = crate::openai::model::execute_list_models(
                &config.base_url,
                &headers,
                &config.retry_config,
            )
            .await?;
            Ok(runtime)
        })
    }
}
