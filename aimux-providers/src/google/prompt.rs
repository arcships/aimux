//! `google-prompt.ts`: the converted prompt.

use serde_json::Value;

/// The result of converting a `LanguageModelPrompt` into Google's
/// `contents` + `systemInstruction` shape.
#[derive(Debug, Clone, Default)]
pub struct GooglePrompt {
    /// Top-level `systemInstruction` object, or `None` when there are no
    /// system messages (or for Gemma models, which don't accept it).
    pub system_instruction: Option<Value>,
    /// The `contents` array — one entry per non-system message (tool messages
    /// are folded into the preceding user turn as `functionResponse` parts).
    pub contents: Vec<Value>,
}
