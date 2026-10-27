//! The chat-completions model (`src/chat/`).

mod api_types;
pub(super) mod chat_language_model;
mod chat_language_model_options;
mod convert_chat_usage;
pub(super) mod convert_to_chat_messages;
mod map_finish_reason;
mod metadata_extractor;
pub(super) mod prepare_tools;

pub use chat_language_model::{ConvertUsage, OpenAICompatibleChatModel, SupportedUrlsFn};
pub use metadata_extractor::{MetadataExtractor, StreamMetadataExtractor};

pub(crate) use chat_language_model::ChatSettings;
#[cfg(test)]
pub(crate) use chat_language_model::chat_test_stream;
pub(super) use chat_language_model::stream_error;
pub(crate) use convert_chat_usage::usage_from_raw;
pub(super) use map_finish_reason::parse_finish_reason;
