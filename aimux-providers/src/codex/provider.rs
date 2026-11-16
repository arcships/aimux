//! Codex provider factory, settings, and channel configuration.

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    AuthScheme, Credential, FetchFunction, HeaderMapOpt, ProviderHeaders, Resolvable,
    validate_base_url,
};

use crate::openai::internal::{OpenAIModelConfig, list_models_once, supported_urls};
use crate::shared::TransformRequestBody;

use super::responses_model::CodexModel;

/// Default API-key base URL (official OpenAI Responses endpoint).
pub const CODEX_API_BASE_URL: &str = "https://api.openai.com/v1";
/// Default subscription base URL (undocumented ChatGPT backend endpoint). The
/// Responses path is appended, giving `https://chatgpt.com/backend-api/codex/responses`
/// (the endpoint the `chatgpt` cassettes record).
pub const CODEX_SUBSCRIPTION_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Environment variable the API-key mode reads when no key is given.
pub const CODEX_API_KEY_ENV_VAR: &str = "CODEX_API_KEY";

const DEFAULT_NAME: &str = "codex";
const DEFAULT_ORIGINATOR: &str = "aimux";

/// Access mode of the Codex provider, with the credential of that channel.
#[derive(Debug, Clone)]
pub enum CodexMode {
    /// Official API-key channel (`api.openai.com`), usage-based. `None` loads
    /// `CODEX_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset; an explicit key is used as
    /// given, `""` included.
    ApiKey(Option<Resolvable<String>>),
    /// ChatGPT-subscription channel (`chatgpt.com/backend-api`), single
    /// personal account; always streams.
    ///
    /// `token` is the access token minted by the integrator's OAuth
    /// device-code flow: the library never performs login, persists tokens, or
    /// refreshes automatically (RFC-0018 §3.2). A [`Resolvable::AsyncFn`] is
    /// evaluated on every request.
    ChatGptAccount {
        /// The account access token, sent as `Authorization: Bearer`.
        token: Resolvable<String>,
        /// The `ChatGPT-Account-Id` header value (optional).
        account_id: Option<String>,
    },
}

impl Default for CodexMode {
    /// The API-key channel, key from `CODEX_API_KEY`.
    fn default() -> Self {
        Self::ApiKey(None)
    }
}

/// Settings of [`create_codex`].
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; the credential and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct CodexProviderSettings {
    /// The access mode and its credential. Default: API key from
    /// `CODEX_API_KEY`.
    pub mode: CodexMode,
    /// Base URL for the API calls. Default [`CODEX_API_BASE_URL`] for
    /// `ApiKey` and [`CODEX_SUBSCRIPTION_BASE_URL`] for `ChatGptAccount`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `Authorization`. They win over the channel headers
    /// (`Originator`, `ChatGPT-Account-Id`); per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of `provider()` (`"{name}.responses"`).
    /// Default `"codex"`.
    pub name: Option<String>,
    /// `ChatGptAccount` mode: the `Originator` header value (client
    /// identifier; reference clients use e.g. `"opencode"` / `"codex_cli_rs"`).
    /// Default `"aimux"`.
    pub originator: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized (and
    /// after the `store: false` rule of the subscription channel) and before it
    /// is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for CodexProviderSettings {
    /// Never prints the credential or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexProviderSettings")
            .field("mode", &self.mode)
            .field("base_url", &self.base_url)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("originator", &self.originator)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Which channel a provider talks to; the model rules differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Channel {
    ApiKey,
    ChatGptAccount,
}

/// Create a Codex provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: credentials are resolved
/// per request, not here.
pub fn create_codex(settings: CodexProviderSettings) -> Result<CodexProvider, AiMuxError> {
    let (channel, default_base_url, credential, fixed) = match settings.mode {
        CodexMode::ApiKey(key) => (
            Channel::ApiKey,
            CODEX_API_BASE_URL,
            Credential::explicit_or_env(key, CODEX_API_KEY_ENV_VAR, "Codex"),
            Vec::new(),
        ),
        CodexMode::ChatGptAccount { token, account_id } => {
            let mut fixed = vec![(
                "Originator".to_string(),
                settings
                    .originator
                    .unwrap_or_else(|| DEFAULT_ORIGINATOR.to_string()),
            )];
            if let Some(id) = account_id {
                fixed.push(("ChatGPT-Account-Id".to_string(), id));
            }
            (
                Channel::ChatGptAccount,
                CODEX_SUBSCRIPTION_BASE_URL,
                Credential::Explicit(token),
                fixed,
            )
        }
    };
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => default_base_url.to_string(),
    };
    Ok(CodexProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        channel,
        base_url,
        headers: ProviderHeaders::new(credential, AuthScheme::Bearer, fixed, settings.headers),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_codex` with default settings (API key from
/// `CODEX_API_KEY`), created on first use. Creating it reads nothing from the
/// environment and cannot fail; a missing key surfaces from the first request
/// instead.
pub fn codex() -> &'static CodexProvider {
    static DEFAULT: OnceLock<CodexProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_codex(CodexProviderSettings::default())
            .expect("default Codex settings are always valid")
    })
}

/// A Codex provider. Cheap to clone the models out of; it holds no HTTP
/// client.
pub struct CodexProvider {
    name: String,
    channel: Channel,
    base_url: String,
    headers: ProviderHeaders,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl CodexProvider {
    fn model_config(&self, method: &str) -> OpenAIModelConfig {
        let mut config = OpenAIModelConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        );
        // Codex speaks the Responses API, so its models take the URLs the
        // OpenAI Responses model takes.
        config.supported_urls = supported_urls(method);
        config
    }

    /// A Codex model over the Responses API (e.g. `"gpt-5.2-codex"`);
    /// `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> CodexModel {
        CodexModel::from_config(
            model_id.to_string(),
            self.channel,
            self.model_config("responses"),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// same model as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for CodexProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn language_model_named(
        &self,
        method: &str,
        model_id: &str,
    ) -> Option<Result<Arc<dyn LanguageModel>, AiMuxError>> {
        (method == "responses").then(|| Ok(Arc::new(self.responses(model_id)) as _))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for CodexProvider {
    /// `GET {base_url}/models` (OpenAI-compatible): one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("models");
        Box::pin(async move { list_models_once(&config).await })
    }
}
