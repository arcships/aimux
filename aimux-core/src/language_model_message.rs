//! Provider-facing message types.
//!
//! `LanguageModelPrompt` is the standardized prompt format that providers receive
//! in `CallOptions.prompt`. It is converted from `ModelMessage` (user-facing)
//! by the `generate_text` / `stream_text` functions.

use crate::content::ContentPart;
use crate::message::Role;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// A single provider-facing message.
///
/// Unlike `ModelMessage` (which can have string content), this always has
/// `content: Vec<ContentPart>` — strings are normalized to `ContentPart::Text`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LanguageModelPromptMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
    /// Message-level provider-specific options (e.g.
    /// `anthropic.cacheControl`), applied to the last part of the message when
    /// converting to a provider prompt. Mirrors the Vercel AI SDK
    /// `LanguageModelV4Message.providerOptions`.
    pub provider_options: Option<Value>,
}

/// The standardized prompt passed to `LanguageModel::do_generate` / `do_stream`.
pub type LanguageModelPrompt = Vec<LanguageModelPromptMessage>;

/// Convert user-facing `ModelMessage`s into a `LanguageModelPrompt`.
///
/// - String content is normalized to a single `ContentPart::Text`.
/// - If `instructions` is provided, it is prepended as a system message.
pub fn convert_to_language_model_prompt(
    messages: &[crate::message::ModelMessage],
    instructions: Option<&str>,
) -> LanguageModelPrompt {
    let mut result = Vec::new();

    // Prepend system instructions if provided.
    if let Some(instructions) = instructions {
        result.push(LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::text(instructions)],
            provider_options: None,
        });
    }

    for msg in messages {
        let parts = match &msg.content {
            crate::message::MessageContent::Text(text) => vec![ContentPart::text(text)],
            crate::message::MessageContent::Parts(parts) => parts.clone(),
        };
        result.push(LanguageModelPromptMessage {
            role: msg.role,
            content: parts,
            provider_options: None,
        });
    }

    result
}
