//! `src/chat/` of `@ai-sdk/deepseek`.

mod chat_api_types;
mod chat_language_model;
mod chat_language_model_options;
pub(super) mod convert_to_chat_messages;
pub(super) mod convert_to_usage;
mod file_part_options;
pub(super) mod is_v4_model;
mod map_finish_reason;
pub(super) mod prepare_tools;

pub(crate) use chat_api_types::deepseek_failed_response_handler;
pub(crate) use chat_language_model::DeepSeekChatConfig;
pub use chat_language_model::DeepSeekChatLanguageModel;
