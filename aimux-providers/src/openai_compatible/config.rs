//! The private per-model configuration of the OpenAI-compatible package.
//!
//! Like the native OpenAI config this is the Rust shape of the object
//! `createOpenAICompatible` hands each model class (`{ provider, url, headers,
//! fetch, ... }`). It has no getters for the credential or the settings that
//! produced it. Compatible endpoint behavior is data (flags and a usage-key
//! name); the shared model never compares a provider name.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::types::{ProviderMetadata, Usage};
use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HeadersFn, HttpRequest, ProviderErrorParts,
    combine_headers, normalize_headers,
};

pub use crate::shared::TransformRequestBody;

/// Converts the raw optional token usage of a chat response.
pub type ConvertUsage = Arc<dyn Fn(Option<&Value>) -> Usage + Send + Sync>;

/// Where the base URL comes from.
///
/// A plain provider has a [`Fixed`](Self::Fixed) URL. A preset whose URL is
/// read from the environment or expanded from template parameters is
/// [`Lazy`](Self::Lazy): it is evaluated per request, so creating the provider
/// (and the preset default instances) never reads the environment and never
/// fails.
#[derive(Clone)]
pub(crate) enum BaseUrl {
    Fixed(String),
    Lazy(Arc<dyn Fn() -> Result<String, AiMuxError> + Send + Sync>),
}

impl BaseUrl {
    pub(crate) fn resolve(&self) -> Result<String, AiMuxError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Lazy(produce) => produce(),
        }
    }
}

/// Maps a provider error payload to the message and code of the API error.
pub(crate) type ErrorStructure = Arc<dyn Fn(&Value) -> ProviderErrorParts + Send + Sync>;

/// Captures vendor-specific metadata from responses (the AI SDK's
/// `MetadataExtractor`). The returned map is merged into `provider_metadata`
/// next to the namespace entry.
pub trait MetadataExtractor: Send + Sync {
    /// Metadata of a non-streaming response body.
    fn extract_metadata(&self, parsed_body: &Value) -> Option<ProviderMetadata>;

    /// A fresh extractor for one streaming response.
    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor>;
}

/// The per-stream half of [`MetadataExtractor`].
pub trait StreamMetadataExtractor: Send {
    /// Called with every parsed chunk (including the one carrying usage).
    fn process_chunk(&mut self, parsed_chunk: &Value);

    /// Metadata merged into the finish part's `provider_metadata`.
    fn build_metadata(&self) -> Option<ProviderMetadata>;
}

/// Everything that distinguishes one compatible vendor's chat endpoint from
/// the baseline, other than the public provider settings.
#[derive(Clone)]
pub(crate) struct ChatDialect {
    /// Send `top_k`. The AI SDK baseline never does (it warns and drops it).
    pub supports_top_k: bool,
    /// Send `tools` / `tool_choice`.
    pub supports_tools: bool,
    /// Send `response_format`.
    pub supports_response_format: bool,
    /// The only `max-tokens` key the vendor accepts: `"max_tokens"` or
    /// `"max_completion_tokens"`. Preset capability data (RFC-0036 section 5).
    pub max_tokens_key: Option<&'static str>,
    /// Streaming usage rides in `chunk[key].usage` instead of `chunk.usage`.
    pub stream_usage_key: Option<String>,
    pub metadata_extractor: Option<Arc<dyn MetadataExtractor>>,
    pub supported_urls: Option<Arc<dyn Fn() -> SupportedUrls + Send + Sync>>,
    pub convert_usage: Option<ConvertUsage>,
    pub error_structure: ErrorStructure,
}

impl ChatDialect {
    /// The AI SDK baseline.
    pub(crate) fn baseline() -> Self {
        Self {
            supports_top_k: false,
            supports_tools: true,
            supports_response_format: true,
            max_tokens_key: None,
            stream_usage_key: None,
            metadata_extractor: None,
            supported_urls: None,
            convert_usage: None,
            error_structure: Arc::new(default_error_structure),
        }
    }
}

/// `{ error: { message, type?, param?, code? } }`, the structure nearly every
/// compatible server answers errors with.
pub(crate) fn default_error_structure(data: &Value) -> ProviderErrorParts {
    let error = data.get("error").unwrap_or(data);
    ProviderErrorParts {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        provider_code: error.get("code").or_else(|| error.get("type")).and_then(
            |value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            },
        ),
    }
}

/// Chat-only settings of a provider.
#[derive(Clone)]
pub(crate) struct ChatSettings {
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
    pub supports_multi_part_tool_content: bool,
    pub supported_urls: SupportedUrls,
    pub dialect: Arc<ChatDialect>,
}

/// What a model needs to talk to the API.
#[derive(Clone)]
pub(crate) struct CompatModelConfig {
    /// The identity the model reports: `"{name}.{method}"`.
    pub provider: String,
    pub base_url: BaseUrl,
    /// Appended to every request URL, in this order.
    pub query_params: Option<Arc<Vec<(String, String)>>>,
    /// Provider headers (credential, user headers), resolved on every request.
    pub headers: HeadersFn,
    /// Transport; `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    pub transform_request_body: Option<TransformRequestBody>,
    pub chat: ChatSettings,
}

impl CompatModelConfig {
    /// The providerOptions namespace of this provider: the first segment of
    /// the provider string.
    pub(crate) fn provider_options_name(&self) -> &str {
        self.provider.split('.').next().unwrap_or_default().trim()
    }

    /// The full URL of an endpoint path (with the query parameters) and the
    /// origin credentialed headers may be sent to.
    pub(crate) fn url(&self, path: &str) -> Result<(String, String), AiMuxError> {
        let base = self.base_url.resolve()?;
        let mut url = url::Url::parse(&format!("{base}{path}")).map_err(|e| {
            AiMuxError::InvalidArgument(format!("invalid request URL for {}: {e}", self.provider))
        })?;
        if let Some(params) = &self.query_params {
            // As the AI SDK does: the configured parameters replace whatever
            // query the base URL carried.
            url.set_query(None);
            if !params.is_empty() {
                url.query_pairs_mut()
                    .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            }
        }
        Ok((url.to_string(), base))
    }

    /// Resolve the provider headers, layer the per-call headers over them
    /// (case-insensitive, later wins, `None` removes) and return the list that
    /// goes on the wire.
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

    /// An exchange for `path` that inherits the operation's cancellation and
    /// recording context and carries this config's transport and credential
    /// origin.
    pub(crate) fn http_request(
        &self,
        path: &str,
        headers: Vec<(String, String)>,
        options: &impl ExchangeContext,
    ) -> Result<HttpRequest, AiMuxError> {
        let (url, origin) = self.url(path)?;
        Ok(self.with_transport(HttpRequest::new(url, headers, options), origin))
    }

    /// Attach the transport and the credential origin to a request.
    pub(crate) fn with_transport(&self, mut request: HttpRequest, origin: String) -> HttpRequest {
        request.fetch = self.fetch.clone();
        request.credentialed_origin = Some(origin);
        request
    }

    /// Run the provider-level body rewrite, when there is one.
    pub(crate) fn transform_body(&self, body: Value) -> Value {
        match &self.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }

    /// The failed-response handler of this provider's error structure.
    pub(crate) fn failed_response_handler(
        &self,
    ) -> aimux_provider_utils::ResponseHandler<AiMuxError> {
        let structure = self.chat.dialect.error_structure.clone();
        aimux_provider_utils::create_json_error_response_handler(move |data| structure(data))
    }
}
