//! Result types for `do_generate` and `do_stream`.

use std::collections::HashMap;

use futures::Stream;
use std::pin::Pin;

use crate::error::AiMuxError;
use crate::stream_part::StreamPart;
use crate::types::{FinishReason, ProviderMetadata, ResponseMetadata, Usage, Warning};

use serde_json::Value;

/// A content item in the generation result.
#[derive(Debug, Clone, PartialEq)]
pub enum GenerateContent {
    /// Generated text.
    Text { text: String },
    /// A tool call requested by the model.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    /// A source / citation (e.g. URL citation from search-preview models).
    Source {
        id: String,
        source_type: String,
        url: Option<String>,
        title: Option<String>,
    },
    /// A reasoning / thinking segment produced by the model.
    ///
    /// Mirrors the Vercel AI SDK `reasoning` content type. `provider_metadata`
    /// carries provider-specific data such as the Anthropic thinking-block
    /// signature (`providerMetadata.anthropic.signature`).
    Reasoning {
        text: String,
        provider_metadata: Option<ProviderMetadata>,
    },
    /// A tool result from a provider-executed tool (e.g. xAI file_search,
    /// web_search). Emitted alongside the preceding `ToolCall` when the
    /// provider executes the tool server-side.
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        result: Value,
    },
}

/// Result of `LanguageModel::do_generate` (non-streaming).
#[derive(Debug)]
pub struct GenerateResult {
    /// Generated content items (text, tool calls, etc.).
    pub content: Vec<GenerateContent>,
    /// Why generation stopped.
    pub finish_reason: FinishReason,
    /// Token usage.
    pub usage: Usage,
    /// Warnings issued by the provider.
    pub warnings: Vec<Warning>,
    /// Provider-specific metadata.
    pub provider_metadata: Option<ProviderMetadata>,
    /// Response metadata (id, timestamp, model_id).
    pub response: ResponseMetadata,
    /// The request body that was sent (for debugging).
    pub request_body: Option<serde_json::Value>,
    /// Response headers.
    pub response_headers: Option<HashMap<String, String>>,
}

/// Result of `LanguageModel::do_stream` (streaming).
pub struct StreamResult {
    /// The stream of `StreamPart` items.
    pub stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>,
    /// The request body that was sent (for debugging).
    pub request_body: Option<serde_json::Value>,
    /// Response headers.
    pub response_headers: Option<HashMap<String, String>>,
}

impl std::fmt::Debug for StreamResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResult")
            .field("request_body", &self.request_body)
            .field("response_headers", &self.response_headers)
            .field("stream", &"<stream>")
            .finish()
    }
}
