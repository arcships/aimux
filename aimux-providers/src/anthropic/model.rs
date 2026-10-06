//! The Anthropic Messages language model — implements `LanguageModel`.
//!
//! One model serves every host of the Messages API (`api.anthropic.com`,
//! Claude Platform on AWS, Anthropic on Vertex): it builds the request body,
//! asks its model configuration for the URL, the headers and the transport
//! and hands the result to the shared [`super::stream`] core, which sends it
//! and parses the response or the SSE stream.

use async_trait::async_trait;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::types::Warning;
use aimux_provider_utils::HttpRequest;

use super::config::AnthropicModelConfig;
use super::convert::build_request_body_for;
use super::stream::{anthropic_generate_core, anthropic_stream_core};
use super::tool_name_mapping::ToolNameMapping;

/// One call, ready to send.
struct PreparedCall {
    http: HttpRequest,
    body: Value,
    warnings: Vec<Warning>,
}

/// An Anthropic Messages model (e.g. `claude-sonnet-4-20250514`), created by
/// `provider.messages(id)`.
pub struct AnthropicMessagesModel {
    model_id: String,
    config: AnthropicModelConfig,
}

impl AnthropicMessagesModel {
    /// A model of the host the config describes.
    pub(crate) fn with_config(model_id: String, config: AnthropicModelConfig) -> Self {
        Self { model_id, config }
    }

    /// Build the request for one call: the body (host preparation and the
    /// provider's `transform_request_body` applied), its warnings, and the
    /// URL/headers/transport the exchange goes through.
    async fn prepare(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<PreparedCall, AiMuxError> {
        let config = self.config.resolved().await?;
        let built =
            build_request_body_for(&self.model_id, options, stream, &config.request_profile())?;
        let body = config.transform_body(built.body);
        let headers = config
            .request_headers(options.headers.as_ref(), &built.betas)
            .await?;
        let http = config.http_request(
            config.messages_url(&self.model_id, stream),
            headers,
            options,
        );
        Ok(PreparedCall {
            http,
            body,
            warnings: built.warnings,
        })
    }
}

#[async_trait]
impl LanguageModel for AnthropicMessagesModel {
    /// `"{name}"`: `"anthropic.messages"` unless the provider was named.
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supported_urls(&self) -> SupportedUrls {
        self.config.supported_urls.clone()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let call = self.prepare(options, false).await?;
        anthropic_generate_core(
            call.http,
            call.body,
            call.warnings,
            &self.config,
            options
                .provider_options
                .as_ref()
                .is_some_and(|options| options.contains_key(&self.config.provider_options_name)),
            &ToolNameMapping::new(options.tools.as_deref()),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let call = self.prepare(options, true).await?;
        anthropic_stream_core(
            call.http,
            call.body,
            call.warnings,
            &self.config,
            options
                .provider_options
                .as_ref()
                .is_some_and(|options| options.contains_key(&self.config.provider_options_name)),
            ToolNameMapping::new(options.tools.as_deref()),
        )
        .await
    }
}
