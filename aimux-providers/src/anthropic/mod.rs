//! Anthropic provider.
//!
//! [`create_anthropic`] is the Rust form of the AI SDK's `createAnthropic`: it
//! takes [`AnthropicProviderSettings`], validates the base URL, rejects an
//! `api_key` given together with an `auth_token`, fixes the provider name and
//! returns an [`AnthropicProvider`]. Credentials are not read there; they are
//! loaded in the request headers of every call, from the settings or (for the
//! key) from `ANTHROPIC_API_KEY`. [`anthropic()`] is the default instance.
//!
//! The Messages model is shared with Anthropic on AWS
//! ([`crate::anthropic_aws`]) and Anthropic on Vertex
//! ([`crate::vertex`]): they build the same private model configuration and differ
//! only in what it holds.

pub mod cache_control;
pub(crate) mod config;
pub mod convert;
pub mod files;
pub mod model;
pub(crate) mod options;
pub mod prepare_tools;
pub mod sanitize_json_schema;
pub mod skills;
pub mod stream;
pub mod tool_name_mapping;
pub mod types;
pub mod usage;

pub use files::AnthropicFiles;
pub use model::AnthropicMessagesModel;
pub use skills::AnthropicSkills;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, load_optional_setting, validate_base_url,
};

use crate::shared::{AuthScheme, Credential, credential_headers};

use config::{AnthropicModelConfig, AnthropicModelHooks};

pub(crate) fn anthropic_failed_response_handler()
-> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error.get("type").and_then(Value::as_str).map(str::to_owned),
        }
    })
}

const API_ORIGIN: &str = "https://api.anthropic.com";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const API_KEY_ENV_VAR: &str = "ANTHROPIC_API_KEY";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_NAME: &str = "anthropic.messages";

/// The first-party URL without its version segment means the versioned one
/// (`normalizeBaseURL` in `createAnthropic`); anything else is taken as given.
fn normalize_base_url(url: &str) -> Result<String, AiMuxError> {
    let url = validate_base_url(url)?;
    Ok(if url == API_ORIGIN {
        DEFAULT_BASE_URL.to_string()
    } else {
        url
    })
}

/// The URL patterns the API fetches itself (`supportedUrls` in `createAnthropic`).
fn supported_urls() -> SupportedUrls {
    let https = regex::Regex::new(r"^https?://.*$").expect("static pattern");
    SupportedUrls(
        [
            ("image/*".to_string(), vec![https.clone()]),
            ("application/pdf".to_string(), vec![https]),
        ]
        .into_iter()
        .collect(),
    )
}

/// Settings of [`create_anthropic`] (the AI SDK's `AnthropicProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url`, `name` and the `api_key`/`auth_token` conflict;
/// credentials and `headers` are evaluated on every request.
#[derive(Clone, Default)]
pub struct AnthropicProviderSettings {
    /// Base URL for the API calls, version segment included. Default
    /// `https://api.anthropic.com/v1`; a trailing slash is removed. The bare
    /// `https://api.anthropic.com` means the default.
    pub base_url: Option<String>,
    /// The API key, sent as `x-api-key`. `None` (with no `auth_token`) loads
    /// `ANTHROPIC_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included: it never falls back to the environment.
    pub api_key: Option<String>,
    /// A non-empty bearer token is sent as `Authorization: Bearer` instead
    /// of `x-api-key`. Giving non-empty values for both credentials is an error.
    pub auth_token: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including one of the fixed ones. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the `provider()` string of the messages model.
    /// Default `"anthropic.messages"`. Its first dot-separated segment is
    /// also the providerOptions key read in addition to `anthropic` (and the
    /// key response metadata is written under).
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Generator for source IDs. Defaults to the provider utility generator.
    pub generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl std::fmt::Debug for AnthropicProviderSettings {
    /// Never prints credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field("auth_token", &self.auth_token.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field("generate_id", &self.generate_id.is_some())
            .finish()
    }
}

/// Create an Anthropic provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host, or when both `api_key` and `auth_token` are given. Those
/// are the only ways this fails: credentials are loaded per request, not here.
pub fn create_anthropic(
    settings: AnthropicProviderSettings,
) -> Result<AnthropicProvider, AiMuxError> {
    let base_url = match load_optional_setting(settings.base_url.as_deref(), "ANTHROPIC_BASE_URL") {
        Some(url) => normalize_base_url(&url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    let is_truthy = |value: &String| !value.is_empty();
    if settings.api_key.as_ref().is_some_and(is_truthy)
        && settings.auth_token.as_ref().is_some_and(is_truthy)
    {
        return Err(AiMuxError::InvalidArgument(
            "Both apiKey and authToken were provided. Please use only one authentication method."
                .to_string(),
        ));
    }
    let (credential, scheme) = match settings.auth_token.filter(is_truthy) {
        Some(token) => (
            Credential::Explicit(Resolvable::Value(token)),
            AuthScheme::Bearer,
        ),
        None => (
            Credential::explicit_or_env(
                settings.api_key.map(Resolvable::Value),
                API_KEY_ENV_VAR,
                "Anthropic",
            ),
            AuthScheme::Header("x-api-key"),
        ),
    };
    Ok(AnthropicProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            credential_headers(
                credential,
                scheme,
                vec![("anthropic-version".to_string(), API_VERSION.to_string())],
                settings.headers,
            ),
            crate::anthropic::options::CANONICAL,
            "4.0.68",
        ),
        fetch: settings.fetch,
        generate_id: settings.generate_id,
        supported_urls: supported_urls(),
    })
}

