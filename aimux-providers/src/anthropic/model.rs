//! Anthropic language model — implements `LanguageModel` trait.
//!
//! The request body building, HTTP send and response/SSE parsing live in the
//! shared [`super::stream`] core; this module only wires the standard Anthropic
//! endpoint, Bearer/x-api-key auth and `Json` body encoding.

use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};

use super::AnthropicConfig;
use super::convert::build_request_body_with_warnings;
use super::stream::{BodyEncoding, anthropic_generate_core, anthropic_stream_core};
use aimux_provider_utils::RetryConfig;

/// An Anthropic language model (e.g. `claude-sonnet-4-20250514`).
pub struct AnthropicModel {
    model_id: String,
    config: AnthropicConfig,
}

impl AnthropicModel {
    pub fn new(model_id: String, config: AnthropicConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.config.base_url)
    }

    /// Build the auth-header closure for the standard (Bearer/x-api-key) path.
    ///
    /// The body bytes are ignored (no request signing). Config-level and
    /// per-call headers and the `anthropic-beta` header are merged last-wins,
    /// matching the original `HashMap` semantics.
    fn make_header_builder(
        &self,
        extra: Option<&HashMap<String, String>>,
        betas: BTreeSet<String>,
    ) -> impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError> {
        let api_version = self.config.api_version.clone();
        let auth_token = self.config.auth_token.clone();
        let api_key = self.config.api_key.clone();
        let cfg_headers = self.config.headers.clone();
        let extra = extra.cloned();
        move |_body: &[u8], _url: &str| -> Result<Vec<(String, String)>, AiMuxError> {
            let mut headers: HashMap<String, String> = HashMap::new();
            headers.insert("anthropic-version".to_string(), api_version.clone());
            // Auth: prefer bearer token, fall back to x-api-key.
            if let Some(token) = &auth_token {
                headers.insert("authorization".to_string(), format!("Bearer {token}"));
            } else {
                headers.insert("x-api-key".to_string(), api_key.clone());
            }
            // Extra config-level headers (lower precedence than per-call headers).
            if let Some(cfg_headers) = &cfg_headers {
                for (k, v) in cfg_headers {
                    headers.insert(k.clone(), v.clone());
                }
            }
            if let Some(extra) = &extra {
                for (k, v) in extra {
                    headers.insert(k.clone(), v.clone());
                }
            }
            if !betas.is_empty() {
                headers.insert(
                    "anthropic-beta".to_string(),
                    betas.iter().cloned().collect::<Vec<_>>().join(","),
                );
            }
            Ok(headers.into_iter().collect())
        }
    }
}

#[async_trait]
impl LanguageModel for AnthropicModel {
    fn provider(&self) -> &str {
        &self.config.name
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let options = merge_anthropic_body_overrides(options, &self.config.body_overrides);
        let req = build_request_body_with_warnings(&self.model_id, &options, false)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref(), req.betas);
        let retry_config = resolve_anthropic_retry(&self.config.retry_config, options.max_retries);
        anthropic_generate_core(
            &endpoint,
            retry_config,
            req.body,
            req.warnings,
            build_headers,
            BodyEncoding::Json,
            options.abort_signal.clone(),
            options.timeout.map(Into::into),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let options = merge_anthropic_body_overrides(options, &self.config.body_overrides);
        let req = build_request_body_with_warnings(&self.model_id, &options, true)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref(), req.betas);
        let retry_config = resolve_anthropic_retry(&self.config.retry_config, options.max_retries);
        anthropic_stream_core(
            &endpoint,
            retry_config,
            req.body,
            req.warnings,
            build_headers,
            BodyEncoding::Json,
            options.abort_signal.clone(),
            options.timeout.map(Into::into),
        )
        .await
    }
}

/// Resolve per-call max_retries override (RFC-0017). RetryConfig is Copy.
fn resolve_anthropic_retry(
    provider: &RetryConfig,
    max_retries_override: Option<u32>,
) -> RetryConfig {
    match max_retries_override {
        Some(n) => RetryConfig {
            max_retries: n,
            ..*provider
        },
        None => *provider,
    }
}

/// Merge provider-level body_overrides into per-call options (RFC-0017).
fn merge_anthropic_body_overrides(
    options: &CallOptions,
    provider_overrides: &Option<serde_json::Value>,
) -> CallOptions {
    match (provider_overrides, &options.body_overrides) {
        (Some(provider), Some(call)) => {
            let mut merged = provider.clone();
            crate::openai::convert::deep_merge_json(&mut merged, call);
            let mut opts = options.clone();
            opts.body_overrides = Some(merged);
            opts
        }
        (Some(provider), None) => {
            let mut opts = options.clone();
            opts.body_overrides = Some(provider.clone());
            opts
        }
        (None, _) => options.clone(),
    }
}
