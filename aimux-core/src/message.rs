//! User-facing message types.
//!
//! `ModelMessage` is what the user passes to `generate_text` / `stream_text`.
//! It is converted to `LanguageModelPrompt` (provider-facing) before calling
//! `LanguageModel::do_generate` / `do_stream`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::content::ContentPart;

/// Who sent the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Role {
    System,
    /// The default role (a bare `Role::default()` yields `User`, matching the
    /// common case for prompts built in tests).
    #[default]
    User,
    Assistant,
    Tool,
}

/// Message body: either a simple string or multi-part content.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// A single user-facing chat message.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelMessage {
    pub role: Role,
    pub content: MessageContent,
}

impl ModelMessage {
    /// Convenience constructor for a text message.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: MessageContent::Text(text.into()),
        }
    }

    /// System message.
    pub fn system(text: impl Into<String>) -> Self {
        Self::text(Role::System, text)
    }

    /// User message.
    pub fn user(text: impl Into<String>) -> Self {
        Self::text(Role::User, text)
    }

    /// Assistant message.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::text(Role::Assistant, text)
    }

    /// User message with multi-part content.
    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Parts(parts),
        }
    }
}

/// What the user passes as `prompt` to `generate_text` / `stream_text`.
///
/// Can be a simple string (converted to a single user message) or a list of messages.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum ModelPrompt {
    /// A plain string prompt — equivalent to a single user message.
    Text(String),

    /// A list of chat messages.
    Messages(Vec<ModelMessage>),
}

impl From<String> for ModelPrompt {
    fn from(s: String) -> Self {
        ModelPrompt::Text(s)
    }
}

impl From<&str> for ModelPrompt {
    fn from(s: &str) -> Self {
        ModelPrompt::Text(s.to_string())
    }
}

impl From<Vec<ModelMessage>> for ModelPrompt {
    fn from(msgs: Vec<ModelMessage>) -> Self {
        ModelPrompt::Messages(msgs)
    }
}
