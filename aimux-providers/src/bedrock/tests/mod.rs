//! Tests ported from `@ai-sdk/amazon-bedrock` 5.0.100, one file per upstream test file.

mod chat_language_model;
mod chat_language_model_event_stream;
mod chat_language_model_options;
mod convert_to_chat_messages;
mod convert_usage;
mod embedding_model;
mod error;
mod event_stream_decoder;
mod event_stream_response_handler;
mod image_model;
mod normalize_tool_call_id;
mod prepare_tools;
mod provider;
mod remaining_support;
mod reranking;
mod sigv4_fetch;
mod support;
mod upstream_e2e_support;
mod usage_raw;
