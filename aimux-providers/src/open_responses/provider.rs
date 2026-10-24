//! The Open Responses provider (open-responses-provider.ts).

use std::sync::Arc;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable, validate_base_url};

use crate::shared::{
    Credential, EndpointConfig, EndpointHeaders, ProviderHeaders, TransformRequestBody,
};

use super::responses::OpenResponsesModel;

// == Settings ==

/// Settings of [`create_open_responses`].
///
/// Everything but `api_key` and `headers` is fixed when the provider is
/// created; those two are evaluated on every request.
#[derive(Clone)]
pub struct OpenResponsesProviderSettings {
    /// The provider name: `provider()` is `"{name}.responses"` and its first
    /// dot-separated segment is the providerOptions key the model reads.
    pub name: String,
    /// Base URL of the server (e.g. `http://localhost:1234/v1`); requests go
    /// to `{base_url}/responses`. A trailing slash is removed.
    pub base_url: String,
    /// The API key, sent as `Authorization: Bearer`. `None` sends no
    /// credential (a local server) and reads no environment variable. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers, resolved on every request (a `None` value removes a
    /// header, including `Authorization`). Per-call headers win over these.
    pub headers: Option<Resolvable<HeaderMapOpt>>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
    /// The package part of the user-agent (AI SDK `userAgentSuffix`).
    /// `None` sends `ai-sdk-open-responses/<version>` of the pinned
    /// `@ai-sdk/open-responses`.
    pub user_agent_suffix: Option<String>,
}

impl OpenResponsesProviderSettings {
    /// Settings with the two required fields; the rest unset.
    #[must_use]
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: None,
            headers: None,
            fetch: None,
            transform_request_body: None,
            user_agent_suffix: None,
        }
    }
}

impl std::fmt::Debug for OpenResponsesProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenResponsesProviderSettings")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("headers", &self.headers.is_some())
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .field("user_agent_suffix", &self.user_agent_suffix)
            .finish()
    }
}

/// The first dot-separated segment of a provider name, trimmed: the
/// providerOptions key of the provider.
fn options_name_of(name: &str) -> String {
    name.split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Create an Open Responses provider.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` when `name` is empty or `base_url` is
/// not an `http(s)` URL with a host. Credentials are resolved per request, not
/// here.
pub fn create_open_responses(
    settings: OpenResponsesProviderSettings,
) -> Result<OpenResponsesProvider, AiMuxError> {
    if settings.name.trim().is_empty() {
        return Err(AiMuxError::invalid_argument(
            "Open Responses requires a non-empty `name`.".to_string(),
        ));
    }
    let base_url = validate_base_url(&settings.base_url)?;
    let credential = match settings.api_key {
        Some(key) => Credential::Explicit(key),
        None => Credential::None,
    };
    let provider = ProviderHeaders::bearer(credential, Vec::new(), None);
    // open-responses-provider.ts:114-127: the key and the user headers, then
    // `withUserAgentSuffix(…, userAgentSuffix ?? ai-sdk-open-responses/VERSION)`.
    let package = crate::shared::aisdk_pins::OPEN_RESPONSES;
    let suffix: Arc<str> = settings
        .user_agent_suffix
        .unwrap_or_else(|| format!("ai-sdk-{}/{}", package.name, package.version))
        .into();
    let headers = EndpointHeaders::WithUserHeaders {
        provider,
        user: settings.headers,
        user_agent_suffix: Some(suffix),
    };
    Ok(OpenResponsesProvider {
        name: settings.name,
        base_url,
        headers,
        fetch: Some(crate::shared::package_fetch(settings.fetch, package)),
        transform_request_body: settings.transform_request_body,
    })
}

// == Provider ==

/// Open Responses provider - creates [`OpenResponsesModel`] instances.
pub struct OpenResponsesProvider {
    name: String,
    base_url: String,
    headers: EndpointHeaders,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl OpenResponsesProvider {
    /// A Responses model; `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> OpenResponsesModel {
        OpenResponsesModel {
            model_id: model_id.to_string(),
            provider_options_name: options_name_of(&self.name),
            config: EndpointConfig::fixed(
                format!("{}.responses", self.name),
                self.base_url.clone(),
                self.headers.clone(),
                self.fetch.clone(),
                self.transform_request_body.clone(),
            ),
        }
    }

    /// The provider as a function: the default language model for an id. The
    /// same model as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for OpenResponsesProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}
