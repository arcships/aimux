//! Anthropic-AWS language model — implements `LanguageModel`.
//!
//! Reuses the shared [`crate::anthropic::convert`] message conversion logic,
//! [`crate::anthropic::types`] response types, and the shared streaming /
//! non-streaming core in [`crate::anthropic::stream`]. Only the endpoint,
//! authentication (AWS SigV4 / x-api-key) and `Bytes` body encoding differ from
//! the standard Anthropic provider — those are supplied via the header-builder
//! closure and [`BodyEncoding::Bytes`].

use std::collections::HashMap;

use async_trait::async_trait;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_provider_utils::RetryConfig;

use crate::anthropic::convert::build_request_body;
use crate::anthropic::stream::{BodyEncoding, anthropic_generate_core, anthropic_stream_core};
use crate::bedrock::sigv4::sign_request;

use super::AnthropicAwsAuth;

/// Configuration for an Anthropic-AWS model instance.
#[derive(Debug, Clone)]
pub struct AnthropicAwsConfig {
    pub base_url: String,
    pub auth: AnthropicAwsAuth,
    pub api_version: String,
    pub workspace_id: Option<String>,
    /// 凭证来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    pub api_key_source: Option<String>,
    /// 重试配置(M1b)。默认 `RetryConfig::default()`（max_retries=2）。
    pub retry_config: RetryConfig,
}

/// An Anthropic-AWS language model (e.g. `claude-sonnet-4-20250514`).
pub struct AnthropicAwsModel {
    model_id: String,
    config: AnthropicAwsConfig,
}

impl AnthropicAwsModel {
    pub fn new(model_id: String, config: AnthropicAwsConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/messages", self.config.base_url)
    }

    /// Build the auth-header closure for the Anthropic-AWS path.
    ///
    /// The serialized body bytes are passed to AWS SigV4 signing (so the
    /// signature is computed over the exact bytes that will be sent, preventing
    /// re-serialization from invalidating it). For `x-api-key` auth the body is
    /// unused.
    fn make_header_builder(
        &self,
        extra: Option<&HashMap<String, String>>,
    ) -> impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError> {
        let api_version = self.config.api_version.clone();
        let workspace_id = self.config.workspace_id.clone();
        let auth = self.config.auth.clone();
        let extra = extra.cloned();
        move |body: &[u8], url: &str| -> Result<Vec<(String, String)>, AiMuxError> {
            let body_str = std::str::from_utf8(body).unwrap_or_default();
            let mut base_headers = vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("anthropic-version".to_string(), api_version.clone()),
            ];

            if let Some(ref ws) = workspace_id {
                base_headers.push(("anthropic-workspace-id".to_string(), ws.clone()));
            }

            if let Some(extra) = &extra {
                for (k, v) in extra {
                    base_headers.push((k.clone(), v.clone()));
                }
            }

            match &auth {
                AnthropicAwsAuth::ApiKey(key) => {
                    base_headers.push(("x-api-key".to_string(), key.clone()));
                    Ok(base_headers)
                }
                AnthropicAwsAuth::SigV4(creds) => {
                    // Sign the request with AWS SigV4 (service:
                    // aws-external-anthropic).
                    let extra_for_signing: Vec<(String, String)> = base_headers
                        .iter()
                        .filter(|(k, _)| k != "Content-Type")
                        .cloned()
                        .collect();

                    let signed = sign_request(
                        creds,
                        "aws-external-anthropic",
                        "POST",
                        url,
                        body_str,
                        &extra_for_signing,
                    );

                    let mut headers =
                        vec![("Content-Type".to_string(), "application/json".to_string())];
                    for (k, v) in &signed.headers {
                        headers.push((k.clone(), v.clone()));
                    }
                    Ok(headers)
                }
            }
        }
    }
}

#[async_trait]
impl LanguageModel for AnthropicAwsModel {
    fn provider(&self) -> &str {
        "anthropic-aws"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        // M2b: record identity + credential source + endpoint config. Only the
        // auth *kind* is serialized — never the SigV4 credentials or api-key
        // plaintext.
        let auth_kind = match &self.config.auth {
            AnthropicAwsAuth::ApiKey(_) => "api_key",
            AnthropicAwsAuth::SigV4(_) => "sigv4",
        };
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id.clone(),
            base_url: Some(self.config.base_url.clone()),
            api_key_source: self
                .config
                .api_key_source
                .clone()
                .unwrap_or_else(|| "explicit".to_string()),
            profile: None,
            provider_options: Some(serde_json::json!({
                "auth_kind": auth_kind,
                "api_version": self.config.api_version,
                "workspace_id": self.config.workspace_id,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options, false)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );
        anthropic_generate_core(
            &endpoint,
            retry_config,
            body,
            Vec::new(),
            build_headers,
            BodyEncoding::Bytes,
            options.abort_signal.clone(),
            options.timeout.map(Into::into),
            options.recording_context.clone(),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options, true)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref());
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );
        anthropic_stream_core(
            &endpoint,
            retry_config,
            body,
            Vec::new(),
            build_headers,
            BodyEncoding::Bytes,
            options.abort_signal.clone(),
            options.timeout.map(Into::into),
            options.recording_context.clone(),
        )
        .await
    }
}
