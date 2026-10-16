//! User-facing message types.
//!
//! `ModelMessage` is what the user passes to `generate_text` / `stream_text`:
//! the AI SDK's `ModelMessage` (`@ai-sdk/provider-utils`
//! `types/model-message.ts:10`), a union by role in which every message may
//! carry message-level `provider_options`. It is converted to
//! `LanguageModelPrompt` (provider-facing) before calling
//! `LanguageModel::do_generate` / `do_stream`.

use std::fmt;

use serde::de::{self, Deserializer, SeqAccess, Visitor};
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
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(untagged)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum UserContent {
    Text(String),
    Parts(Vec<UserContentPart>),
}

/// The content of an assistant message: a string or parts (upstream
/// `AssistantContent`).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(untagged)]
#[cfg_attr(feature = "ts-export", ts(export))]
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
#[derive(Debug, Clone, Serialize, TS)]
#[serde(untagged)]
#[cfg_attr(feature = "ts-export", ts(export))]
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

// ── Deserialize by JSON kind ────────────────────────────────────────────────
//
// The three types above serialize untagged, but an untagged `Deserialize`
// tries each variant in turn and, when none fits, reports only "data did not
// match any variant of untagged enum ...": a prompt part with an unknown
// `type` would lose the "unknown variant `x`, expected one of ..." error its
// own union gives (RFC-0042 §7.2, `rfc/0042-v4-type-boundaries.md:211`). They
// are read by JSON kind instead: a string is the text, an array is read as
// the parts or messages, with each element's own error.

/// Read a string or an array of `T` by JSON kind: `text` builds the value
/// from a string, `array` from the array's elements, each decoded with its
/// own error.
fn text_or_array<'de, D, T, U>(
    deserializer: D,
    expecting: &'static str,
    text: fn(String) -> U,
    array: fn(Vec<T>) -> U,
) -> Result<U, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct KindVisitor<T, U> {
        expecting: &'static str,
        text: fn(String) -> U,
        array: fn(Vec<T>) -> U,
    }

    impl<'de, T: Deserialize<'de>, U> Visitor<'de> for KindVisitor<T, U> {
        type Value = U;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.expecting)
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<U, E> {
            Ok((self.text)(value.to_owned()))
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<U, E> {
            Ok((self.text)(value))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<U, A::Error> {
            Vec::deserialize(de::value::SeqAccessDeserializer::new(seq)).map(self.array)
        }
    }

    deserializer.deserialize_any(KindVisitor {
        expecting,
        text,
        array,
    })
}

impl<'de> Deserialize<'de> for UserContent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        text_or_array(
            deserializer,
            "a string or an array of user content parts",
            Self::Text,
            Self::Parts,
        )
    }
}

impl<'de> Deserialize<'de> for AssistantContent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        text_or_array(
            deserializer,
            "a string or an array of assistant content parts",
            Self::Text,
            Self::Parts,
        )
    }
}

impl<'de> Deserialize<'de> for ModelPrompt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        text_or_array(
            deserializer,
            "a prompt string or an array of messages",
            Self::Text,
            Self::Messages,
        )
    }
}
