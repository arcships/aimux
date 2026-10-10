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
use aimux_core::language_model_message::{LanguageModelMessage, UserPart};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::types::Warning;
use aimux_provider_utils::HttpRequest;

use super::config::AnthropicModelConfig;
use super::convert::build_request_body_for;
use super::options::CANONICAL;
use super::stream::{CitationDocument, anthropic_generate_core, anthropic_stream_core};
use super::tool_name_mapping::ToolNameMapping;

/// One call, ready to send.
struct PreparedCall {
    http: HttpRequest,
    body: Value,
    warnings: Vec<Warning>,
    uses_json_response_tool: bool,
}

/// An Anthropic Messages model (e.g. `claude-sonnet-4-20250514`), created by
/// `provider.messages(id)`.
pub struct AnthropicMessagesModel {
    model_id: String,
    config: AnthropicModelConfig,
    generate_id: aimux_provider_utils::IdGenerator,
}

impl AnthropicMessagesModel {
    /// A model of the host the config describes.
    pub(crate) fn with_config(model_id: String, config: AnthropicModelConfig) -> Self {
        Self {
            model_id,
            config,
            generate_id: aimux_provider_utils::generate_id,
        }
    }

    #[must_use]
    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<aimux_provider_utils::IdGenerator>,
    ) -> Self {
        if let Some(generate_id) = generate_id {
            self.generate_id = generate_id;
        }
        self
    }

    fn citation_documents(options: &CallOptions) -> Vec<CitationDocument> {
        options
            .prompt
            .iter()
            .filter_map(|message| match message {
                LanguageModelMessage::User { content, .. } => Some(content),
                _ => None,
            })
            .flatten()
            .filter_map(|part| match part {
                UserPart::File(file)
                    if matches!(file.media_type.as_str(), "application/pdf" | "text/plain")
                        && file
                            .provider_options
                            .as_ref()
                            .and_then(|options| options.get(CANONICAL))
                            .and_then(|options| options.get("citations"))
                            .and_then(|citations| citations.get("enabled"))
                            .and_then(Value::as_bool)
                            == Some(true) =>
                {
                    Some(CitationDocument {
                        title: file
                            .filename
                            .clone()
                            .unwrap_or_else(|| "Untitled Document".to_string()),
                        filename: file.filename.clone(),
                        media_type: file.media_type.clone(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn uses_custom_options(&self, options: &CallOptions) -> bool {
        self.config.provider_options_name != CANONICAL
            && options
                .provider_options
                .as_ref()
                .is_some_and(|value| value.contains_key(&self.config.provider_options_name))
    }

    /// Build the request for one call: the body (host preparation applied),
    /// its warnings, and the URL/headers/transport the exchange goes through.
    async fn prepare(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<PreparedCall, AiMuxError> {
        let config = self.config.resolved().await?;
        let built =
            build_request_body_for(&self.model_id, options, stream, &config.request_profile())?;
        let body = config.prepare_body(built.body);
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
            uses_json_response_tool: built.uses_json_response_tool,
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
            &ToolNameMapping::new(options.tools.as_deref()),
            self.uses_custom_options(options),
            call.uses_json_response_tool,
            Self::citation_documents(options),
            self.generate_id,
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
            ToolNameMapping::new(options.tools.as_deref()),
            self.uses_custom_options(options),
            call.uses_json_response_tool,
            Self::citation_documents(options),
            self.generate_id,
        )
        .await
    }
}
