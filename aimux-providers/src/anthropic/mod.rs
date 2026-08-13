//! Anthropic (Claude) provider.

pub mod cache_control;
pub mod convert;
pub mod files;
pub mod model;
pub mod prepare_tools;
pub mod sanitize_json_schema;
pub mod stream;
pub mod tool_name_mapping;
pub mod types;
pub mod usage;

use std::collections::HashMap;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{RetryConfig, load_api_key};
use serde_json::Value;

/// The bare (unversioned) Anthropic API URL.
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";

/// Configuration for the Anthropic provider.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    /// API key sent via the `x-api-key` header. Empty when authenticating with
    /// an `auth_token` instead.
    pub api_key: String,
    /// Bearer token sent via the `Authorization` header. When set, `api_key`
    /// is ignored.
    pub auth_token: Option<String>,
    /// Unversioned API root (no trailing slash, no `/v1` suffix). The endpoint
    /// is built as `{base_url}/v1/messages`.
    pub base_url: String,
    /// `anthropic-version` header value.
    pub api_version: String,
    /// Provider name surfaced by `LanguageModel::provider` (TS default
    /// `anthropic.messages`).
    pub name: String,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// 重试配置。默认 `RetryConfig::default()`（max_retries=2）。
    pub retry_config: RetryConfig,
    /// Provider 级请求体覆盖（RFC-0017）。在标准请求体之后 deep-merge。
    pub body_overrides: Option<Value>,
    /// api_key 来源(RFC-0023 `ProviderRecord.api_key_source`):`None` = 显式
    /// (config_snapshot 记为 "explicit");`Some("env:VAR")` = 来自环境变量;
    /// `Some("none")` = 本地/匿名占位。不存明文之外的信息。
    pub api_key_source: Option<String>,
}

impl AnthropicConfig {
    /// Create a config authenticated with an API key, using the default
    /// Anthropic base URL.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            auth_token: None,
            base_url: ANTHROPIC_API_URL.to_string(),
            api_version: "2023-06-01".to_string(),
            name: "anthropic.messages".to_string(),
            headers: None,
            retry_config: RetryConfig::default(),
            body_overrides: None,
            api_key_source: None,
        }
    }

    /// Start a builder for more involved configurations (e.g. `auth_token`,
    /// custom `name`, extra `headers`). The builder validates that `api_key`
    /// and `auth_token` are not both provided.
    pub fn builder() -> AnthropicConfigBuilder {
        AnthropicConfigBuilder::default()
    }

    /// Override the base URL. Normalizes the value (strips a trailing slash and
    /// a trailing `/v1` segment so the endpoint formula `{base_url}/v1/messages`
    /// never doubles the version) and rejects empty strings.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = normalize_base_url(&url.into());
        self
    }

    /// Set the retry configuration. Pass `max_retries: 0` to disable retries.
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Set a custom provider name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Authenticate with a bearer token instead of an API key.
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Attach extra headers merged into every request.
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set provider-level request body overrides (RFC-0017).
    pub fn with_body_overrides(mut self, overrides: Value) -> Self {
        self.body_overrides = Some(overrides);
        self
    }

    /// 标注 api_key 来源(RFC-0023 回放重建用)。
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(|s| s.to_string());
        self
    }

    /// Load the configuration from the environment.
    ///
    /// Reads `ANTHROPIC_API_KEY` (required) and the optional
    /// `ANTHROPIC_BASE_URL`. `ANTHROPIC_AUTH_TOKEN` is not auto-loaded here;
    /// use [`AnthropicConfig::builder`]`.auth_token` for token auth.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "ANTHROPIC_API_KEY", "Anthropic")?;
        let mut config = Self::new(api_key).with_api_key_source(Some("env:ANTHROPIC_API_KEY"));
        if let Ok(base) = std::env::var("ANTHROPIC_BASE_URL")
            && !base.is_empty()
        {
            config = config.with_base_url(base);
        }
        Ok(config)
    }
}

/// Builder for [`AnthropicConfig`] supporting both `api_key` and `auth_token`
/// authentication, with conflict validation mirroring the TS `createAnthropic`.
#[derive(Debug, Default, Clone)]
pub struct AnthropicConfigBuilder {
    api_key: Option<String>,
    auth_token: Option<String>,
    base_url: Option<String>,
    name: Option<String>,
    headers: Option<HashMap<String, String>>,
    body_overrides: Option<Value>,
}

