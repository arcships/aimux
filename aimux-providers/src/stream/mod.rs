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

use serde_json::Value;

use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::Warning;
use aimux_provider_utils::TransformStreamController;

pub(crate) use chat_tool_calls::{ChatToolCalls, ToolCallDelta};

/// The request a stream answers, fixed before the first chunk: the
/// warnings `start` hands out, whether raw chunks are forwarded, and what an
/// in-stream error reports (the URL, the request body and the response
/// headers). A stream that reports none of these leaves them empty.
#[derive(Default)]
pub(crate) struct StreamContext {
    pub(crate) warnings: Vec<Warning>,
    /// `includeRawChunks`.
    pub(crate) include_raw_chunks: bool,
    pub(crate) url: String,
    pub(crate) request_body: Value,
    pub(crate) response_headers: HashMap<String, String>,
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
        }
    }

    /// The `stream-start` part every stream begins with.
    pub(crate) fn start(&mut self, controller: &mut TransformStreamController<StreamPart>) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    /// Forward `chunk` as a `raw` part when raw chunks were asked for.
    pub(crate) fn raw(
        &self,
        chunk: &Value,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        if self.include_raw_chunks {
            controller.enqueue(StreamPart::Raw {
                raw_value: chunk.clone(),
            });
        }
    }
}
