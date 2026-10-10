//! # Azure OpenAI provider
//!
//! [`create_azure`] is the Rust form of the AI SDK's `createAzure`: it takes
//! [`AzureOpenAIProviderSettings`], rejects a non-empty `api_key` given with a
//! `token_provider`, validates the base URL and returns an
//! [`AzureOpenAIProvider`]. Nothing is read from the environment there: the
//! API key (`AZURE_API_KEY`), the Entra ID token and the resource name
//! (`AZURE_RESOURCE_NAME`) are resolved on every request, so a missing one
//! fails that request with `AiMuxError::LoadApiKey` / `LoadSetting`.
//! [`azure()`] is the default instance.
//!
//! Azure speaks the OpenAI wire format, so its models are the OpenAI package's
//! (`crate::openai`) configured for Azure: provider strings
//! `azure.chat` / `azure.responses` / `azure.embeddings` / `azure.image` /
//! `azure.transcription` / `azure.speech`, a URL that follows the AI SDK's rules
//! (`{base}/v1{path}?api-version=` or the deployment form
//! `{base}/deployments/{id}{path}?api-version=`), `api-key` or
//! `Authorization: Bearer` authentication, and the Responses model's `azure`
//! providerOptions namespace and `assistant-` file-id prefix.
//!
//! Not ported from the AI SDK package: Azure-hosted DeepSeek (`deepseek`),
//! the legacy completion model and MAI-Voice (`maiBaseURL`, `webSocket`).

pub(crate) mod options;
mod transcription;

pub use transcription::AzureTranscriptionModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, HeaderMapOpt, HeadersFn,
    HttpRequest, Resolvable, default_fetch, load_setting, validate_base_url,
    without_trailing_slash,
};

use crate::openai::config::{OpenAIModelConfig, OpenAIUrl};
use crate::openai::responses::ResponsesProfile;
use crate::openai::{
    OpenAIEmbeddingModel, OpenAIImageModel, OpenAIModel, OpenAIResponsesModel, OpenAISpeechModel,
    OpenAITranscriptionModel,
};
use crate::shared::{AuthScheme, Credential, credential_headers, is_valid_hostname_part};

/// The chat-completions model of the Azure package (`provider.chat(id)`): the
/// OpenAI one, configured for Azure.
pub type AzureChatModel = OpenAIModel;
/// The Responses model of the Azure package: the OpenAI one, configured for
/// Azure.
pub type AzureResponsesModel = OpenAIResponsesModel;

const API_KEY_ENV_VAR: &str = "AZURE_API_KEY";
const RESOURCE_NAME_ENV_VAR: &str = "AZURE_RESOURCE_NAME";
/// The `api-version` of the AI SDK's v1 URL form.
const DEFAULT_API_VERSION: &str = "v1";
/// The `api-version` the deployments listing needs: the v1 API has no such
/// call, so discovery keeps the dated version unless the caller sets one.
const DEPLOYMENTS_API_VERSION: &str = "2024-10-21";

