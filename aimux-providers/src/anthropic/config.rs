//! The private per-model configuration every host of the Anthropic Messages
//! API shares.
//!
//! This is the Rust shape of the object `createAnthropic` hands
//! `AnthropicLanguageModel` (`{ provider, baseURL, headers, fetch,
//! supportedUrls, ... }`). The first-party provider, Claude Platform on AWS and
//! Anthropic on Vertex each build one; the model reads nothing else, so what
//! differs between the hosts (endpoint, credentials, a signing transport, a
//! body envelope) is data here, never a branch in the model. It has no getters
//! for the credential or the settings that produced it.

use indexmap::IndexSet;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{Credential, ProviderHeaders};
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HttpRequest, ResponseHandler, combine_headers,
    normalize_headers,
};

use super::language_model::RequestProfile;
pub use crate::shared::TransformRequestBody;
use crate::shared::{Endpoint, ResolveEndpoint};

/// Where the messages call of a model goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MessagesUrl {
    /// `url("/messages")`: the first-party API and hosts that mirror it.
    #[default]
    Messages,
    /// `{base_url}/{model id}:rawPredict`, or `:streamRawPredict` when
    /// streaming: Anthropic on Vertex AI.
    RawPredict,
}

/// Where a model's endpoint (base URL and provider headers) comes from.
#[derive(Clone, Default)]
pub(crate) enum AnthropicEndpoint {
    /// The config's own `base_url` and `headers`.
    #[default]
    Fixed,
    /// An endpoint the host resolves for every request (Anthropic on Vertex
    /// AI, Claude Platform on AWS): the project, location, region or
    /// credentials are only known when a call is made. The config's own
    /// `base_url` and `headers` then only describe the model's static
    /// identity. The host implements [`ResolveEndpoint`], so this crate's
    /// Anthropic code never names the hosts (RFC-0046 §3.3).
    Resolved(Arc<dyn ResolveEndpoint>),
    /// An invalid base URL setting (`ANTHROPIC_BASE_URL`): every request
    /// fails with this error before it is sent.
    Invalid(AiMuxError),
}

/// Builds the host's error handler for non-2xx responses.
pub(crate) type ErrorHandlerFn = fn() -> ResponseHandler<AiMuxError>;

/// How a host differs from the first-party Messages API (the upstream
/// `buildRequestUrl`, `transformRequestBody`, `supportsNativeStructuredOutput`
/// and `supportsStrictTools` config members, plus its error shape).
#[derive(Clone)]
pub(crate) struct AnthropicModelHooks {
    /// Where the messages call goes.
    pub(crate) messages_url: MessagesUrl,
    /// Applied to the finished request body before the provider's own
    /// `transform_request_body`.
    pub(crate) prepare_body: Option<fn(Value) -> Value>,
    /// Parses a failed response.
    pub(crate) failed_response_handler: ErrorHandlerFn,
    /// Structured outputs and their beta header.
    pub(crate) supports_native_structured_output: bool,
    /// `strict` on tool definitions.
    pub(crate) supports_strict_tools: bool,
}

impl Default for AnthropicModelHooks {
    /// The first-party API.
    fn default() -> Self {
        Self {
            messages_url: MessagesUrl::Messages,
            prepare_body: None,
            failed_response_handler: super::error::anthropic_failed_response_handler,
            supports_native_structured_output: true,
            supports_strict_tools: true,
        }
    }
}

/// What a model needs to talk to the API. Built by the provider that owns the
/// model; read by [`AnthropicMessagesModel`](super::AnthropicMessagesModel)
/// and the files interface.
#[derive(Clone)]
pub(crate) struct AnthropicModelConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Provider headers (credential, `anthropic-version`, user headers),
    /// resolved on every request.
    pub(crate) headers: ProviderHeaders,
    /// Transport; `None` uses the process default, resolved per request.
    pub(crate) fetch: Option<FetchFunction>,
    /// URLs the model fetches itself. Empty: the caller downloads them.
    pub(crate) supported_urls: SupportedUrls,
    /// Provider-level request-body rewrite.
    pub(crate) transform_request_body: Option<TransformRequestBody>,
    /// The URL every endpoint path is appended to; also the origin
    /// credentialed headers may be sent to.
    pub(crate) base_url: String,
    /// The providerOptions key read in addition to `anthropic` and written
    /// on response metadata.
    pub(crate) provider_options_name: String,
    /// Host differences.
    pub(crate) hooks: AnthropicModelHooks,
    /// Where `base_url` and `headers` of a request come from.
    pub(crate) endpoint: AnthropicEndpoint,
}

