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

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, ResponseHandler,
    combine_headers, normalize_headers,
};

use super::convert::RequestProfile;
pub use crate::shared::TransformRequestBody;

/// Maps an endpoint path (`"/messages"`) to the full request URL.
pub(crate) type UrlFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// `(model id, streaming)` to the full URL of the messages call, for hosts
/// that put the model and the mode in the path.
pub(crate) type RequestUrlFn = Arc<dyn Fn(&str, bool) -> String + Send + Sync>;

/// Rewrites the request body a host-specific way.
pub(crate) type BodyFn = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// Builds the host's error handler for non-2xx responses.
pub(crate) type ErrorHandlerFn = fn() -> ResponseHandler<AiMuxError>;

/// How a host differs from the first-party Messages API (the upstream
/// `buildRequestUrl`, `transformRequestBody`, `supportsNativeStructuredOutput`
/// and `supportsStrictTools` config members, plus its error shape).
#[derive(Clone)]
pub(crate) struct AnthropicModelHooks {
    /// The messages URL when it depends on the model and the mode. `None`:
    /// `url("/messages")`.
    pub(crate) request_url: Option<RequestUrlFn>,
    /// Applied to the finished request body before the provider's own
    /// `transform_request_body`.
    pub(crate) prepare_body: Option<BodyFn>,
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
            request_url: None,
            prepare_body: None,
            failed_response_handler: super::anthropic_failed_response_handler,
            supports_native_structured_output: true,
            supports_strict_tools: true,
        }
    }
}

/// What a model needs to talk to the API. Built by the provider that owns the
/// model; read by [`AnthropicMessagesModel`](super::model::AnthropicMessagesModel)
/// and the files interface.
#[derive(Clone)]
pub(crate) struct AnthropicModelConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Endpoint path to full URL.
    pub(crate) url: UrlFn,
    /// Provider headers (credential, `anthropic-version`, user headers),
    /// resolved on every request.
    pub(crate) headers: HeadersFn,
    /// Transport; `None` uses the process default, resolved per request.
    pub(crate) fetch: Option<FetchFunction>,
    /// URLs the model fetches itself. Empty: the caller downloads them.
    pub(crate) supported_urls: SupportedUrls,
    /// Provider-level request-body rewrite.
    pub(crate) transform_request_body: Option<TransformRequestBody>,
    /// The origin credentialed headers may be sent to. Never read for URLs.
    pub(crate) base_url: String,
    /// The providerOptions key read in addition to `anthropic` and written
    /// on response metadata.
    pub(crate) provider_options_name: String,
    /// Host differences.
    pub(crate) hooks: AnthropicModelHooks,
}

impl AnthropicModelConfig {
    /// The full URL of an endpoint path.
    pub(crate) fn url(&self, path: &str) -> String {
        (self.url)(path)
    }

    /// The URL of the messages call for a model.
    pub(crate) fn messages_url(&self, model_id: &str, stream: bool) -> String {
        match &self.hooks.request_url {
            Some(request_url) => request_url(model_id, stream),
            None => self.url("/messages"),
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

    /// Resolve the provider headers, add the `anthropic-beta` header for
    /// `betas`, layer the per-call headers over both (case-insensitive, later
    /// wins, `None` removes) and return the list that goes on the wire.
    ///
    /// # Errors
    ///
    /// Returns the error of the credential or header producer, for instance
    /// `AiMuxError::LoadApiKey` when no key was given and the environment
    /// variable is unset.
    pub(crate) async fn request_headers(
        &self,
        call_headers: Option<&HashMap<String, String>>,
        betas: &BTreeSet<String>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        let provider = self.headers.resolve().await?;
        let mut beta_layer = HeaderMapOpt::new();
        if !betas.is_empty() {
            beta_layer.insert(
                "anthropic-beta".to_string(),
                Some(betas.iter().cloned().collect::<Vec<_>>().join(",")),
            );
        }
        let call: HeaderMapOpt = call_headers
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), Some(value.clone())))
                    .collect()
            })
            .unwrap_or_default();
        Ok(normalize_headers(combine_headers(&[
            &provider,
            &beta_layer,
            &call,
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
        if let Some(prepare) = &self.hooks.prepare_body {
            body = prepare(body);
        }
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}
