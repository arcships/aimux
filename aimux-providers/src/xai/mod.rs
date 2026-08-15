//! xAI (Grok) provider — a thin wrapper with xAI-specific behaviour.
//!
//! xAI exposes an OpenAI-compatible Chat Completions API at
//! `https://api.x.ai/v1`. While the wire format is OpenAI-compatible, xAI has
//! enough provider-specific behaviour (reasoning content extraction, citations,
//! search parameters, xai-keyed provider options, non-inclusive cached tokens,
//! reasoning-effort model gating, 200-status errors) to warrant its own model
//! implementation ([`XaiModel`]) rather than reusing `OpenAIModel`.

pub mod convert;
mod model;
pub mod responses;
mod types;

pub use model::XaiModel;
pub use responses::XaiResponsesModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{RetryConfig, load_api_key};

use crate::openai::OpenAIConfig;

const DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
const ENV_VAR: &str = "XAI_API_KEY";
const PROVIDER_NAME: &str = "xai";

/// Configuration for the xAI provider (wraps [`OpenAIConfig`]).
#[derive(Clone)]
pub struct XAIConfig(OpenAIConfig);

impl XAIConfig {
    /// Create from an API key, using the default xAI base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self(OpenAIConfig::new(api_key).with_base_url(DEFAULT_BASE_URL))
    }

    /// Create from the `XAI_API_KEY` environment variable.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when `XAI_API_KEY` is not set.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let key = load_api_key(None, ENV_VAR, "xAI")?;
        Ok(Self::new(key).with_api_key_source(Some("env:XAI_API_KEY")))
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

    /// Override the base URL (useful for tests / self-hosted endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.0 = self.0.with_base_url(url);
        self
    }

    /// Get the API key.
    pub(crate) fn api_key(&self) -> &str {
        &self.0.api_key
    }

    /// Get the base URL.
    pub(crate) fn base_url(&self) -> &str {
        &self.0.base_url
    }

    /// Get the retry config.
    pub(crate) fn retry_config(&self) -> RetryConfig {
        self.0.retry_config
    }
}

/// xAI provider — creates [`XaiModel`] instances pointed at xAI.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct XAIProvider {
    config: XAIConfig,
}

impl XAIProvider {
    #[must_use]
    pub fn new(config: XAIConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given xAI model id (e.g. `"grok-2"`).
    ///
    /// Clones the provider config so the model inherits the same
    /// `api_key_source` / `retry_config` (M2b: previously reconstructed with
    /// `XAIConfig::new`, which dropped the credential source).
    #[must_use]
    pub fn model(&self, model_id: &str) -> XaiModel {
        XaiModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Responses API model instance for the given xAI model id.
    ///
    /// Uses the xAI `/responses` endpoint with the Responses API wire format
    /// (input items, reasoning objects, provider-executed tools, etc.).
    #[must_use]
    pub fn responses_model(&self, model_id: &str) -> XaiResponsesModel {
        XaiResponsesModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for XAIProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List models via `GET {base_url}/models` (OpenAI-compatible, RFC-0027).
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
        let config = OpenAIConfig::new(self.config.api_key())
            .with_base_url(self.config.base_url())
            .with_provider(PROVIDER_NAME);
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