/// Settings of [`create_azure`] (the AI SDK's `AzureOpenAIProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and the `api_key` / `token_provider` conflict;
/// the credential, the resource name and `headers` are evaluated on every
/// request.
#[derive(Clone, Default)]
pub struct AzureOpenAIProviderSettings {
    /// Name of the Azure OpenAI resource, the `{resource}` in
    /// `https://{resource}.openai.azure.com/openai`. Either this or `base_url`
    /// can be used; `None` (and no `base_url`) loads `AZURE_RESOURCE_NAME`
    /// when a request is made and fails that request with
    /// `AiMuxError::LoadSetting` if it is unset. It must be a single DNS label.
    pub resource_name: Option<String>,
    /// A different URL prefix for API calls, e.g. a proxy. Wins over
    /// `resource_name`. With an unversioned Azure OpenAI base URL the request
    /// URL is `{base_url}/v1{path}`; one that already ends in `/openai/v1` is
    /// used as is; a non-Azure gateway gets `{base_url}{path}`. A trailing
    /// slash is removed.
    pub base_url: Option<String>,
    /// Azure Speech endpoint prefix. Independent of `base_url` and `api_version`.
    /// Defaults to the resource's Cognitive Services endpoint.
    pub speech_base_url: Option<String>,
    /// The API key, sent as `api-key`. `None` (with no `token_provider`) loads
    /// `AZURE_API_KEY` when a request is made and fails that request with
    /// `AiMuxError::LoadApiKey` if it is unset. An explicit value is used as
    /// given, `""` included. Giving a non-empty key together with
    /// `token_provider` is an error.
    pub api_key: Option<Resolvable<String>>,
    /// Microsoft Entra ID access token, sent as `Authorization: Bearer` (and
    /// no `api-key`). Use an [`Resolvable::AsyncFn`] to get a fresh token on
    /// every request. A caller-set `Authorization` header wins over it.
    pub token_provider: Option<Resolvable<String>>,
    /// Extra headers on every request. A `None` value removes the header.
    /// Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// The `api-version` query parameter. Default `"v1"`.
    pub api_version: Option<String>,
    /// Use the legacy deployment URL form
    /// `{base}/deployments/{deployment}{path}?api-version=` instead of
    /// `{base}/v1{path}`.
    pub use_deployment_based_urls: bool,
}

impl std::fmt::Debug for AzureOpenAIProviderSettings {
    /// Never prints credentials or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureOpenAIProviderSettings")
            .field("resource_name", &self.resource_name)
            .field("base_url", &self.base_url)
            .field("speech_base_url", &self.speech_base_url)
            .field("api_key", &self.api_key)
            .field("token_provider", &self.token_provider)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("fetch", &self.fetch.is_some())
            .field("api_version", &self.api_version)
            .field("use_deployment_based_urls", &self.use_deployment_based_urls)
            .finish()
    }
}

/// What the AI SDK's `getAzureOpenAIBaseURLInfo` derives from a base URL.
#[derive(Debug, Clone, Copy)]
struct BaseUrlInfo {
    is_azure_openai: bool,
    is_foundry_project: bool,
    is_versioned: bool,
}

fn base_url_info(base_url: Option<&str>) -> Result<BaseUrlInfo, AiMuxError> {
    let Some(base_url) = base_url else {
        return Ok(BaseUrlInfo {
            is_azure_openai: true,
            is_foundry_project: false,
            is_versioned: false,
        });
    };
    let url = url::Url::parse(base_url)
        .map_err(|e| AiMuxError::InvalidArgument(format!("invalid Azure base URL: {e}")))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let is_azure_openai = host.ends_with(".openai.azure.com")
        || host.ends_with(".services.ai.azure.com")
        || host.ends_with(".cognitiveservices.azure.com");
    let path = url.path().trim_end_matches('/').to_string();
    Ok(BaseUrlInfo {
        is_azure_openai,
        is_foundry_project: host.ends_with(".services.ai.azure.com")
            && path.starts_with("/api/projects/"),
        is_versioned: is_azure_openai && path.to_ascii_lowercase().ends_with("/openai/v1"),
    })
}

