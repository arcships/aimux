//! The private per-model configuration every OpenAI modality reads.
//!
//! This is the Rust shape of the object `createOpenAI` hands each model class
//! (`{ provider, url, headers, fetch }`): the model asks it for a URL, for the
//! request headers (evaluated per request, so a key is loaded when a call is
//! made and never when the provider is created) and for the transport. It has
//! no getters for the credential or the settings that produced it. The
//! credential origin is the one of each request's own URL, so the transport
//! refuses to send credentialed headers to any other origin after a redirect.

use std::collections::HashMap;

use serde_json::Value;

use crate::shared::ProviderHeaders;
use aimux_core::language_model::SupportedUrls;
use aimux_core::{AiMuxError, ErrorCause};
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HttpRequest, Resolvable, combine_headers,
    is_valid_hostname_part, load_setting, normalize_headers, without_trailing_slash,
};

use super::responses::ResponsesProfile;

/// The environment variable Azure reads its resource name from.
const AZURE_RESOURCE_NAME_ENV_VAR: &str = "AZURE_RESOURCE_NAME";

/// What the AI SDK's `getAzureOpenAIBaseURLInfo` derives from a base URL.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BaseUrlInfo {
    pub(crate) is_azure_openai: bool,
    pub(crate) is_foundry_project: bool,
    pub(crate) is_versioned: bool,
}

/// Everything the URL of an Azure OpenAI request depends on. Azure fills
/// the values; the rules are part of the OpenAI configuration so the OpenAI
/// models never reach into the Azure provider (RFC-0046 §3.3).
#[derive(Clone)]
pub(crate) struct UrlRules {
    pub(crate) resource_name: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) info: BaseUrlInfo,
    pub(crate) api_version: String,
    pub(crate) use_deployment_based_urls: bool,
}

