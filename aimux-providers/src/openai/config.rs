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

pub use crate::shared::TransformRequestBody;

/// Maps an endpoint path (`"/chat/completions"`) to the full request URL.
pub(crate) type UrlFn = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// What a model needs to talk to the API. Built by `OpenAIProvider`; the
/// providers that have not moved to their own package yet build it with
/// [`StaticBearerConfig`].
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

/// A fixed bearer credential, base URL and provider name: the transitional
/// configuration of the providers that reuse the OpenAI models but have not
/// moved to their own settings yet (Codex, xAI, Hugging Face). Each of them
/// is replaced by its package's factory in the native-package groups; nothing
/// new should use this.
#[derive(Clone)]
pub(crate) struct StaticBearerConfig {
    provider: String,
    secret: String,
    origin: String,
    extra_headers: Option<HashMap<String, String>>,
}

impl std::fmt::Debug for StaticBearerConfig {
    /// Never prints the credential or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticBearerConfig")
            .field("provider", &self.provider)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl StaticBearerConfig {
    pub(crate) fn new(provider: &str, secret: impl Into<String>, origin: impl AsRef<str>) -> Self {
        Self {
            provider: provider.to_string(),
            secret: secret.into(),
            origin: aimux_provider_utils::without_trailing_slash(origin.as_ref()),
            extra_headers: None,
        }
    }

    #[must_use]
    pub(crate) fn with_origin(mut self, origin: impl AsRef<str>) -> Self {
        self.origin = aimux_provider_utils::without_trailing_slash(origin.as_ref());
        self
    }

    #[must_use]
    pub(crate) fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = Some(headers);
        self
    }

    /// The base URL requests are sent to.
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    /// The credential, for the providers that build their own requests.
    pub(crate) fn secret(&self) -> &str {
        &self.secret
    }

    /// The model configuration for `method` (`"chat"`, `"responses"`,
    /// `"models"`, ...): provider `"{provider}.{method}"`, headers
    /// `Authorization: Bearer <secret>` then the extra headers.
    pub(crate) fn model_config(&self, method: &str) -> OpenAIModelConfig {
        let mut fixed = HeaderMapOpt::new();
        fixed.insert(
            "Authorization".to_string(),
            Some(format!("Bearer {}", self.secret)),
        );
        let headers = match &self.extra_headers {
            Some(extra) => {
                let extra: HeaderMapOpt = extra
                    .iter()
                    .map(|(k, v)| (k.clone(), Some(v.clone())))
                    .collect();
                combine_headers(&[&fixed, &extra])
            }
            None => fixed,
        };
        let base = self.origin.clone();
        OpenAIModelConfig {
            provider: format!("{}.{method}", self.provider),
            url: Arc::new(move |path| format!("{base}{path}")),
            headers: aimux_provider_utils::Resolvable::Value(headers),
            fetch: None,
            supported_urls: SupportedUrls::default(),
            transform_request_body: None,
            base_url: self.origin.clone(),
        }
    }
}
