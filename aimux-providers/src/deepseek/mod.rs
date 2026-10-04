//! DeepSeek provider.
//!
//! [`create_deepseek`] is the Rust form of the AI SDK's `createDeepSeek`: a
//! provider of [`DeepSeekChatLanguageModel`], the package's own chat model
//! (`@ai-sdk/deepseek` does not build on the OpenAI-compatible package). Its
//! models report `"deepseek.chat"` and read providerOptions and report provider
//! metadata under `deepseek`.
//!
//! As in the other packages the API key is not read when the provider is
//! created: `DEEPSEEK_API_KEY` is loaded on every request unless `api_key` is
//! given. [`deepseek()`] is the default instance.

mod convert;
mod finish_reason;
mod is_v4_model;
mod model;
mod options;
mod prepare_tools;
mod types;
mod usage;

pub use model::DeepSeekChatLanguageModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::types::Usage;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::openai_compatible::ChatProfile;
use crate::openai_compatible::chat::baseline_usage;
use crate::openai_compatible::config::ChatDialect;
use crate::shared::{Credential, EndpointConfig, TransformRequestBody, provider_headers};
use model::DeepSeekChatConfig;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const API_KEY_ENV_VAR: &str = "DEEPSEEK_API_KEY";

/// DeepSeek's chat behavior as data for the shared compatible chat model. The
/// DeepSeek package no longer uses it; only the registry presets of the
/// `deepseek` family do.
pub(crate) fn profile() -> ChatProfile {
    let mut dialect = ChatDialect::baseline();
    dialect.supports_top_k = true;
    dialect.max_tokens_key = Some("max_tokens");
    dialect.convert_usage = Some(Arc::new(convert_usage));
    ChatProfile {
        include_usage: true,
        supports_structured_outputs: true,
        supports_multi_part_tool_content: false,
        dialect,
    }
}

/// OpenAI-shaped usage, with DeepSeek's prompt-cache fields as the cache split.
fn convert_usage(raw: &Value) -> Usage {
    let mut usage = baseline_usage(raw);
    if let Some(hit) = raw.get("prompt_cache_hit_tokens").and_then(Value::as_u64) {
        let hit = hit as u32;
        let prompt = usage.input_tokens.total.unwrap_or(0);
        let miss = raw
            .get("prompt_cache_miss_tokens")
            .and_then(Value::as_u64)
            .map_or_else(|| prompt.saturating_sub(hit), |miss| miss as u32);
        usage.input_tokens.cache_read = Some(hit);
        usage.input_tokens.no_cache = Some(miss);
    }
    usage
}

/// The URL patterns the chat model fetches itself (`supportedUrls` of the AI
/// SDK's `DeepSeekChatLanguageModel`): `http(s)` images.
fn chat_supported_urls() -> SupportedUrls {
    let http = regex::Regex::new(r"^https?://.*$").expect("static pattern");
    SupportedUrls(std::iter::once(("image/*".to_string(), vec![http])).collect())
}

/// Settings of [`create_deepseek`] (the AI SDK's `DeepSeekProviderSettings`).
#[derive(Clone, Default)]
pub struct DeepSeekProviderSettings {
    /// Base URL for the API calls. Default `https://api.deepseek.com`; a
    /// trailing slash is removed.
    pub base_url: Option<String>,
    /// The API key. `None` loads `DEEPSEEK_API_KEY` when a request is made and
    /// fails that request with `AiMuxError::LoadApiKey` if it is unset. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers on every request; a `None` value removes the header.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the prefix of every model's `provider()` string and
    /// the providerOptions namespace. Default `"deepseek"`.
    pub name: Option<String>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for DeepSeekProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepSeekProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// Create a DeepSeek provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host or `name` is empty / contains `.`. The key is not read
/// here.
pub fn create_deepseek(settings: DeepSeekProviderSettings) -> Result<DeepSeekProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(DeepSeekProvider {
        name: settings.name.unwrap_or_else(|| "deepseek".to_string()),
        base_url,
        headers: provider_headers(
            Credential::explicit_or_env(settings.api_key, API_KEY_ENV_VAR, "DeepSeek"),
            Vec::new(),
            settings.headers,
        ),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_deepseek` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail.
pub fn deepseek() -> &'static DeepSeekProvider {
    static DEFAULT: OnceLock<DeepSeekProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_deepseek(DeepSeekProviderSettings::default())
            .expect("default DeepSeek settings are always valid")
    })
}

/// A DeepSeek provider (the AI SDK's `DeepSeekProvider`). Language models
/// only: embedding and image models are `NoSuchModel`.
pub struct DeepSeekProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl DeepSeekProvider {
    fn endpoint_config(&self, method: &str) -> EndpointConfig {
        EndpointConfig::fixed(
            format!("{}.{method}", self.name),
            self.base_url.clone(),
            self.headers.clone(),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// A chat model; `provider()` is `"{name}.chat"` (`"deepseek.chat"`).
    #[must_use]
    pub fn chat(&self, model_id: &str) -> DeepSeekChatLanguageModel {
        DeepSeekChatLanguageModel::from_config(
            model_id.to_string(),
            DeepSeekChatConfig {
                endpoint: self
                    .endpoint_config("chat")
                    .with_supported_urls(Arc::new(|_| chat_supported_urls())),
                supports_assistant_prefix_completion: self.base_url.ends_with("/beta"),
                supports_strict_tool_calls: self.base_url.ends_with("/beta"),
            },
        )
    }

    /// The provider as a function: the default language model, the chat model.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for DeepSeekProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

impl ProviderDiscovery for DeepSeekProvider {
    /// `GET {base_url}/models`: one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.endpoint_config("models");
        Box::pin(async move {
            crate::shared::list_data_models(
                &config,
                aimux_provider_utils::create_standard_json_error_response_handler(),
            )
            .await
        })
    }
}