impl AnthropicModelConfig {
    /// The configuration to use for one request: itself, or itself with the
    /// endpoint the host resolves now.
    ///
    /// # Errors
    ///
    /// Returns the error of the host's setting or credential loading, for
    /// instance `AiMuxError::LoadSetting` for an unset Vertex project.
    pub(crate) async fn resolved(&self) -> Result<Self, AiMuxError> {
        match &self.endpoint {
            AnthropicEndpoint::Fixed => Ok(self.clone()),
            AnthropicEndpoint::Invalid(error) => Err(error.clone()),
            AnthropicEndpoint::Resolved(endpoint) => {
                let Endpoint { base_url, headers } = endpoint.resolve().await?;
                Ok(Self {
                    base_url,
                    headers: ProviderHeaders::bearer(Credential::None, Vec::new(), Some(headers)),
                    endpoint: AnthropicEndpoint::Fixed,
                    ..self.clone()
                })
            }
        }
    }

    /// The full URL of an endpoint path: `base_url` followed by `path`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// The URL of the messages call for a model.
    pub(crate) fn messages_url(&self, model_id: &str, stream: bool) -> String {
        match self.hooks.messages_url {
            MessagesUrl::Messages => self.url("/messages"),
            MessagesUrl::RawPredict => {
                let method = if stream {
                    "streamRawPredict"
                } else {
                    "rawPredict"
                };
                format!("{}/{model_id}:{method}", self.base_url)
            }
        }
    }

    /// What the request body builder needs to know about this host.
    pub(crate) fn request_profile(&self) -> RequestProfile {
        RequestProfile {
            options_name: self.provider_options_name.clone(),
            supports_native_structured_output: self.hooks.supports_native_structured_output,
            supports_strict_tools: self.hooks.supports_strict_tools,
        }
    }

    /// The host's error handler.
    pub(crate) fn failed_response_handler(&self) -> ResponseHandler<AiMuxError> {
        (self.hooks.failed_response_handler)()
    }

    /// Resolve the provider headers, add the `anthropic-beta` header and
    /// return the list that goes on the wire. The header joins, in the AI
    /// SDK's insertion order and without duplicates: `betas`, the
    /// lower-cased values of the provider then the per-call `anthropic-beta`
    /// headers (case-insensitive name), and `option_betas`
    /// (`providerOptions.anthropic.anthropicBeta`).
    ///
    /// # Errors
    ///
    /// Returns the error of the credential or header producer, for instance
    /// `AiMuxError::LoadApiKey` when no key was given and the environment
    /// variable is unset.
    pub(crate) async fn request_headers(
        &self,
        call_headers: Option<&HashMap<String, String>>,
        betas: &IndexSet<String>,
        option_betas: &[String],
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
        let mut merged_betas = betas.clone();
        for headers in [&provider, &call] {
            for (name, value) in headers {
                if name.eq_ignore_ascii_case("anthropic-beta") {
                    for beta in value.as_deref().unwrap_or_default().split(',') {
                        let beta = beta.trim().to_ascii_lowercase();
                        if !beta.is_empty() {
                            merged_betas.insert(beta);
                        }
                    }
                }
            }
        }
        merged_betas.extend(option_betas.iter().cloned());
        let mut beta_layer = HeaderMapOpt::new();
        if !merged_betas.is_empty() {
            beta_layer.insert(
                "anthropic-beta".to_string(),
                Some(merged_betas.into_iter().collect::<Vec<_>>().join(",")),
            );
        }
        Ok(normalize_headers(combine_headers(&[
            &provider,
            &call,
            &beta_layer,
        ])))
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

    /// Attach the transport and the credential origin to a request.
    pub(crate) fn with_transport(&self, mut request: HttpRequest) -> HttpRequest {
        request.fetch = self.fetch.clone();
        request.credentialed_origin = Some(self.base_url.clone());
        request
    }

    /// Run the host's body preparation, then the provider-level rewrite.
    pub(crate) fn transform_body(&self, mut body: Value) -> Value {
        if let Some(prepare) = self.hooks.prepare_body {
            body = prepare(body);
        }
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}
