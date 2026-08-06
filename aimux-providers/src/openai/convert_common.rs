//! Shared model-capability / version helpers used by both the OpenAI Chat
//! Completions converter (`convert.rs`) and the Responses API converter
//! (`responses/convert.rs`).
//!
//! These were previously copy-pasted (~140 lines) between the two modules;
//! any change had to be applied twice (issue M10).

/// Parsed GPT version info (mirrors TS `getGptVersion`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GptVersion {
    pub major: u32,
    pub minor: Option<u32>,
    pub variant: Option<String>,
}

/// Extract GPT version from a model ID (e.g. `gpt-5.1-codex` → major=5, minor=1).
pub fn get_gpt_version(model_id: &str) -> Option<GptVersion> {
    let rest = model_id.strip_prefix("gpt-")?;
    let (major_str, remainder) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    if major_str.is_empty() {
        return None;
    }
    let major: u32 = major_str.parse().ok()?;

    let (minor, remainder) = if let Some(stripped) = remainder.strip_prefix('.') {
        let (minor_str, after) = stripped
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| (&stripped[..i], &stripped[i..]))
            .unwrap_or((stripped, ""));
        if minor_str.is_empty() {
            return Some(GptVersion {
                major,
                minor: None,
                variant: if remainder.is_empty() {
                    None
                } else {
                    Some(remainder.trim_start_matches('-').to_string())
                },
            });
        }
        (
            minor_str.parse::<u32>().ok(),
            if after.is_empty() {
                None
            } else {
                Some(after.trim_start_matches('-').to_string())
            },
        )
    } else {
        (
            None,
            if remainder.is_empty() {
                None
            } else {
                Some(remainder.trim_start_matches('-').to_string())
            },
        )
    };

    Some(GptVersion {
        major,
        minor,
        variant: remainder,
    })
}

/// Extract o-series version (e.g. `o3-mini` → 3). Mirrors TS `getOSeriesVersion`.
pub fn get_o_series_version(model_id: &str) -> Option<u32> {
    let rest = model_id.strip_prefix('o')?;
    let (digits, _) = rest
        .find(|c: char| !c.is_ascii_digit())
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, ""));
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

/// Model capabilities relevant to request body construction.
pub struct ModelCapabilities {
    pub is_reasoning_model: bool,
    pub system_message_mode: SystemMessageMode,
    pub supports_flex_processing: bool,
    pub supports_priority_processing: bool,
    pub supports_non_reasoning_parameters: bool,
}

/// How system messages are mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessageMode {
    System,
    Developer,
    Remove,
}

pub fn get_model_capabilities(model_id: &str) -> ModelCapabilities {
    let o_version = get_o_series_version(model_id);
    let gpt_version = get_gpt_version(model_id);
    let is_gpt_chat_model = gpt_version.as_ref().is_some_and(|v| {
        v.minor.is_none() && v.variant.as_deref().is_some_and(|s| s.starts_with("chat"))
    });
    let is_gpt_nano_model = gpt_version
        .as_ref()
        .is_some_and(|v| v.variant.as_deref().is_some_and(|s| s.starts_with("nano")));

    let supports_flex_processing = o_version.is_some_and(|v| v >= 3)
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_chat_model);

    let supports_priority_processing = model_id.starts_with("gpt-4")
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_nano_model && !is_gpt_chat_model)
        || o_version.is_some_and(|v| v >= 3);

    let is_reasoning_model = o_version.is_some()
        || gpt_version
            .as_ref()
            .is_some_and(|v| v.major >= 5 && !is_gpt_chat_model);

    let supports_non_reasoning_parameters = gpt_version
        .as_ref()
        .is_some_and(|v| v.major > 5 || (v.major == 5 && v.minor.unwrap_or(0) >= 1));

    let system_message_mode = if is_reasoning_model {
        SystemMessageMode::Developer
    } else {
        SystemMessageMode::System
    };

    ModelCapabilities {
        is_reasoning_model,
        system_message_mode,
        supports_flex_processing,
        supports_priority_processing,
        supports_non_reasoning_parameters,
    }
}