/// Create an Azure OpenAI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when a non-empty `api_key` and
/// `token_provider` are given or `base_url` / `speech_base_url` is not an `http(s)` URL with a
/// host. Those are the only ways this fails: credentials and the resource
/// name are resolved per request, not here.
pub fn create_azure(
    settings: AzureOpenAIProviderSettings,
) -> Result<AzureOpenAIProvider, AiMuxError> {
    let has_api_key = match &settings.api_key {
        None => false,
        Some(Resolvable::Value(value)) => !value.is_empty(),
        Some(_) => true,
    };
    if has_api_key && settings.token_provider.is_some() {
        return Err(AiMuxError::InvalidArgument(
            "Both apiKey and tokenProvider were provided. Please use only one authentication \
             method."
                .to_string(),
        ));
    }
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    let speech_base_url = settings
        .speech_base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    let info = base_url_info(base_url.as_deref())?;
    let fetch = settings
        .token_provider
        .clone()
        .map(|token| {
            Arc::new(BearerTokenFetch {
                token,
                fetch: settings.fetch.clone(),
            }) as FetchFunction
        })
        .or(settings.fetch);
    let speech_headers = match &settings.token_provider {
        Some(_) => Resolvable::Value(settings.headers.clone().unwrap_or_default()),
        None => credential_headers(
            Credential::explicit_or_env(settings.api_key.clone(), API_KEY_ENV_VAR, "Azure Speech"),
            AuthScheme::Header("Ocp-Apim-Subscription-Key"),
            Vec::new(),
            settings.headers.clone(),
        ),
    };
    let headers = match settings.token_provider {
        Some(_) => Resolvable::Value(settings.headers.unwrap_or_default()),
        None => credential_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "Azure OpenAI"),
            AuthScheme::Header("api-key"),
            Vec::new(),
            settings.headers,
        ),
    };
    Ok(AzureOpenAIProvider {
        resource_name: settings.resource_name,
        base_url,
        speech_base_url,
        speech_headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            speech_headers,
            options::NAMESPACE,
            "4.0.84",
        ),
        info,
        api_version: settings.api_version,
        use_deployment_based_urls: settings.use_deployment_based_urls,
        fetch,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            headers,
            options::NAMESPACE,
            "4.0.84",
        ),
    })
}

/// Resolve authentication after per-call headers have been merged.
struct BearerTokenFetch {
    token: Resolvable<String>,
    fetch: Option<FetchFunction>,
}

#[async_trait::async_trait]
impl Fetch for BearerTokenFetch {
    async fn fetch(&self, mut request: FetchRequest) -> Result<FetchResponse, FetchError> {
        if !request.headers.contains_key("authorization") {
            let token = self
                .token
                .resolve()
                .await
                .map_err(|error| FetchError::Other(error.to_string()))?;
            request.headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {token}").parse().map_err(
                    |error: reqwest::header::InvalidHeaderValue| {
                        FetchError::Other(error.to_string())
                    },
                )?,
            );
        }
        self.fetch
            .clone()
            .unwrap_or_else(default_fetch)
            .fetch(request)
            .await
    }
}

/// The default provider: `create_azure` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key or resource name surfaces from the first request instead.
pub fn azure() -> &'static AzureOpenAIProvider {
    static DEFAULT: OnceLock<AzureOpenAIProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_azure(AzureOpenAIProviderSettings::default())
            .expect("default Azure settings are always valid")
    })
}

/// An Azure OpenAI provider (the AI SDK's `AzureOpenAIProvider`). Models are
/// addressed by deployment name. Cheap to clone the models out of; it holds no
/// HTTP client.
pub struct AzureOpenAIProvider {
    resource_name: Option<String>,
    base_url: Option<String>,
    speech_base_url: Option<String>,
    speech_headers: HeadersFn,
    info: BaseUrlInfo,
    api_version: Option<String>,
    use_deployment_based_urls: bool,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
}

/// Everything the URL of a request depends on.
#[derive(Clone)]
pub(crate) struct UrlRules {
    resource_name: Option<String>,
    base_url: Option<String>,
    info: BaseUrlInfo,
    api_version: String,
    use_deployment_based_urls: bool,
}

impl UrlRules {
    /// The URL prefix: the base URL, or the resource's `/openai` endpoint.
    fn prefix(&self) -> Result<String, AiMuxError> {
        if let Some(base_url) = &self.base_url {
            return Ok(without_trailing_slash(base_url));
        }
        let resource = load_setting(
            self.resource_name.as_deref(),
            RESOURCE_NAME_ENV_VAR,
            "resourceName",
        )?;
        // The resource name becomes part of the request host, so only a DNS
        // label is accepted (`user@internal:8080/#` would rewrite the host).
        if !is_valid_hostname_part(&resource) {
            return Err(AiMuxError::InvalidArgument(
                "Invalid Azure resource name. Expected a single DNS label (letters, digits, and \
                 hyphens). Use `base_url` for custom endpoints."
                    .to_string(),
            ));
        }
        Ok(format!("https://{resource}.openai.azure.com/openai"))
    }

