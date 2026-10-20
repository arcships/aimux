//! Which Claude models on Bedrock support strict tools and native structured
//! output (amazon-bedrock-anthropic-model-support.ts).

// ── Tools ───────────────────────────────────────────────────────────────────

/// Whether a Bedrock model id supports the `strict` tool schema field.
///
/// Mirrors the TS `supportsStrictTools`: the newest Claude models reject
/// `strict` (and `output_config.format`), so the field is omitted for them.
#[must_use]
pub fn supports_strict_tools(model_id: &str) -> bool {
    !MODELS_WITHOUT_STRICT_TOOL_SUPPORT
        .iter()
        .any(|m| model_id.contains(m))
}

const MODELS_WITHOUT_STRICT_TOOL_SUPPORT: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-fable-5",
    "claude-sonnet-5",
];

pub(super) fn supports_native_structured_output(model_id: &str) -> bool {
    let prefix = MODELS_WITHOUT_STRICT_TOOL_SUPPORT[0]
        .split_once('-')
        .unwrap()
        .0;
    supports_strict_tools(model_id)
        && !["sonnet-4-6", "haiku-4-5"]
            .iter()
            .any(|suffix| model_id.contains(&format!("{prefix}-{suffix}")))
}
