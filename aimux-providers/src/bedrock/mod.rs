//! Amazon Bedrock provider.
//!
//! Implements the `LanguageModel` trait against the Bedrock Converse API
//! (`bedrock-runtime.{region}.amazonaws.com/model/{model-id}/converse`). The
//! Converse API provides a unified interface across all Bedrock-backed models
//! (Anthropic Claude, Meta Llama, Mistral, Amazon Nova, etc.).
//!
//! Authentication is either via:
//! - **AWS SigV4** signing (using `access_key_id` + `secret_access_key` +
//!   `region`), or
//! - **Bearer token** (using `AWS_BEARER_TOKEN_BEDROCK` / `apiKey`), which
//!   takes precedence when provided.

pub mod convert;
pub mod embedding;
pub mod event_stream;
pub mod image;
mod model;
pub mod reranking;
pub mod sigv4;
mod types;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::without_trailing_slash;

pub use embedding::BedrockEmbeddingModel;
pub use image::BedrockImageModel;
pub use model::{BedrockConfig, BedrockModel};
pub use reranking::BedrockRerankingModel;
pub use sigv4::AwsCredentials;

/// Authentication method for the Bedrock provider.
#[derive(Debug, Clone)]
pub enum BedrockAuth {
    /// Bearer token authentication (takes precedence when provided).
    BearerToken(String),
    /// AWS SigV4 signing with static credentials.
    SigV4(AwsCredentials),
}

/// Configuration for the Amazon Bedrock provider.
#[derive(Debug, Clone)]
pub struct BedrockProviderConfig {
    pub base_url: String,
    pub auth: BedrockAuth,
    /// AWS region, used for constructing model ARNs and agent-runtime URLs.
    pub region: String,
}

impl BedrockProviderConfig {
    /// Create a config using AWS SigV4 credentials.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let base_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);
        Self {
            base_url,
            auth: BedrockAuth::SigV4(AwsCredentials {
                access_key_id: access_key_id.into(),
                secret_access_key: secret_access_key.into(),
                session_token: None,
                region: region.clone(),
            }),
            region,
        }
    }

    /// Create a config using a Bearer token.
    pub fn with_bearer_token(token: impl Into<String>, region: impl Into<String>) -> Self {
        let region = region.into();
        let base_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);
        Self {
            base_url,
            auth: BedrockAuth::BearerToken(token.into()),
            region,
        }
    }

    /// Override the base URL (useful for tests / proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Add a session token for temporary STS credentials.
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        if let BedrockAuth::SigV4(ref mut creds) = self.auth {
            creds.session_token = Some(token.into());
        }
        self
    }

    /// Create from environment variables.
    ///
    /// Checks for `AWS_BEARER_TOKEN_BEDROCK` first (Bearer auth), then falls
    /// back to `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` + `AWS_REGION`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        // Check for bearer token first.
        if let Ok(token) = std::env::var("AWS_BEARER_TOKEN_BEDROCK")
            && !token.trim().is_empty()
        {
            let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
            return Ok(Self::with_bearer_token(token, region));
        }

        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_ACCESS_KEY_ID environment variable is required for Bedrock SigV4 auth"
                    .to_string(),
            )
        })?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
            AiMuxError::InvalidArgument(
                "AWS_SECRET_ACCESS_KEY environment variable is required for Bedrock SigV4 auth"
                    .to_string(),
            )
        })?;
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

        let mut config = Self::new(access_key_id, secret_access_key, region);
        if let Ok(token) = std::env::var("AWS_SESSION_TOKEN") {
            config = config.with_session_token(token);
        }
        Ok(config)
    }

    /// The Bedrock Agent Runtime base URL for this region (used for reranking).
    pub fn agent_runtime_url(&self) -> String {
        format!(
            "https://bedrock-agent-runtime.{}.amazonaws.com",
            self.region
        )
    }
}

/// Amazon Bedrock provider — creates [`BedrockModel`] instances.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct BedrockProvider {
    config: BedrockProviderConfig,
}

