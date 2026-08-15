//! Anthropic-AWS (Claude Platform on AWS) provider.
//!
//! Implements the `LanguageModel` trait against the Claude Platform on AWS API
//! (`aws-external-anthropic.{region}.api.aws/v1/messages`). This is the
//! Anthropic Messages API hosted in AWS, authenticated with either AWS SigV4
//! or an AWS-provisioned API key.
//!
//! The request/response format is identical to the standard Anthropic API, so
//! this provider reuses the shared [`crate::anthropic::convert`] message
//! conversion logic. The differences are:
//! - **Endpoint**: `aws-external-anthropic.{region}.api.aws/v1/messages`
//!   instead of `api.anthropic.com/v1/messages`.
//! - **Authentication**: AWS SigV4 signing (service name
//!   `aws-external-anthropic`) or `x-api-key` header.
//!
//! Reference: <https://docs.anthropic.com/en/api/messages>

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{RetryConfig, without_trailing_slash};

mod model;

pub use model::{AnthropicAwsConfig, AnthropicAwsModel};

use crate::bedrock::sigv4::AwsCredentials;

/// Authentication method for the Anthropic-AWS provider.
#[derive(Debug, Clone)]
pub enum AnthropicAwsAuth {
    /// `x-api-key` header authentication (takes precedence when provided).
    ApiKey(String),
    /// AWS SigV4 signing with static credentials (service:
    /// `aws-external-anthropic`).
    SigV4(AwsCredentials),
}

/// Configuration for the Anthropic-AWS provider.
#[derive(Debug, Clone)]
pub struct AnthropicAwsProviderConfig {
    pub base_url: String,
    pub auth: AnthropicAwsAuth,
    pub api_version: String,
    /// Optional workspace ID sent as `anthropic-workspace-id` header.
    pub workspace_id: Option<String>,
    /// 凭证来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// 重试配置(M1b)。默认 `RetryConfig::default()`（max_retries=2）。
    /// 取代之前硬编码的 `RetryConfig::default()`,让 per-call `max_retries` 生效。
    pub retry_config: RetryConfig,
}

impl AnthropicAwsProviderConfig {
    /// Create a config using AWS SigV4 credentials.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let base_url = format!("https://aws-external-anthropic.{region}.api.aws/v1");
        Self {
            base_url,
            auth: AnthropicAwsAuth::SigV4(AwsCredentials {
                access_key_id: access_key_id.into(),
                secret_access_key: secret_access_key.into(),
                session_token: None,
                region,
            }),
            api_version: "2023-06-01".to_string(),
            workspace_id: None,
            api_key_source: None,
            retry_config: RetryConfig::default(),
        }
    }

    /// Create a config using an API key (`x-api-key` auth).
    pub fn with_api_key(api_key: impl Into<String>, region: impl Into<String>) -> Self {
        let region = region.into();
        let base_url = format!("https://aws-external-anthropic.{region}.api.aws/v1");
        Self {
            base_url,
            auth: AnthropicAwsAuth::ApiKey(api_key.into()),
            api_version: "2023-06-01".to_string(),
            workspace_id: None,
            api_key_source: None,
            retry_config: RetryConfig::default(),
        }
    }

    /// Override the base URL (useful for tests / proxies).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Set the Anthropic API version header.
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Set the workspace ID.
    #[must_use]
    pub fn with_workspace_id(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }

    /// 标注凭证来源(RFC-0023 回放重建用)。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(std::string::ToString::to_string);
        self
    }

    /// Set the retry configuration. Pass `max_retries: 0` to disable retries.
    #[must_use]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Add a session token for temporary STS credentials.
    #[must_use]
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        if let AnthropicAwsAuth::SigV4(ref mut creds) = self.auth {
            creds.session_token = Some(token.into());
        }
        self
    }

    /// Create from environment variables.
    ///
    /// Checks for `ANTHROPIC_AWS_API_KEY` first (API key auth), then falls
    /// back to `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` + `AWS_REGION`.
    ///
    /// # Errors
    ///
    /// Returns `AiMuxError::InvalidArgument` when neither `ANTHROPIC_AWS_API_KEY`
    /// nor the `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` pair is present.
    pub fn from_env() -> Result<Self, AiMuxError> {
        if let Ok(key) = std::env::var("ANTHROPIC_AWS_API_KEY")
            && !key.trim().is_empty()
        {
            let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
            return Ok(Self::with_api_key(key, region)
                .with_api_key_source(Some("env:ANTHROPIC_AWS_API_KEY")));
        }

        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_ACCESS_KEY_ID (or ANTHROPIC_AWS_API_KEY) environment variable is required for Anthropic-AWS"
                    .to_string(),
            )
        })?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_SECRET_ACCESS_KEY environment variable is required for Anthropic-AWS"
                    .to_string(),
            )
        })?;
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let mut config = Self::new(access_key_id, secret_access_key, region)
            .with_api_key_source(Some("env:AWS_ACCESS_KEY_ID"));
        if let Ok(token) = std::env::var("AWS_SESSION_TOKEN") {
            config = config.with_session_token(token);
        }
        if let Ok(ws) = std::env::var("ANTHROPIC_AWS_WORKSPACE_ID") {
            config = config.with_workspace_id(ws);
        }
        Ok(config)
    }
}

/// Anthropic-AWS provider — creates [`AnthropicAwsModel`] instances.
pub struct AnthropicAwsProvider {
    config: AnthropicAwsProviderConfig,
}

impl AnthropicAwsProvider {
    #[must_use]
    pub fn new(config: AnthropicAwsProviderConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given Anthropic model id
    /// (e.g. `"claude-sonnet-4-20250514"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> AnthropicAwsModel {
        AnthropicAwsModel::new(
            model_id.to_string(),
            AnthropicAwsConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
                api_version: self.config.api_version.clone(),
                workspace_id: self.config.workspace_id.clone(),
                api_key_source: self.config.api_key_source.clone(),
                retry_config: self.config.retry_config,
            },
        )
    }
}

impl Provider for AnthropicAwsProvider {
    fn name(&self) -> &str {
        "anthropic-aws"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List models via `GET {base_url}/models` (Anthropic-compatible, RFC-0027).
    /// API-key auth uses a simple header; SigV4 auth is not supported for
    /// listing (returns `Unsupported`).
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
            let base = config.base_url.trim_end_matches('/');
            let url = format!("{base}/models");
            let mut headers = vec![
                ("anthropic-version".to_string(), config.api_version.clone()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ];
            match &config.auth {
                AnthropicAwsAuth::ApiKey(key) => {
                    headers.push(("x-api-key".to_string(), key.clone()));
                }
                AnthropicAwsAuth::SigV4(_) => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "list_models via SigV4 not supported for Anthropic-AWS; use API key auth"
                            .into(),
                    ));
                }
            }

            use aimux_provider_utils::{
                DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, send_timed,
            };
            let resp = send_timed(
                HttpRequest {
                    method: HttpMethod::Get,
                    url,
                    headers,
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