    /// The AI SDK's `url({ path, modelId })`.
    pub(crate) fn url(&self, path: &str, model_id: &str) -> Result<String, AiMuxError> {
        let prefix = self.prefix()?;
        let info = self.info;
        let full = if self.use_deployment_based_urls {
            format!("{prefix}/deployments/{model_id}{path}")
        } else if !info.is_azure_openai || info.is_versioned {
            // Custom gateways own Azure routing and versioning themselves, and
            // complete Azure OpenAI v1 URLs own theirs.
            format!("{prefix}{path}")
        } else {
            format!("{prefix}/v1{path}")
        };
        let mut url = url::Url::parse(&full)
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid Azure URL `{full}`: {e}")))?;
        if self.use_deployment_based_urls
            || (info.is_azure_openai && !info.is_versioned && !info.is_foundry_project)
        {
            set_query_param(&mut url, "api-version", &self.api_version);
        }
        Ok(url.to_string())
    }

    /// The Azure AI Speech transcription URL: `speech_base_url`, or the
    /// resource's Cognitive Services host.
    pub(crate) fn speech_url(&self, speech_base_url: Option<&str>) -> Result<String, AiMuxError> {
        let prefix = match speech_base_url {
            Some(prefix) => without_trailing_slash(prefix),
            None => {
                // Reuse the resource-name validation and lazy environment lookup.
                let mut rules = self.clone();
                rules.base_url = None;
                rules
                    .prefix()?
                    .replace(".openai.azure.com/openai", ".cognitiveservices.azure.com")
            }
        };
        Ok(format!(
            "{prefix}/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
        ))
    }
}

/// `URLSearchParams.set`: replace the parameter, or append it.
fn set_query_param(url: &mut url::Url, name: &str, value: &str) {
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    match pairs.iter().position(|(k, _)| k == name) {
        Some(index) => {
            pairs[index].1 = value.to_string();
            let mut seen = false;
            pairs.retain(|(k, _)| {
                if k == name {
                    let keep = !seen;
                    seen = true;
                    keep
                } else {
                    true
                }
            });
        }
        None => pairs.push((name.to_string(), value.to_string())),
    }
    url.query_pairs_mut().clear().extend_pairs(pairs);
}