/// The default provider: `create_anthropic` with default settings, created on
/// first use. The base URL is loaded at creation; the key is loaded per request.
pub fn anthropic() -> &'static AnthropicProvider {
    static DEFAULT: OnceLock<AnthropicProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_anthropic(AnthropicProviderSettings::default()).expect("invalid ANTHROPIC_BASE_URL")
    })
}

/// An Anthropic provider (the AI SDK's `AnthropicProvider`). Cheap to clone
/// the models out of; it holds no HTTP client.
pub struct AnthropicProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    supported_urls: SupportedUrls,
}

impl AnthropicProvider {
    /// The model configuration reporting `provider` as its identity.
    fn model_config(&self, provider: String) -> AnthropicModelConfig {
        let base = self.base_url.clone();
        AnthropicModelConfig {
            provider,
            url: Arc::new(move |path| format!("{base}{path}")),
            headers: self.headers.clone(),
            fetch: self.fetch.clone(),
            supported_urls: self.supported_urls.clone(),
            transform_request_body: None,
            base_url: self.base_url.clone(),
            provider_options_name: options::options_name_of(&self.name),
            hooks: AnthropicModelHooks::default(),
            resolve: None,
        }
    }

    /// The name with the `.messages` suffix removed: the prefix of the
    /// provider strings of the other modalities.
    fn bare_name(&self) -> &str {
        self.name.strip_suffix(".messages").unwrap_or(&self.name)
    }

    /// A Messages model; `provider()` is the provider name
    /// (`"anthropic.messages"` by default).
    #[must_use]
    pub fn messages(&self, model_id: &str) -> AnthropicMessagesModel {
        AnthropicMessagesModel::with_config(
            model_id.to_string(),
            self.model_config(self.name.clone()),
        )
        .with_generate_id(self.generate_id.clone())
    }

    /// The files interface; `provider()` is the provider name.
    #[must_use]
    pub fn files(&self) -> AnthropicFiles {
        AnthropicFiles::from_config(self.model_config(self.name.clone()))
    }

    /// The skills upload interface.
    #[must_use]
    pub fn skills(&self) -> AnthropicSkills {
        AnthropicSkills::from_config(
            self.model_config(format!("{}.skills", self.name.replacen(".messages", "", 1))),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as
    /// [`messages`](Self::messages) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.messages(model_id))
    }
}

impl Provider for AnthropicProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        Some(Arc::new(self.files()))
    }

    fn skills(&self) -> Option<Arc<dyn aimux_core::skills_model::Skills>> {
        Some(Arc::new(self.skills()))
    }
}

impl ProviderDiscovery for AnthropicProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config(format!("{}.models", self.bare_name()));
        Box::pin(async move { list_models_once(&config).await })
    }
}

/// One `GET {base_url}/models` exchange against an Anthropic-shaped API: no
/// retry, no recording. Discovery is not a Core operation and the AI SDK has
/// no equivalent, so a failure is reported to the caller as it happened.
///
/// # Errors
///
/// Returns the header-resolution error (a missing key is `LoadApiKey`),
/// `ApiCall` for HTTP/transport failures and `JsonParse` when the body does
/// not deserialize into the models list.
pub(crate) async fn list_models_once(
    config: &AnthropicModelConfig,
) -> Result<Vec<RuntimeModel>, AiMuxError> {
    let config = &config.resolved().await?;
    #[derive(serde::Deserialize)]
    struct Resp {
        #[serde(default)]
        data: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        #[serde(default)]
        display_name: Option<String>,
    }

    let headers = config
        .request_headers(None, &std::collections::BTreeSet::new())
        .await?;
    let resp = aimux_provider_utils::get_from_api(
        config.with_transport(aimux_provider_utils::HttpRequest {
            url: config.url("/models"),
            headers,
            ..Default::default()
        }),
        aimux_provider_utils::create_json_response_handler(),
        config.failed_response_handler(),
    )
    .await?;
    let parsed: Resp = resp.value;
    Ok(parsed
        .data
        .into_iter()
        .map(|entry| RuntimeModel {
            id: entry.id,
            owned_by: entry.display_name,
            created: None,
        })
        .collect())
}
