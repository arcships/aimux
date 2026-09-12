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
use super::tool_name_mapping::ToolNameMapping;

/// An Anthropic language model (e.g. `claude-sonnet-4-20250514`).
pub struct AnthropicModel {
    model_id: String,
    config: AnthropicConfig,
}

impl AnthropicModel {
    #[must_use]
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

    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
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
            // RFC-0023 C6: ProviderOptions-shaped object (headers/max_retries/
            // body_overrides are the shared ProviderOptions fields) plus the
            // Anthropic-specific `api_version`. Sensitive headers are redacted
            // at the recording boundary (recording.rs redacts provider_options).
            provider_options: Some(serde_json::json!({
                "headers": self.config.headers,
                "api_version": self.config.api_version,
                "max_retries": self.config.retry_config.max_retries,
                "body_overrides": self.config.body_overrides,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let options = merge_anthropic_body_overrides(options, &self.config.body_overrides);
        let req = build_request_body_with_warnings(&self.model_id, &options, false)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref(), req.betas);
        anthropic_generate_core(
            &endpoint,
            req.body,
            req.warnings,
            build_headers,
            BodyEncoding::Json,
            options.abort_signal.clone(),
            options.recording_context.clone(),
            &ToolNameMapping::new(options.tools.as_deref()),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let options = merge_anthropic_body_overrides(options, &self.config.body_overrides);
        let req = build_request_body_with_warnings(&self.model_id, &options, true)?;
        let endpoint = self.endpoint();
        let build_headers = self.make_header_builder(options.headers.as_ref(), req.betas);
        anthropic_stream_core(
            &endpoint,
            req.body,
            req.warnings,
            build_headers,
            BodyEncoding::Json,
            options.abort_signal.clone(),
            options.recording_context.clone(),
            ToolNameMapping::new(options.tools.as_deref()),
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_snapshot_matches_model_identity() {
        let config = AnthropicConfig::new("sk-test");
        let model = AnthropicModel::new("claude-sonnet-4-20250514".to_string(), config);
        let snap = model.config_snapshot();
        assert_eq!(snap.provider, model.provider());
        assert_eq!(snap.model_id, model.model_id());
        assert!(snap.base_url.is_some());
        assert_eq!(snap.api_key_source, "explicit");
    }
}
