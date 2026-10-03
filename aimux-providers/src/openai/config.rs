//! The private per-model configuration every OpenAI modality reads.
//!
//! This is the Rust shape of the object `createOpenAI` hands each model class
//! (`{ provider, url, headers, fetch }`): the model asks it for a URL, for the
//! request headers (evaluated per request, so a key is loaded when a call is
//! made and never when the provider is created) and for the transport. It has
//! no getters for the credential or the settings that produced it; the one
//! field that names an origin, `base_url`, exists so the transport can refuse
//! to send credentialed headers anywhere else.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, combine_headers,
    normalize_headers,
};

use super::OpenAICompatProfile;

/// A provider-level rewrite of every JSON request body, called once after the
/// body is serialized and before it is sent.
pub type TransformRequestBody = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// Maps an endpoint path (`"/chat/completions"`) to the full request URL.
pub(crate) type UrlFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// What a model needs to talk to the API. Built by `OpenAIProvider` for the
/// native package and by `OpenAIConfig::into_model_config` for the
/// OpenAI-compatible consumers that are still configured through the builder.
#[derive(Clone)]
pub(crate) struct OpenAIModelConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Endpoint path to full URL.
    pub(crate) url: UrlFn,
    /// Provider headers (credential, organization, project, user headers),
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
    /// Wire-format differences of OpenAI-compatible vendors. Temporary: the
    /// compat package takes it over with the preset generator.
    pub(crate) profile: OpenAICompatProfile,
}

impl OpenAIModelConfig {
    /// The full URL of an endpoint path.
    pub(crate) fn url(&self, path: &str) -> String {
        (self.url)(path)
    }

    /// The WebSocket form of an endpoint path: `https` becomes `wss` and
    /// `http` becomes `ws`, as the AI SDK's `toWebSocketUrl` does.
    pub(crate) fn ws_url(&self, path: &str) -> String {
        let url = self.url(path);
        if let Some(rest) = url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            url
        }
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
        Ok(normalize_headers(combine_headers(&[&provider, &call])))
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

    /// Run the provider-level body rewrite, when there is one.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}
