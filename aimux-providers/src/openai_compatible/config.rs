//! The private per-model configuration of the OpenAI-compatible package.
//!
//! Like the native OpenAI config this is the Rust shape of the object
//! `createOpenAICompatible` hands each model class (`{ provider, url, headers,
//! fetch, ... }`). It has no getters for the credential or the settings that
//! produced it. Where the AI SDK passes functions (`url`, `headers`,
//! `convertUsage`, `supportedUrls`, `errorStructure`), this config holds data;
//! the shared model never compares a provider name.

use std::collections::HashMap;
use std::sync::Arc;

use aimux_core::types::Usage;
use serde_json::Value;

use aimux_core::AiMuxError;
use aimux_core::language_model::SupportedUrls;
use aimux_provider_utils::{
    ExchangeContext, FetchFunction, HeaderMapOpt, HttpRequest, ProviderErrorParts, combine_headers,
    normalize_headers,
};

use crate::preset::PresetDescriptor;
use crate::shared::ProviderHeaders;

pub use crate::shared::TransformRequestBody;

/// Where the base URL comes from.
///
/// A plain provider has a [`Fixed`](Self::Fixed) URL. A preset whose URL is
/// read from the environment or expanded from template parameters is
/// [`Preset`](Self::Preset): it is evaluated per request, so creating the
/// provider (and the preset default instances) never reads the environment
/// and never fails.
#[derive(Clone, Debug)]
pub(crate) enum BaseUrl {
    Fixed(String),
    Preset {
        descriptor: &'static PresetDescriptor,
        /// The explicit template parameters of the settings.
        params: Arc<HashMap<String, String>>,
    },
}

impl BaseUrl {
    pub(crate) fn resolve(&self) -> Result<String, AiMuxError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Preset { descriptor, params } => {
                crate::preset::resolve_base_url(descriptor, params)
            }
        }
    }
}

/// How the raw `usage` object of a chat response becomes core [`Usage`]
/// (the AI SDK's `convertUsage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConvertUsage {
    /// The generic OpenAI-shaped usage (`convertOpenAICompatibleChatUsage`).
    #[default]
    OpenAICompatible,
    /// Alibaba: cache writes from `prompt_tokens_details.cache_creation_input_tokens`
    /// (or `cache_write_tokens`), taken out of the uncached input.
    Alibaba,
    /// Moonshot AI: cache reads from the top-level `cached_tokens`.
    MoonshotAI,
}

impl ConvertUsage {
    pub(crate) fn convert(self, raw: Option<&Value>) -> Usage {
        let mut usage = super::chat::usage_from_raw(raw);
        let Some(raw) = raw.filter(|raw| !raw.is_null()) else {
            return usage;
        };
        let tokens = |value: &Value| value.as_u64().and_then(|value| u32::try_from(value).ok());
        match self {
            Self::OpenAICompatible => {}
            Self::Alibaba => {
                let details = &raw["prompt_tokens_details"];
                let cache_write = tokens(&details["cache_creation_input_tokens"])
                    .or_else(|| tokens(&details["cache_write_tokens"]))
                    .unwrap_or(0);
                usage.input_tokens.cache_write = Some(cache_write);
                usage.input_tokens.no_cache = usage
                    .input_tokens
                    .no_cache
                    .map(|tokens| tokens.saturating_sub(cache_write));
            }
            Self::MoonshotAI => {
                if let Some(cached) = tokens(&raw["cached_tokens"]) {
                    usage.input_tokens.cache_read = Some(cached);
                    usage.input_tokens.no_cache = usage
                        .input_tokens
                        .total
                        .map(|tokens| tokens.saturating_sub(cached));
                }
            }
        }
        usage
    }
}

/// `{ error: { message, type?, param?, code? } }`, the structure nearly every
/// compatible server answers errors with (the AI SDK's default
/// `errorStructure`).
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

/// The chat settings of a compatible endpoint: the AI SDK's
/// `includeUsage` / `supportsStructuredOutputs` / `supportedUrls` /
/// `convertUsage`, plus the preset capability data of RFC-0036 section 5.
#[derive(Clone, Debug)]
pub(crate) struct ChatSettings {
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
    pub supported_urls: SupportedUrls,
    pub convert_usage: ConvertUsage,
    /// Send `top_k`. The AI SDK baseline never does (it warns and drops it).
    pub supports_top_k: bool,
    /// Send `tools` / `tool_choice`.
    pub supports_tools: bool,
    /// Send `response_format`.
    pub supports_response_format: bool,
    /// The only `max-tokens` key the vendor accepts: `"max_tokens"` or
    /// `"max_completion_tokens"`.
    pub max_tokens_key: Option<&'static str>,
    /// Streaming usage rides in `chunk[key].usage` instead of `chunk.usage`.
    pub stream_usage_key: Option<String>,
}

impl Default for ChatSettings {
    /// The AI SDK baseline.
    fn default() -> Self {
        Self {
            include_usage: false,
            supports_structured_outputs: false,
            supported_urls: SupportedUrls::default(),
            convert_usage: ConvertUsage::default(),
            supports_top_k: false,
            supports_tools: true,
            supports_response_format: true,
            max_tokens_key: None,
            stream_usage_key: None,
        }
    }
}

/// What a model needs to talk to the API.
#[derive(Clone)]
pub(crate) struct CompatModelConfig {
    /// The identity the model reports: `"{name}.{method}"`.
    pub provider: String,
    pub base_url: BaseUrl,
    /// Appended to every request URL, in this order.
    pub query_params: Option<Arc<Vec<(String, String)>>>,
    /// Provider headers (credential, user headers), produced on every request.
    pub headers: ProviderHeaders,
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

    /// The failed-response handler of the compatible error structure.
    pub(crate) fn failed_response_handler(
        &self,
    ) -> aimux_provider_utils::ResponseHandler<AiMuxError> {
        aimux_provider_utils::create_json_error_response_handler(default_error_structure)
    }
}
