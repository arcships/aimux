//! The provider-options namespace and profile of the Responses model
//! (openai-responses-language-model-options.ts).

use aimux_core::shared::{JsonObject, SharedProviderOptions};
use serde_json::Value;

/// The providerOptions keys a Responses model reads, in order of precedence
/// (the first one present wins as a whole), and the key it writes response
/// metadata under. The AI SDK's `OpenAIResponsesLanguageModel` picks
/// `providerOptionsName` from the host: `openai` for the OpenAI API, `azure`
/// for Azure (which still falls back to `openai` when no `azure` options were
/// given).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsesNamespace {
    read: &'static [&'static str],
    write: &'static str,
}

impl ResponsesNamespace {
    /// The OpenAI API: reads and writes `openai`.
    pub const OPENAI: Self = Self::new(&["openai"], "openai");

    /// A host namespace. `read` is non-empty and in order of precedence;
    /// `write` is the metadata key.
    #[must_use]
    pub const fn new(read: &'static [&'static str], write: &'static str) -> Self {
        Self { read, write }
    }

    /// The key response metadata is written under.
    #[must_use]
    pub fn write_key(self) -> &'static str {
        self.write
    }

    /// The first present options object among the read keys.
    pub(crate) fn find(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }

    /// [`find`](Self::find) for `CallOptions::provider_options`.
    pub(crate) fn find_in(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }
}

impl Default for ResponsesNamespace {
    fn default() -> Self {
        Self::OPENAI
    }
}

/// How a host differs in the Responses API model: its namespace, and the
/// prefixes that mark a file part's data as an uploaded file id instead of
/// content to inline (`fileIdPrefixes` in the AI SDK).
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponsesProfile {
    pub(crate) namespace: ResponsesNamespace,
    /// Empty: no file part is treated as a file id.
    pub(crate) file_id_prefixes: Vec<&'static str>,
    pub(crate) explicit_message_item_type: bool,
}

/// Get a value from the host namespace's options (`openai.<key>` for OpenAI).
pub(crate) fn openai_option(
    ns: ResponsesNamespace,
    options: &Option<SharedProviderOptions>,
    key: &str,
) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| ns.find_in(m))
        .and_then(|o| o.get(key))
        .cloned()
}
