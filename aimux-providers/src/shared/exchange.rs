//! The private per-model configuration every Gemini-API model reads.
//!
//! This is the Rust shape of the object `createGoogle` hands each model class
//! (`{ provider, baseURL, headers, fetch, supportedUrls }`), and the Vertex
//! package hands its models the same one. The model asks it for an
//! [`Exchange`] before every request: the base URL and the request
//! headers are evaluated then, so a key, a project or a token is loaded when a
//! call is made and never when the provider is created. It has no getters for
//! the credential or the settings that produced it.

use std::collections::HashMap;

use base64::Engine as _;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, combine_headers,
    load_setting, normalize_headers, validate_base_url,
};

use super::{Credential, ProviderHeaders};

/// What one request is addressed to: the base URL and the provider headers,
/// both resolved for this request.
pub(crate) struct Endpoint {
    pub(crate) base_url: String,
    pub(crate) headers: HeaderMapOpt,
}

/// Where the base URL of a request comes from.
#[derive(Clone, Debug)]
pub(crate) enum BaseUrl {
    /// Fixed when the provider is created.
    Fixed(String),
    /// Read from the environment variable on every request and validated; an
    /// unset variable fails that request with `AiMuxError::LoadSetting`
    /// (a self-hosted instance).
    Env {
        var: &'static str,
        description: &'static str,
    },
}

impl BaseUrl {
    fn resolve(&self) -> Result<String, AiMuxError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Env { var, description } => {
                validate_base_url(&load_setting(None, var, description)?)
            }
        }
    }
}

/// Where the provider headers of a request come from.
#[derive(Clone, Debug)]
pub(crate) enum EndpointHeaders {
    /// Credential, fixed and user headers (the common case).
    Provider(ProviderHeaders),
    /// `Authorization: Basic base64(login:password)`, both parts resolved for
    /// every request, then the user's headers.
    Basic {
        login: Credential,
        password: Credential,
        user: Option<HeaderMapOpt>,
    },
    /// The provider headers, then the user's headers as the caller gave them
    /// (a value or a producer called per request).
    WithUserHeaders {
        provider: ProviderHeaders,
        user: HeadersFn,
    },
}

impl From<ProviderHeaders> for EndpointHeaders {
    fn from(headers: ProviderHeaders) -> Self {
        Self::Provider(headers)
    }
}

impl EndpointHeaders {
    async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError> {
        match self {
            Self::Provider(headers) => headers.resolve().await,
            Self::Basic {
                login,
                password,
                user,
            } => {
                let login = login.secret().await?.unwrap_or_default();
                let password = password.secret().await?.unwrap_or_default();
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{login}:{password}").as_bytes());
                let mut layer = HeaderMapOpt::new();
                layer.insert(
                    "Authorization".to_string(),
                    Some(format!("Basic {encoded}")),
                );
                Ok(match user {
                    Some(user) => combine_headers(&[&layer, user]),
                    None => layer,
                })
            }
            Self::WithUserHeaders { provider, user } => {
                let layer = provider.resolve().await?;
                Ok(combine_headers(&[&layer, &user.resolve().await?]))
            }
        }
    }
}

/// Where the endpoint (base URL and provider headers) of a request comes
/// from. Every variant is data; the work happens in [`EndpointConfig::endpoint`]
/// when a request is made.
#[derive(Clone)]
pub(crate) enum EndpointSource {
    /// A base URL and headers, each resolved on its own.
    Http {
        base_url: BaseUrl,
        headers: EndpointHeaders,
    },
    /// A Gemini-publisher model of Google Vertex AI: the mode, key, token,
    /// project and location decide the URL and the headers together.
    Vertex(crate::vertex::GeminiEndpoint),
    /// An Amazon Bedrock service: explicit URL, endpoint variables or the
    /// regional host; API key or SigV4.
    Bedrock(crate::bedrock::BedrockEndpoint),
    /// Amazon Polly: explicit URL or the regional host; SigV4.
    AwsPolly(crate::aws_polly::PollyEndpoint),
}

impl EndpointSource {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        match self {
            Self::Http { base_url, headers } => Ok(Endpoint {
                base_url: base_url.resolve()?,
                headers: headers.resolve().await?,
            }),
            Self::Vertex(endpoint) => endpoint.resolve().await,
            Self::Bedrock(endpoint) => endpoint.resolve().await,
            Self::AwsPolly(endpoint) => endpoint.resolve().await,
        }
    }
}

