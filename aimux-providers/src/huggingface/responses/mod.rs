//! Hugging Face Responses API language model.
//!
//! Implements the [`LanguageModel`](aimux_core::language_model::LanguageModel)
//! trait against the Hugging Face Responses API (`POST /responses`). This is
//! the lightest Responses implementation in the SDK: it supports function
//! tools only (no built-in tools), uses `text.format` for structured output,
//! and surfaces reasoning content from `reasoning` output items.
//!
//! One file per TS source, the vendor prefix dropped:
//! - `responses_language_model.rs` — `huggingface-responses-language-model.ts`
//! - `convert_to_responses_messages.rs` — `convert-to-huggingface-responses-messages.ts`
//! - `responses_prepare_tools.rs` — `huggingface-responses-prepare-tools.ts`
//! - `convert_responses_usage.rs` — `convert-huggingface-responses-usage.ts`
//! - `map_responses_finish_reason.rs` — `map-huggingface-responses-finish-reason.ts`
//! - `responses_language_model_options.rs` — `huggingface-responses-language-model-options.ts`

mod convert_responses_usage;
mod convert_to_responses_messages;
mod map_responses_finish_reason;
mod responses_language_model;
mod responses_language_model_options;
mod responses_prepare_tools;

pub use convert_to_responses_messages::convert_to_huggingface_responses_messages;
pub use responses_language_model::{
    HuggingFaceResponsesModel, RequestBodyResult, build_request_body_with_warnings,
};
pub use responses_prepare_tools::prepare_responses_tools;
