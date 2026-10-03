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
use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, combine_headers,
    normalize_headers,
};

use super::TransformRequestBody;

/// What one request is addressed to: the base URL and the provider headers,
/// both resolved for this request.
pub(crate) struct Endpoint {
    pub(crate) base_url: String,
    pub(crate) headers: HeaderMapOpt,
}

/// Resolves the endpoint of one request.
pub(crate) type EndpointFn =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Endpoint, AiMuxError>> + Send + Sync>;

/// The URL patterns a model fetches itself, for a model id.
pub(crate) type SupportedUrlsFn = Arc<dyn Fn(&str) -> SupportedUrls + Send + Sync>;

/// What a model needs to talk to the API. Built by the provider that owns the
/// model.
#[derive(Clone)]
pub(crate) struct EndpointConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Base URL and provider headers, resolved on every request.
    pub(crate) endpoint: EndpointFn,
    /// Transport; `None` uses the process default, resolved per request.
    pub(crate) fetch: Option<FetchFunction>,
    /// URLs the model fetches itself, by model id. Empty: the caller
    /// downloads them.
    pub(crate) supported_urls: SupportedUrlsFn,
    /// Provider-level request-body rewrite.
    pub(crate) transform_request_body: Option<TransformRequestBody>,
}

impl EndpointConfig {
    /// A config for a provider whose base URL is fixed when it is created and
    /// whose headers are evaluated on every request (the common case).
    pub(crate) fn fixed(
        provider: String,
        base_url: String,
        headers: HeadersFn,
        fetch: Option<FetchFunction>,
        transform_request_body: Option<TransformRequestBody>,
    ) -> Self {
        Self {
            provider,
            endpoint: Arc::new(move || {
                let base_url = base_url.clone();
                let headers = headers.clone();
                Box::pin(async move {
                    Ok(Endpoint {
                        base_url,
                        headers: headers.resolve().await?,
                    })
                })
            }),
            fetch,
            supported_urls: Arc::new(|_| SupportedUrls::default()),
            transform_request_body,
        }
    }

    /// The same config with the URLs the model fetches itself.
    #[must_use]
    pub(crate) fn with_supported_urls(mut self, urls: SupportedUrlsFn) -> Self {
        self.supported_urls = urls;
        self
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
        let endpoint = (self.endpoint)().await?;
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
            transform_request_body: self.transform_request_body.clone(),
        })
    }
}

/// One request's resolved address, headers and transport.
pub(crate) struct Exchange {
    base_url: String,
    headers: Vec<(String, String)>,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
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

    /// Run the provider-level body rewrite, when there is one.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}
