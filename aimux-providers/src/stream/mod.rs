//! What the vendor `doStream` transformers share.
//!
//! A vendor stream is a [`Transformer`](aimux_provider_utils::Transformer)
//! struct: the read-only [`StreamContext`] of its request plus the state it
//! accumulates between chunks. The OpenAI-style chat streams also share
//! [`ChatToolCalls`].

mod chat_tool_calls;
#[cfg(test)]
mod chat_tool_calls_tests;
mod tool_call_arguments;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};
use aimux_provider_utils::TransformStreamController;

pub(crate) use chat_tool_calls::{ChatToolCalls, ToolCallDelta};

/// The request a stream answers, fixed before the first chunk: the
/// warnings `start` hands out, whether raw chunks are forwarded, what an
/// in-stream error reports (the URL, the request body and the response
/// headers) and the model's id generator. A stream that reports none of
/// these leaves them empty.
pub(crate) struct StreamContext {
    pub(crate) warnings: Vec<Warning>,
    /// `includeRawChunks`.
    pub(crate) include_raw_chunks: bool,
    pub(crate) url: String,
    pub(crate) request_body: Value,
    pub(crate) response_headers: HashMap<String, String>,
    /// The model's `generateId`; [`aimux_provider_utils::generate_id`] unless
    /// the provider settings replace it.
    pub(crate) generate_id: GenerateId,
}

/// A model's id generator.
pub(crate) type GenerateId = Arc<dyn Fn() -> String + Send + Sync>;

impl Default for StreamContext {
    fn default() -> Self {
        Self {
            warnings: Vec::new(),
            include_raw_chunks: false,
            url: String::new(),
            request_body: Value::Null,
            response_headers: HashMap::new(),
            generate_id: Arc::new(aimux_provider_utils::generate_id),
        }
    }
}

impl StreamContext {
    pub(crate) fn new(
        options: &CallOptions,
        warnings: Vec<Warning>,
        url: String,
        request_body: &Value,
        response_headers: &HashMap<String, String>,
    ) -> Self {
        Self {
            warnings,
            include_raw_chunks: options.include_raw_chunks == Some(true),
            url,
            request_body: request_body.clone(),
            response_headers: response_headers.clone(),
            ..Self::default()
        }
    }

    /// The `stream-start` part every stream begins with.
    pub(crate) fn start(&mut self, controller: &mut Controller) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    /// Forward `chunk` as a `raw` part when raw chunks were asked for.
    pub(crate) fn raw(&self, chunk: &Value, controller: &mut Controller) {
        if self.include_raw_chunks {
            controller.enqueue(StreamPart::Raw {
                raw_value: chunk.clone(),
            });
        }
    }
}

/// The finish reason of a stream that ended without one.
pub(crate) const FINISH_OTHER: FinishReason = FinishReason {
    unified: FinishReasonUnified::Other,
    raw: None,
};

/// The finish reason after an error the stream reported.
pub(crate) const FINISH_ERROR: FinishReason = FinishReason {
    unified: FinishReasonUnified::Error,
    raw: None,
};

/// What a vendor stream writes its parts to.
pub(crate) type Controller = TransformStreamController<StreamPart>;

/// One-line forms of the parts a stream emits without provider metadata.
pub(crate) trait Emit {
    fn text_start(&mut self, id: impl Into<String>);
    fn text_delta(&mut self, id: impl Into<String>, delta: impl Into<String>);
    fn text_end(&mut self, id: impl Into<String>);
    fn reasoning_start(&mut self, id: impl Into<String>);
    fn reasoning_delta(&mut self, id: impl Into<String>, delta: impl Into<String>);
    fn reasoning_end(&mut self, id: impl Into<String>);
    /// A client-side tool call's input starts.
    fn tool_input_start(&mut self, id: impl Into<String>, tool_name: impl Into<String>);
    fn tool_input_delta(&mut self, id: impl Into<String>, delta: impl Into<String>);
    fn tool_input_end(&mut self, id: impl Into<String>);
}

impl Emit for Controller {
    fn text_start(&mut self, id: impl Into<String>) {
        self.enqueue(StreamPart::TextStart {
            id: id.into(),
            provider_metadata: None,
        });
    }

    fn text_delta(&mut self, id: impl Into<String>, delta: impl Into<String>) {
        self.enqueue(StreamPart::TextDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        });
    }

    fn text_end(&mut self, id: impl Into<String>) {
        self.enqueue(StreamPart::TextEnd {
            id: id.into(),
            provider_metadata: None,
        });
    }

    fn reasoning_start(&mut self, id: impl Into<String>) {
        self.enqueue(StreamPart::ReasoningStart {
            id: id.into(),
            provider_metadata: None,
        });
    }

    fn reasoning_delta(&mut self, id: impl Into<String>, delta: impl Into<String>) {
        self.enqueue(StreamPart::ReasoningDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        });
    }

    fn reasoning_end(&mut self, id: impl Into<String>) {
        self.enqueue(StreamPart::ReasoningEnd {
            id: id.into(),
            provider_metadata: None,
        });
    }

    fn tool_input_start(&mut self, id: impl Into<String>, tool_name: impl Into<String>) {
        self.enqueue(StreamPart::ToolInputStart {
            id: id.into(),
            tool_name: tool_name.into(),
            provider_executed: None,
            dynamic: None,
            title: None,
            provider_metadata: None,
        });
    }

    fn tool_input_delta(&mut self, id: impl Into<String>, delta: impl Into<String>) {
        self.enqueue(StreamPart::ToolInputDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        });
    }

    fn tool_input_end(&mut self, id: impl Into<String>) {
        self.enqueue(StreamPart::ToolInputEnd {
            id: id.into(),
            provider_metadata: None,
        });
    }
}