/// The URL patterns a model fetches itself.
#[derive(Clone, Debug, Default)]
pub(crate) enum SupportedUrlsSource {
    /// No patterns: the caller downloads every URL.
    #[default]
    None,
    /// The same patterns for every model.
    Fixed(SupportedUrls),
    /// The Gemini API's patterns, which depend on the base URL and the model
    /// id.
    Google { base_url: String },
}

impl SupportedUrlsSource {
    fn for_model(&self, model_id: &str) -> SupportedUrls {
        match self {
            Self::None => SupportedUrls::default(),
            Self::Fixed(urls) => urls.clone(),
            Self::Google { base_url } => crate::google::supported_urls(base_url, Some(model_id)),
        }
    }
}

/// What a model needs to talk to the API. Built by the provider that owns the
/// model.
#[derive(Clone)]
pub(crate) struct EndpointConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Base URL and provider headers, resolved on every request.
    pub(crate) endpoint: EndpointSource,
    /// Transport; `None` uses the process default, resolved per request.
    pub(crate) fetch: Option<FetchFunction>,
    /// URLs the model fetches itself, by model id. Empty: the caller
    /// downloads them.
    pub(crate) supported_urls: SupportedUrlsSource,
}

impl EndpointConfig {
    /// A config for a provider whose base URL is fixed when it is created and
    /// whose headers are evaluated on every request (the common case).
    pub(crate) fn fixed(
        provider: String,
        base_url: String,
        headers: impl Into<EndpointHeaders>,
        fetch: Option<FetchFunction>,
    ) -> Self {
        Self::new(
            provider,
            EndpointSource::Http {
                base_url: BaseUrl::Fixed(base_url),
                headers: headers.into(),
            },
            fetch,
        )
    }

    /// A config whose endpoint comes from `endpoint`.
    pub(crate) fn new(
        provider: String,
        endpoint: EndpointSource,
        fetch: Option<FetchFunction>,
    ) -> Self {
        Self {
            provider,
            endpoint,
            fetch,
            supported_urls: SupportedUrlsSource::None,
        }
    }

    /// The same config with the URLs the model fetches itself.
    #[must_use]
    pub(crate) fn with_supported_urls(mut self, urls: SupportedUrlsSource) -> Self {
        self.supported_urls = urls;
        self
    }

    /// The URL patterns `model_id` fetches itself.
    pub(crate) fn supported_urls(&self, model_id: &str) -> SupportedUrls {
        self.supported_urls.for_model(model_id)
    }

    /// The base URL and provider headers of one request.
    ///
    /// # Errors
    ///
    /// Returns the error of the credential, header or setting lookup.
    pub(crate) async fn endpoint(&self) -> Result<Endpoint, AiMuxError> {
        self.endpoint.resolve().await
    }

    /// Resolve the endpoint and layer the per-call headers over the provider
    /// headers (case-insensitive, later wins, `None` removes).
    ///
    /// # Errors
    ///
    /// Returns the error of the credential, header or setting producer, for
    /// instance `AiMuxError::LoadApiKey` when no key was given and the
    /// environment variable is unset.
    pub(crate) async fn exchange(
        &self,
        call_headers: Option<&HashMap<String, String>>,
    ) -> Result<Exchange, AiMuxError> {
        let endpoint = self.endpoint().await?;
        let call: HeaderMapOpt = call_headers
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Some(value.clone())))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Exchange {
            base_url: endpoint.base_url,
            headers: normalize_headers(combine_headers(&[&endpoint.headers, &call])),
            fetch: self.fetch.clone(),
        })
    }
}

/// One request's resolved address, headers and transport.
pub(crate) struct Exchange {
    base_url: String,
    headers: Vec<(String, String)>,
    fetch: Option<FetchFunction>,
}

impl Exchange {
    /// The base URL of this request, no trailing slash.
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `base_url` followed by `path` (which starts with `/`).
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// The request headers that go on the wire.
    pub(crate) fn headers(&self) -> Vec<(String, String)> {
        self.headers.clone()
    }

    /// An exchange that inherits the operation's cancellation and recording
    /// context and carries this request's headers, transport and credential
    /// origin.
    pub(crate) fn request(&self, url: String, options: &impl ExchangeContext) -> HttpRequest {
        self.with_transport(HttpRequest::new(url, self.headers(), options))
    }

    /// Attach the transport and the credential origin to a request.
    pub(crate) fn with_transport(&self, mut request: HttpRequest) -> HttpRequest {
        request.fetch = self.fetch.clone();
        request.credentialed_origin = Some(self.base_url.clone());
        request
    }
}
