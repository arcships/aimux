//! `chat/`: the Chat Completions language model.

pub(super) mod chat_api;
pub(super) mod chat_language_model;
pub(super) mod chat_language_model_options;
pub(super) mod chat_prepare_tools;
pub(super) mod convert_chat_usage;
pub(super) mod convert_to_chat_messages;
pub(super) mod map_finish_reason;

pub use chat_language_model::OpenAIModel;
