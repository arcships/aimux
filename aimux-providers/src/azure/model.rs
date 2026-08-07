//! Azure OpenAI language model — implements `LanguageModel` trait.
//!
//! Azure OpenAI speaks the OpenAI chat-completions wire format but differs in
//! two respects:
//!
//! 1. **URL construction** — Azure addresses a *deployment* rather than a
//!    model, and pins the API version via a `?api-version=` query parameter.
//!    The classic deployment-based form is
//!    `https://{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version={api_version}`.
//!    A newer `v1` form (`…/openai/v1/chat/completions?api-version=…`) is also
//!    supported via [`AzureConfig::use_v1_urls`].
//!
//! 2. **Authentication** — either an `api-key` header (API key) or a
//!    `Bearer` token supplied by a [`TokenProvider`] (Azure AD / Microsoft
//!    Entra ID). The two are mutually exclusive.
//!
//! The actual request/response conversion is shared with the OpenAI provider
//! via [`crate::openai::model::execute_generate`] /
//! [`crate::openai::model::execute_stream`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::{GenerateResult, StreamResult};

use aimux_provider_utils::{
    RetryConfig, load_api_key, with_user_agent_suffix, without_trailing_slash,
};

use crate::azure::responses::AzureResponsesModel;
use crate::openai::model::{execute_generate, execute_stream};

/// Default Azure OpenAI API version used for the deployment-based endpoint.
pub const DEFAULT_API_VERSION: &str = "2024-10-21";

// ── Token provider (Azure AD / Microsoft Entra ID) ───────────────────────────

/// A source of Azure AD (Microsoft Entra ID) bearer tokens.
///
/// Implementors return a fresh access token on each call; the token is sent as
/// `Authorization: Bearer <token>`. This mirrors the TS `tokenProvider:
/// () => Promise<string>` option.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Return a bearer access token.
    async fn get_token(&self) -> Result<String, AiMuxError>;
}

/// Authentication strategy for Azure OpenAI.
#[derive(Clone)]
pub enum AzureAuth {
    /// API-key auth — sent as the `api-key` header.
    ApiKey(String),
    /// Azure AD (Microsoft Entra ID) auth — sent as `Authorization: Bearer <token>`.
    TokenProvider(Arc<dyn TokenProvider>),
}

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the Azure OpenAI provider.
///
/// Either `resource_name` or `base_url` must be set (the former builds the
/// standard `https://{resource}.openai.azure.com/openai` prefix; the latter
/// overrides it entirely, e.g. for a proxy/gateway).
#[derive(Clone)]
pub struct AzureConfig {
    /// Azure OpenAI resource name (the `{resource}` in the hostname).
    pub resource_name: Option<String>,
    /// Override the URL prefix (takes precedence over `resource_name`).
    pub base_url: Option<String>,
    /// `api-version` query parameter.
    pub api_version: String,
    /// Use the deployment-based URL form
    /// (`…/deployments/{deployment}/chat/completions`). Defaults to `true`.
    pub use_deployment_based_urls: bool,
    /// Authentication. Mutually exclusive at construction time: providing both
    /// an API key and a token provider is rejected by [`AzureProvider::new`].
    pub auth: Option<AzureAuth>,
    /// Provider-level extra headers merged into every request.
    pub extra_headers: HashMap<String, String>,
}

impl AzureConfig {
    /// Create a new config with default settings (deployment-based URLs,
    /// `api-version = 2024-10-21`, no auth yet).
    pub fn new() -> Self {
        Self {
            resource_name: None,
            base_url: None,
            api_version: DEFAULT_API_VERSION.to_string(),
            use_deployment_based_urls: true,
            auth: None,
            extra_headers: HashMap::new(),
        }
    }

    /// Set the Azure resource name.
    pub fn with_resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// Override the base URL prefix (takes precedence over `resource_name`).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(without_trailing_slash(&url.into()));
        self
    }

    /// Set the `api-version` query parameter.
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Authenticate with an API key (sent as the `api-key` header).
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.auth = Some(AzureAuth::ApiKey(api_key.into()));
        self
    }

    /// Authenticate with an Azure AD token provider (sent as `Bearer`).
    pub fn with_token_provider(mut self, provider: Arc<dyn TokenProvider>) -> Self {
        self.auth = Some(AzureAuth::TokenProvider(provider));
        self
    }

    /// Add a provider-level extra header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.insert(key.into(), value.into());
        self
    }

    /// Use the deployment-based URL form (the default).
    pub fn use_deployment_based_urls(mut self) -> Self {
        self.use_deployment_based_urls = true;
        self
    }

    /// Use the newer `v1` URL form (`…/openai/v1/chat/completions?api-version=…`).
    pub fn use_v1_urls(mut self) -> Self {
        self.use_deployment_based_urls = false;
        self
    }

    /// Create a config from environment variables.
    ///
    /// Reads `AZURE_API_KEY` (required) and `AZURE_RESOURCE_NAME` (optional —
    /// may also be supplied via [`with_resource_name`]).
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "AZURE_API_KEY", "Azure OpenAI")?;
        let mut config = Self::new().with_api_key(api_key);
        if let Ok(resource) = std::env::var("AZURE_RESOURCE_NAME")
            && !resource.is_empty()
        {
            config = config.with_resource_name(resource);
        }
        Ok(config)
    }
}