impl AzureOpenAIProvider {
    fn url_rules(&self) -> UrlRules {
        UrlRules {
            resource_name: self.resource_name.clone(),
            base_url: self.base_url.clone(),
            info: self.info,
            api_version: self
                .api_version
                .clone()
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_string()),
            use_deployment_based_urls: self.use_deployment_based_urls,
        }
    }

    /// The model configuration of a deployment, reporting `provider` as its
    /// identity.
    fn model_config(&self, provider: &str, deployment: &str) -> OpenAIModelConfig {
        OpenAIModelConfig {
            provider: provider.to_string(),
            url: OpenAIUrl::AzureDeployment {
                rules: self.url_rules(),
                deployment: deployment.to_string(),
            },
            headers: self.headers.clone(),
            token_provider: None,
            fetch: self.fetch.clone(),
            supported_urls: crate::openai::config::supported_urls(provider),
            responses: ResponsesProfile::default(),
            chat_options: options::CHAT_OPTIONS,
        }
    }

    /// A chat-completions model for a deployment; `provider()` is
    /// `"azure.chat"`.
    #[must_use]
    pub fn chat(&self, deployment: &str) -> AzureChatModel {
        OpenAIModel::from_config(
            deployment.to_string(),
            self.model_config("azure.chat", deployment),
        )
    }

    /// A Responses API model for a deployment; `provider()` is
    /// `"azure.responses"`. Reads providerOptions from `azure` (then `openai`),
    /// writes metadata under `azure`, and treats `assistant-` file data as
    /// uploaded file ids.
    #[must_use]
    pub fn responses(&self, deployment: &str) -> AzureResponsesModel {
        let mut config = self.model_config("azure.responses", deployment);
        config.responses = ResponsesProfile {
            namespace: options::RESPONSES,
            file_id_prefixes: vec!["assistant-"],
            explicit_message_item_type: self.info.is_foundry_project,
        };
        OpenAIResponsesModel::from_config(deployment.to_string(), config)
    }

    /// An embedding model for a deployment; `provider()` is
    /// `"azure.embeddings"`.
    #[must_use]
    pub fn embedding(&self, deployment: &str) -> OpenAIEmbeddingModel {
        OpenAIEmbeddingModel::from_config(
            deployment.to_string(),
            self.model_config("azure.embeddings", deployment),
        )
    }

    /// An image model for a deployment (e.g. DALL-E); `provider()` is
    /// `"azure.image"`.
    #[must_use]
    pub fn image(&self, deployment: &str) -> OpenAIImageModel {
        OpenAIImageModel::from_config(
            deployment.to_string(),
            self.model_config("azure.image", deployment),
        )
    }

    /// A transcription model; `provider()` is `"azure.transcription"`.
    /// The Azure Speech API is selected per call by `providerOptions.azure.api`
    /// and defaults to Speech for the MAI transcription model, OpenAI otherwise.
    #[must_use]
    pub fn transcription(&self, deployment: &str) -> AzureTranscriptionModel {
        let mut speech = self.model_config("azure.transcription", deployment);
        speech.url = OpenAIUrl::AzureSpeech {
            rules: self.url_rules(),
            speech_base_url: self.speech_base_url.clone(),
        };
        speech.headers = self.speech_headers.clone();
        AzureTranscriptionModel::new(
            deployment.to_string(),
            OpenAITranscriptionModel::from_config(
                deployment.to_string(),
                self.model_config("azure.transcription", deployment),
            ),
            speech,
        )
    }

    /// A speech model for a deployment; `provider()` is `"azure.speech"`.
    #[must_use]
    pub fn speech(&self, deployment: &str) -> OpenAISpeechModel {
        OpenAISpeechModel::from_config(
            deployment.to_string(),
            self.model_config("azure.speech", deployment),
        )
    }

    /// The provider as a function: the default language model for a
    /// deployment. The AI SDK's callable provider returns the Responses model,
    /// the same one as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, deployment: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(deployment))
    }
}

impl Provider for AzureOpenAIProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }

    fn speech_model(&self, model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech(model_id))))
    }
}

impl ProviderDiscovery for AzureOpenAIProvider {
    /// `GET {prefix}/deployments?api-version=...`: one exchange, no retry.
    /// Azure lists *deployments*, not models; each deployment's `id` is the
    /// model id to pass to [`Provider::language_model`].
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let mut rules = self.url_rules();
        if self.api_version.is_none() {
            rules.api_version = DEPLOYMENTS_API_VERSION.to_string();
        }
        let config = self.model_config("azure.responses", "");
        let fetch = self.fetch.clone();
        Box::pin(async move {
            // Azure response: { data: [{ id, model, modelName, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                data: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                id: String,
                #[serde(default)]
                model: Option<String>,
                #[serde(default, rename = "modelName")]
                model_name: Option<String>,
            }

            let provider_headers = config.request_headers(None).await?;
            let mut url = url::Url::parse(&format!("{}/deployments", rules.prefix()?))
                .map_err(|e| AiMuxError::InvalidArgument(format!("invalid Azure URL: {e}")))?;
            set_query_param(&mut url, "api-version", &rules.api_version);
            let url = url.to_string();
            let resp = aimux_provider_utils::get_from_api(
                HttpRequest {
                    credentialed_origin: Some(url.clone()),
                    fetch,
                    url,
                    headers: provider_headers,
                    ..Default::default()
                },
                aimux_provider_utils::create_json_response_handler(),
                crate::openai::openai_failed_response_handler(),
            )
            .await?;
            let parsed: Resp = resp.value;
            Ok(parsed
                .data
                .into_iter()
                .map(|entry| RuntimeModel {
                    id: entry.id,
                    owned_by: entry.model_name.or(entry.model),
                    created: None,
                })
                .collect())
        })
    }
}