impl UrlRules {
    /// The URL prefix: the base URL, or the resource's `/openai` endpoint.
    pub(crate) fn prefix(&self) -> Result<String, AiMuxError> {
        if let Some(base_url) = &self.base_url {
            return Ok(without_trailing_slash(base_url));
        }
        let resource = load_setting(
            self.resource_name.as_deref(),
            AZURE_RESOURCE_NAME_ENV_VAR,
            "resourceName",
        )?;
        // The resource name becomes part of the request host, so only a DNS
        // label is accepted (`user@internal:8080/#` would rewrite the host).
        if !is_valid_hostname_part(&resource) {
            return Err(AiMuxError::invalid_argument(
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
        let mut url = url::Url::parse(&full).map_err(|e| AiMuxError::InvalidArgument {
            message: format!("invalid Azure URL `{full}`: {e}"),
            cause: Some(ErrorCause::source(&e)),
        })?;
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
pub(crate) fn set_query_param(url: &mut url::Url, name: &str, value: &str) {
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

/// How an endpoint path (`"/chat/completions"`) becomes the full request URL.
/// Building it can fail because a host may only know the address when a call
/// is made (an Azure resource name read from the environment).
#[derive(Clone)]
pub(crate) enum OpenAIUrl {
    /// The base URL followed by the endpoint path.
    Base(String),
    /// Azure OpenAI's URL rules for a deployment.
    AzureDeployment { rules: UrlRules, deployment: String },
    /// The Azure AI Speech transcription endpoint, whatever the path: the
    /// speech base URL, or the resource's Cognitive Services host.
    AzureSpeech {
        rules: UrlRules,
        speech_base_url: Option<String>,
    },
    /// An invalid base URL setting (`OPENAI_BASE_URL`): every endpoint fails
    /// with this error, so every request reports it.
    Invalid(AiMuxError),
}

impl OpenAIUrl {
    /// The full URL of an endpoint path.
    ///
    /// # Errors
    ///
    /// Returns the host's setting error (`AiMuxError::LoadSetting` for an
    /// unset Azure resource name).
    pub(crate) fn url(&self, path: &str) -> Result<String, AiMuxError> {
        match self {
            Self::Base(base_url) => Ok(format!("{base_url}{path}")),
            Self::AzureDeployment { rules, deployment } => rules.url(path, deployment),
            Self::AzureSpeech {
                rules,
                speech_base_url,
            } => rules.speech_url(speech_base_url.as_deref()),
            Self::Invalid(error) => Err(error.clone()),
        }
    }
}

pub use crate::shared::TransformRequestBody;

/// What a model needs to talk to the API. Built by the provider that owns the
/// model: `OpenAIProvider`, Azure, Codex and the Hugging Face chat extension.
#[derive(Clone)]
pub(crate) struct OpenAIModelConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Endpoint path to full URL.
    pub(crate) url: OpenAIUrl,
    /// Provider headers (credential, organization, project, user headers),
    /// resolved on every request.
    pub(crate) headers: ProviderHeaders,
    pub(crate) token_provider: Option<Resolvable<String>>,
    /// Transport; `None` uses the process default, resolved per request.
    pub(crate) fetch: Option<FetchFunction>,
    /// URLs the model fetches itself. Empty: the caller downloads them.
    pub(crate) supported_urls: SupportedUrls,
    /// Provider-level request-body rewrite.
    pub(crate) transform_request_body: Option<TransformRequestBody>,
    /// How the Responses API model reads and writes providerOptions and file
    /// ids for this host.
    pub(crate) responses: ResponsesProfile,
    pub(crate) chat_options: crate::openai::chat::chat_language_model_options::ChatOptionsParser,
}

impl OpenAIModelConfig {
    /// A config whose URLs are `base_url` followed by the endpoint path.
    pub(crate) fn fixed(
        provider: String,
        base_url: String,
        headers: ProviderHeaders,
        fetch: Option<FetchFunction>,
        transform_request_body: Option<TransformRequestBody>,
    ) -> Self {
        Self {
            provider,
            url: OpenAIUrl::Base(base_url),
            headers,
            token_provider: None,
            fetch,
            supported_urls: SupportedUrls::default(),
            transform_request_body,
            responses: ResponsesProfile::default(),
            chat_options: crate::openai::chat::chat_language_model_options::parse_chat_options,
        }
    }

    /// The full URL of an endpoint path.
    ///
    /// # Errors
    ///
    /// Returns the host's setting error (`AiMuxError::LoadSetting` for an
    /// unset Azure resource name).
    pub(crate) fn url(&self, path: &str) -> Result<String, AiMuxError> {
        self.url.url(path)
    }

    /// The WebSocket form of an endpoint path: `https` becomes `wss` and
    /// `http` becomes `ws`, as the AI SDK's `toWebSocketUrl` does.
    ///
    /// # Errors
    ///
    /// As [`url`](Self::url).
    #[cfg(feature = "realtime")]
    pub(crate) fn ws_url(&self, path: &str) -> Result<String, AiMuxError> {
        let url = self.url(path)?;
        Ok(if let Some(rest) = url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            url
        })
    }

    /// Resolve the provider headers, layer the per-call headers over them
    /// (case-insensitive, later wins, `None` removes) and return the list that
    /// goes on the wire.
    ///
    /// # Errors
    ///
    /// Returns the error of the credential or header producer, for instance
    /// `AiMuxError::LoadApiKey` when no key was given and the environment
    /// variable is unset.
    pub(crate) async fn request_headers(
        &self,
        call_headers: Option<&HashMap<String, String>>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        let provider = self.headers.resolve().await?;
        let call: HeaderMapOpt = call_headers
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Some(value.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let mut headers = combine_headers(&[&provider, &call]);
        if let Some(token) = &self.token_provider
            && !headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("authorization") && value.is_some())
        {
            headers.insert(
                "authorization".to_string(),
                Some(format!("Bearer {}", token.resolve().await?)),
            );
        }
        Ok(normalize_headers(headers))
    }

    /// An exchange that inherits the operation's cancellation and recording
    /// context and carries this config's transport and credential origin.
    pub(crate) fn http_request(
        &self,
        url: String,
        headers: Vec<(String, String)>,
        options: &impl ExchangeContext,
    ) -> HttpRequest {
        self.with_transport(HttpRequest::new(url, headers, options))
    }

    /// Attach the transport and the credential origin (the request's own) to
    /// a request.
    pub(crate) fn with_transport(&self, mut request: HttpRequest) -> HttpRequest {
        request.fetch = self.fetch.clone();
        request.credentialed_origin = Some(request.url.clone());
        request
    }

    /// Run the provider-level body rewrite, when there is one.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}

/// Native language model URL patterns from the upstream model classes.
pub(crate) fn supported_urls(method: &str) -> SupportedUrls {
    let mut urls = HashMap::new();
    if method.ends_with("chat") || method.ends_with("responses") {
        let pattern = regex::Regex::new(r"^https?://.*$").expect("valid URL pattern");
        urls.insert("image/*".to_string(), vec![pattern.clone()]);
        if method.ends_with("responses") {
            urls.insert("application/pdf".to_string(), vec![pattern]);
        }
    }
    SupportedUrls(urls)
}