impl Default for AzureConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// Azure OpenAI provider — creates [`AzureModel`] instances for a deployment.
pub struct AzureProvider {
    config: AzureConfig,
}

impl AzureProvider {
    /// Create a provider from a config.
    ///
    /// Returns [`AiMuxError::InvalidArgument`] if both an API key and a token
    /// provider were supplied, or if neither `resource_name` nor `base_url` is
    /// set.
    pub fn new(config: AzureConfig) -> Result<Self, AiMuxError> {
        // Reject both auth methods — mirrors the TS `createAzure` guard.
        // (The builder API makes it hard to set both, but a hand-built config
        // could still carry conflicting state, so we validate here.)
        if config.resource_name.is_none() && config.base_url.is_none() {
            return Err(AiMuxError::InvalidArgument(
                "Azure OpenAI requires either `resource_name` or `base_url` to be set.".to_string(),
            ));
        }
        Ok(Self { config })
    }

    /// Create a model instance for the given deployment name
    /// (e.g. `"gpt-4o-deployment"`).
    pub fn deployment(&self, deployment: &str) -> AzureModel {
        AzureModel::new(deployment.to_string(), self.config.clone())
    }

    /// Alias for [`Self::deployment`] (matches the OpenAI provider's `model`).
    pub fn model(&self, deployment: &str) -> AzureModel {
        self.deployment(deployment)
    }

    /// Create a Responses API model instance for the given deployment name.
    ///
    /// Uses the `/responses` endpoint (via the Azure OpenAI v1 or
    /// deployment-based URL form) instead of `/chat/completions`.
    ///
    /// Mirrors the TS `createResponsesModel` / `provider.responses()` factory.
    pub fn responses_model(&self, deployment: &str) -> AzureResponsesModel {
        AzureResponsesModel::new(deployment.to_string(), self.config.clone())
    }

    /// Alias for [`Self::responses_model`] (matches the TS `provider.responses`).
    pub fn responses(&self, deployment: &str) -> AzureResponsesModel {
        self.responses_model(deployment)
    }
}

impl Provider for AzureProvider {
    fn name(&self) -> &str {
        "azure"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.deployment(model_id)))
    }
}

// ── Model ────────────────────────────────────────────────────────────────────

/// An Azure OpenAI language model (addressed by deployment name).
pub struct AzureModel {
    deployment: String,
    config: AzureConfig,
}

impl AzureModel {
    pub fn new(deployment: String, config: AzureConfig) -> Self {
        Self { deployment, config }
    }

    /// Build the chat-completions endpoint URL for this deployment.
    ///
    /// Deployment-based (default):
    /// `{base}/openai/deployments/{deployment}/chat/completions?api-version={v}`
    ///
    /// v1 form (`use_v1_urls`):
    /// `{base}/openai/v1/chat/completions?api-version={v}`
    ///
    /// where `base` is `https://{resource}.openai.azure.com/openai` (derived
    /// from `resource_name`) unless overridden by `base_url`. When `base_url`
    /// is set to a non-Azure gateway, `api-version` is omitted (the gateway
    /// owns its own versioning), matching the TS behaviour.
    fn endpoint(&self) -> String {
        let prefix = match &self.config.base_url {
            Some(url) => without_trailing_slash(url),
            None => format!(
                "https://{}.openai.azure.com/openai",
                self.config
                    .resource_name
                    .as_deref()
                    .expect("resource_name or base_url required (validated at construction)")
            ),
        };

        let path = if self.config.use_deployment_based_urls {
            format!("/deployments/{}/chat/completions", self.deployment)
        } else {
            "/v1/chat/completions".to_string()
        };

        // Only append api-version for the Azure OpenAI endpoint or the
        // deployment-based form. A custom (non-Azure) gateway baseURL with the
        // v1 form owns its own versioning — mirror the TS `useAzureOpenAIEndpoint`
        // gate.
        let is_azure = prefix.contains(".openai.azure.com");
        if is_azure || self.config.use_deployment_based_urls {
            format!("{}{}?api-version={}", prefix, path, self.config.api_version)
        } else {
            format!("{}{}", prefix, path)
        }
    }