impl AnthropicConfigBuilder {
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn auth_token(mut self, auth_token: impl Into<String>) -> Self {
        self.auth_token = Some(auth_token.into());
        self
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    pub fn body_overrides(mut self, overrides: Value) -> Self {
        self.body_overrides = Some(overrides);
        self
    }

    /// Build the config, validating that `api_key` and `auth_token` are not
    /// both set and that a provided `base_url` is non-empty.
    pub fn build(self) -> Result<AnthropicConfig, AiMuxError> {
        if self.api_key.is_some() && self.auth_token.is_some() {
            return Err(AiMuxError::InvalidArgument(
                "Both apiKey and authToken were provided. Please use only one authentication method."
                    .to_string(),
            ));
        }
        let base_url = match &self.base_url {
            Some(url) => normalize_base_url(url),
            None => ANTHROPIC_API_URL.to_string(),
        };
        Ok(AnthropicConfig {
            api_key: self.api_key.unwrap_or_default(),
            auth_token: self.auth_token,
            base_url,
            api_version: "2023-06-01".to_string(),
            name: self
                .name
                .unwrap_or_else(|| "anthropic.messages".to_string()),
            headers: self.headers,
            retry_config: RetryConfig::default(),
            body_overrides: self.body_overrides,
            api_key_source: None,
        })
    }
}

/// Normalize an Anthropic base URL: reject empty strings, strip a trailing
/// slash, then strip a trailing `/v1` segment (so the endpoint formula
/// `{base_url}/v1/messages` produces a single version segment regardless of
/// whether the caller supplied `https://api.anthropic.com/`,
/// `https://api.anthropic.com/v1`, or a proxy like
/// `https://proxy.example/v1/`).
fn normalize_base_url(url: &str) -> String {
    if url.is_empty() {
        return ANTHROPIC_API_URL.to_string();
    }
    let mut s = url.trim_end_matches('/').to_string();
    if s.ends_with("/v1") {
        s = s[..s.len() - "/v1".len()].trim_end_matches('/').to_string();
    }
    s
}

/// Anthropic provider — creates `AnthropicModel` instances.
pub struct AnthropicProvider {
    config: AnthropicConfig,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        Self { config }
    }

    pub fn model(&self, model_id: &str) -> model::AnthropicModel {
        model::AnthropicModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Files interface for uploading files to Anthropic.
    pub fn files(&self) -> files::AnthropicFiles {
        files::AnthropicFiles::new(self.config.clone())
    }
}

impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List models via `GET {base_url}/v1/models` (Anthropic native), enriched
    /// with the community catalogue portrait when available (RFC-0027).
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        Box::pin(async move {
            let base = config
                .base_url
                .trim_end_matches("/v1")
                .trim_end_matches('/');
            let url = format!("{base}/v1/models");
            let mut headers = std::collections::HashMap::new();
            headers.insert("anthropic-version".to_string(), config.api_version.clone());
            if let Some(token) = &config.auth_token {
                headers.insert("authorization".to_string(), format!("Bearer {token}"));
            } else {
                headers.insert("x-api-key".to_string(), config.api_key.clone());
            }
            if let Some(extra) = &config.headers {
                for (k, v) in extra {
                    headers.insert(k.clone(), v.clone());
                }
            }
            let mut header_list: Vec<(String, String)> = headers.into_iter().collect();
            header_list.push(("Content-Type".to_string(), "application/json".to_string()));

            use aimux_provider_utils::{
                DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, send_timed,
            };
            let resp = send_timed(
                HttpRequest {
                    method: HttpMethod::Get,
                    url,
                    headers: header_list,
                    body: HttpBody::Empty,
                    abort_signal: None,
                    call_id: None,
                    recording_context: None,
                },
                config.retry_config,
                &DEFAULT_ERROR_STRUCTURE,
                None,
            )
            .await?;

            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                data: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                id: String,
                #[serde(default)]
                display_name: Option<String>,
            }
            let parsed: Resp = serde_json::from_slice(&resp.body)?;
            let runtime: Vec<aimux_core::model_catalogue::RuntimeModel> = parsed
                .data
                .into_iter()
                .map(|e| aimux_core::model_catalogue::RuntimeModel {
                    id: e.id,
                    owned_by: e.display_name,
                    created: None,
                })
                .collect();
            Ok(runtime)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_slash_and_v1() {
        assert_eq!(
            normalize_base_url("https://api.anthropic.com/"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            normalize_base_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com"
        );
        assert_eq!(
            normalize_base_url("https://proxy.example/v1/"),
            "https://proxy.example"
        );
        assert_eq!(
            normalize_base_url("https://proxy.example"),
            "https://proxy.example"
        );
    }
}