impl BedrockProvider {
    pub fn new(config: BedrockProviderConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given Bedrock model id
    /// (e.g. `"anthropic.claude-3-5-sonnet-20240620-v1:0"`).
    pub fn model(&self, model_id: &str) -> BedrockModel {
        BedrockModel::new(
            model_id.to_string(),
            BedrockConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
        )
    }

    /// Create an embedding model instance for the given Bedrock model id
    /// (e.g. `"amazon.titan-embed-text-v2:0"`).
    pub fn embedding_model(&self, model_id: &str) -> BedrockEmbeddingModel {
        BedrockEmbeddingModel::new(
            model_id.to_string(),
            BedrockConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
        )
    }

    /// Create an image generation model instance for the given Bedrock model id
    /// (e.g. `"amazon.titan-image-generator-v1"` or `"amazon.nova-canvas-v1:0"`).
    pub fn image(&self, model_id: &str) -> BedrockImageModel {
        BedrockImageModel::new(
            model_id.to_string(),
            BedrockConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
        )
    }

    /// Create a reranking model instance for the given Bedrock model id
    /// (e.g. `"cohere.rerank-v3-5:0"`). Uses the Bedrock Agent Runtime
    /// `/rerank` endpoint.
    pub fn reranking_model(&self, model_id: &str) -> BedrockRerankingModel {
        // Derive the agent-runtime URL from the base URL by replacing
        // `bedrock-runtime` with `bedrock-agent-runtime`. When the base URL
        // is overridden (e.g. for tests), the replacement is a no-op.
        let reranking_base_url = self
            .config
            .base_url
            .replace("bedrock-runtime", "bedrock-agent-runtime");
        BedrockRerankingModel::new(
            model_id.to_string(),
            reranking_base_url,
            self.config.region.clone(),
            self.config.auth.clone(),
        )
    }
}

impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        "amazon-bedrock"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List foundation models via AWS Bedrock `ListFoundationModels` API
    /// (SigV4-signed GET, RFC-0027).
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::ResolvedModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let region = self.config.region.clone();
        let auth = self.config.auth.clone();
        Box::pin(async move {
            let url = format!("https://bedrock.{region}.api.amazonaws.com/foundation-models");
            // Sign the request (empty body for GET).
            let signed = match &auth {
                BedrockAuth::SigV4(creds) => {
                    sigv4::sign_request(creds, "bedrock", "GET", &url, "", &[])
                }
                BedrockAuth::BearerToken(_) => {
                    return Err(AiMuxError::Unsupported(
                        "list_models via BearerToken not supported for Bedrock; use SigV4 credentials".into(),
                    ));
                }
            };
            let mut headers = signed.headers;
            headers.push(("Accept".to_string(), "application/json".to_string()));

            use aimux_provider_utils::{
                DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, send,
            };
            let resp = send(
                HttpRequest {
                    method: HttpMethod::Get,
                    url,
                    headers,
                    body: HttpBody::Empty,
                    abort_signal: None,
                },
                aimux_provider_utils::RetryConfig::default(),
                &DEFAULT_ERROR_STRUCTURE,
            )
            .await?;

            // AWS response: { modelSummaries: [{ modelId, modelName, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default, rename = "modelSummaries")]
                summaries: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                #[serde(rename = "modelId")]
                id: String,
                #[serde(default, rename = "modelName")]
                name: Option<String>,
            }
            let parsed: Resp = serde_json::from_slice(&resp.body)
                .map_err(|e| AiMuxError::Json(format!("bedrock list_models: parse: {e}")))?;
            let runtime: Vec<aimux_core::model_catalogue::RuntimeModel> = parsed
                .summaries
                .into_iter()
                .map(|e| aimux_core::model_catalogue::RuntimeModel {
                    id: e.id,
                    owned_by: e.name.or(Some("amazon".to_string())),
                    created: None,
                })
                .collect();
            let resolved = crate::catalogue::resolve_with_catalogue(
                &crate::catalogue::default_sync(),
                "bedrock",
                runtime,
            )
            .await;
            Ok(resolved)
        })
    }
}