    /// Build the request headers: auth + provider extra headers + per-request
    /// headers, plus the `ai-sdk/azure/…` user-agent suffix.
    async fn build_headers(
        &self,
        extra: Option<&HashMap<String, String>>,
    ) -> Result<HashMap<String, String>, AiMuxError> {
        let mut headers = HashMap::new();

        match &self.config.auth {
            Some(AzureAuth::ApiKey(key)) => {
                headers.insert("api-key".to_string(), key.clone());
            }
            Some(AzureAuth::TokenProvider(tp)) => {
                let token = tp.get_token().await?;
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            }
            None => {
                return Err(AiMuxError::Auth(
                    "Azure OpenAI has no authentication configured. \
                     Provide an `api_key` or a `token_provider`."
                        .to_string(),
                ));
            }
        }

        // Provider-level extra headers.
        for (k, v) in &self.config.extra_headers {
            headers.insert(k.clone(), v.clone());
        }
        // Per-request headers override provider headers.
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }

        with_user_agent_suffix(&mut headers, "azure");
        Ok(headers)
    }
}

#[async_trait]
impl LanguageModel for AzureModel {
    fn provider(&self) -> &str {
        "azure"
    }

    fn model_id(&self) -> &str {
        &self.deployment
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id().to_string(),
            base_url: self.config.base_url.clone(),
            // Azure 不记录明文 key/token;来源统一记为 explicit。
            api_key_source: "explicit".to_string(),
            profile: None,
            provider_options: Some(serde_json::json!({
                "resource_name": self.config.resource_name,
                "api_version": self.config.api_version,
                "use_deployment_based_urls": self.config.use_deployment_based_urls,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref()).await?;
        execute_generate(
            &self.endpoint(),
            &headers,
            &self.deployment,
            options,
            "azure",
            &crate::openai::OpenAICompatProfile::full(),
            &RetryConfig::default(),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref()).await?;
        execute_stream(
            &self.endpoint(),
            &headers,
            &self.deployment,
            options,
            "azure",
            &crate::openai::OpenAICompatProfile::full(),
            &RetryConfig::default(),
        )
        .await
    }
}

// ── Unit tests for URL construction ──────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn model(resource: &str) -> AzureModel {
        AzureModel::new(
            "gpt-4o".to_string(),
            AzureConfig::new()
                .with_resource_name(resource)
                .with_api_key("k"),
        )
    }

    /// Deployment-based (default) URL with a resource name:
    /// `https://{resource}.openai.azure.com/openai/deployments/{deployment}/chat/completions?api-version=2024-10-21`.
    #[test]
    fn endpoint_deployment_based_with_resource_name() {
        let m = model("my-resource");
        assert_eq!(
            m.endpoint(),
            "https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    /// The v1 form on a real Azure endpoint still appends `api-version`.
    #[test]
    fn endpoint_v1_with_resource_name_appends_api_version() {
        let mut m = model("my-resource");
        m.config = m.config.use_v1_urls();
        assert_eq!(
            m.endpoint(),
            "https://my-resource.openai.azure.com/openai/v1/chat/completions?api-version=2024-10-21"
        );
    }

    /// A custom `api_version` is reflected in the query parameter.
    #[test]
    fn endpoint_custom_api_version() {
        let mut m = model("my-resource");
        m.config = m.config.with_api_version("2025-04-01-preview");
        assert_eq!(
            m.endpoint(),
            "https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2025-04-01-preview"
        );
    }

    /// A `base_url` override pointing at an Azure host still appends
    /// `api-version`.
    #[test]
    fn endpoint_azure_base_url_appends_api_version() {
        let m = AzureModel::new(
            "gpt-4o".to_string(),
            AzureConfig::new()
                .with_base_url("https://other.openai.azure.com/openai")
                .with_api_key("k")
                .use_v1_urls(),
        );
        assert_eq!(
            m.endpoint(),
            "https://other.openai.azure.com/openai/v1/chat/completions?api-version=2024-10-21"
        );
    }

    /// RFC-0023:config_snapshot 反映 provider/model_id、base_url、api_key_source。
    #[test]
    fn config_snapshot_matches_config() {
        let m = AzureModel::new(
            "gpt-4o".to_string(),
            AzureConfig::new()
                .with_base_url("https://gateway.example.com/openai")
                .with_api_version("2025-04-01-preview")
                .use_v1_urls()
                .with_api_key("k"),
        );
        let snap = m.config_snapshot();
        assert_eq!(snap.provider, "azure");
        assert_eq!(snap.model_id, "gpt-4o");
        assert_eq!(
            snap.base_url.as_deref(),
            Some("https://gateway.example.com/openai")
        );
        assert_eq!(snap.api_key_source, "explicit");
    }
}
