//! User-facing message types.
//!
//! `ModelMessage` is what the user passes to `generate_text` / `stream_text`:
//! the AI SDK's `ModelMessage` (`@ai-sdk/provider-utils`
//! `types/model-message.ts:10`), a union by role in which every message may
//! carry message-level `provider_options`. It is converted to
//! `LanguageModelPrompt` (provider-facing) before calling
//! `LanguageModel::do_generate` / `do_stream`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::content::{AssistantContentPart, ToolContentPart, UserContentPart};
use crate::shared::SharedProviderOptions;

/// A single user-facing chat message, a union by role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "role",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ModelMessage {
    /// Upstream `SystemModelMessage`.
    System {
        content: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Upstream `UserModelMessage`.
    User {
        content: UserContent,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Upstream `AssistantModelMessage`.
    Assistant {
        content: AssistantContent,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
    /// Upstream `ToolModelMessage`.
    Tool {
        content: ToolContent,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_options: Option<SharedProviderOptions>,
    },
}

/// The content of a user message: a string or parts (upstream `UserContent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum UserContent {
    Text(String),
    Parts(Vec<UserContentPart>),
}

/// The content of an assistant message: a string or parts (upstream
/// `AssistantContent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum AssistantContent {
    Text(String),
    Parts(Vec<AssistantContentPart>),
}

/// The content of a tool message (upstream `ToolContent`).
pub type ToolContent = Vec<ToolContentPart>;

impl ModelMessage {
    /// System message.
    pub fn system(text: impl Into<String>) -> Self {
        Self::System {
            content: text.into(),
            provider_options: None,
        }
    }

    /// User message with text content.
    pub fn user(text: impl Into<String>) -> Self {
        Self::User {
            content: UserContent::Text(text.into()),
            provider_options: None,
        }
    }

    /// Assistant message with text content.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::Assistant {
            content: AssistantContent::Text(text.into()),
            provider_options: None,
        }
    }

    /// User message with multi-part content.
    #[must_use]
    pub fn user_parts(parts: Vec<UserContentPart>) -> Self {
        Self::User {
            content: UserContent::Parts(parts),
            provider_options: None,
        }
    }

    /// Assistant message with multi-part content.
    #[must_use]
    pub fn assistant_parts(parts: Vec<AssistantContentPart>) -> Self {
        Self::Assistant {
            content: AssistantContent::Parts(parts),
            provider_options: None,
        }
    }

    /// Tool message.
    #[must_use]
    pub fn tool(parts: ToolContent) -> Self {
        Self::Tool {
            content: parts,
            provider_options: None,
        }
    }

    /// The message's role: `system`, `user`, `assistant` or `tool`.
    #[must_use]
    pub fn role(&self) -> &'static str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::Tool { .. } => "tool",
        }
    }

    /// The message-level provider options.
    #[must_use]
    pub fn provider_options(&self) -> Option<&SharedProviderOptions> {
        match self {
            Self::System {
                provider_options, ..
            }
            | Self::User {
                provider_options, ..
            }
            | Self::Assistant {
                provider_options, ..
            }
            | Self::Tool {
                provider_options, ..
            } => provider_options.as_ref(),
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
