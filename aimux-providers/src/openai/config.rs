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
use std::sync::Arc;

use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, Resolvable,
    combine_headers, normalize_headers,
};

use super::responses::ResponsesProfile;

pub use crate::shared::TransformRequestBody;

/// Maps an endpoint path (`"/chat/completions"`) to the full request URL. It
/// can fail because a host may only know the address when a call is made (an
/// Azure resource name read from the environment).
pub(crate) type UrlFn = Arc<dyn Fn(&str) -> Result<String, AiMuxError> + Send + Sync>;

/// What a model needs to talk to the API. Built by the provider that owns the
/// model: `OpenAIProvider`, Azure, Codex and the Hugging Face chat extension.
#[derive(Clone)]
pub(crate) struct OpenAIModelConfig {
    /// The identity the model reports from `provider()`.
    pub(crate) provider: String,
    /// Endpoint path to full URL.
    pub(crate) url: UrlFn,
    /// Provider headers (credential, organization, project, user headers),
    /// resolved on every request.
    pub(crate) headers: HeadersFn,
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
}

impl OpenAIModelConfig {
    /// A config whose URLs are `base_url` followed by the endpoint path.
    pub(crate) fn fixed(
        provider: String,
        base_url: String,
        headers: HeadersFn,
        fetch: Option<FetchFunction>,
        transform_request_body: Option<TransformRequestBody>,
    ) -> Self {
        Self {
            provider,
            url: Arc::new(move |path| Ok(format!("{base_url}{path}"))),
            headers,
            token_provider: None,
            fetch,
            supported_urls: SupportedUrls::default(),
            transform_request_body,
            responses: ResponsesProfile::default(),
        }
    }

    /// The full URL of an endpoint path.
    ///
    /// # Errors
    ///
    /// Returns the host's setting error (`AiMuxError::LoadSetting` for an
    /// unset Azure resource name).
    pub(crate) fn url(&self, path: &str) -> Result<String, AiMuxError> {
        (self.url)(path)
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
