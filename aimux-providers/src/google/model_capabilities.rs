//! `google-model-capabilities.ts`.

/// Gemini model capabilities, mirroring `getGoogleModelCapabilities` in the TS
/// SDK. Determines which provider-defined tools a model supports and whether the
/// Gemini 3 combined tool shape applies.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoogleModelCapabilities {
    /// `google_search` / `url_context` / `code_execution` / `google_maps` /
    /// `enterprise_web_search` / `vertex_rag_store` require Gemini 2.0+.
    pub supports_gemini_2_tools: bool,
    /// `file_search` requires Gemini 2.5+ or Gemini 3.
    pub supports_file_search: bool,
    /// Gemini 3 keeps function + provider tools together with
    /// `includeServerSideToolInvocations`.
    pub uses_gemini_3_features: bool,
}

/// Classify Gemini capabilities by model id.
///
/// Mirrors `getGoogleModelCapabilities`. Unrecognized Gemini ids inherit the
/// newest supported behaviour (matching the TS intent); only known older
/// generations are downgraded.
#[must_use]
pub fn get_google_model_capabilities(model_id: &str) -> GoogleModelCapabilities {
    let lower = model_id.to_lowercase();

    let is_gemini_model = contains_at_boundary(&lower, "gemini-");
    let is_gemini_2 = matches_prefix_boundary(&lower, "gemini-2");
    let is_gemini_25 = matches_prefix_boundary(&lower, "gemini-2.5");
    let is_gemini_1 = matches_prefix_boundary(&lower, "gemini-1");
    let is_known_pre_gemini_2 = is_gemini_1
        || matches_at_end(&lower, "gemini-pro")
        || matches_at_end(&lower, "gemini-pro-vision")
        || matches_prefix_boundary(&lower, "gemini-robotics-er-1.5");
    let is_known_older_model = is_known_pre_gemini_2 || is_gemini_2;
    let uses_gemini_3_features = is_gemini_model && !is_known_older_model;

    GoogleModelCapabilities {
        supports_gemini_2_tools: (is_gemini_model && !is_known_pre_gemini_2)
            || lower.contains("nano-banana"),
        supports_file_search: is_gemini_25 || uses_gemini_3_features,
        uses_gemini_3_features,
    }
}

/// `/(^|\/)prefix/` — `prefix` appears at the start of `lower` or just after a
/// `/`, with no requirement on what follows.
fn contains_at_boundary(lower: &str, prefix: &str) -> bool {
    lower.starts_with(prefix) || lower.contains(&format!("/{prefix}"))
}

/// `/(^|\/)prefix(?:[.-]|$)/` — `prefix` appears at the start of `lower` or
/// just after a `/`, and is followed by `.`, `-`, or end-of-string.
pub(super) fn matches_prefix_boundary(lower: &str, prefix: &str) -> bool {
    let bytes = lower.as_bytes();
    let plen = prefix.len();
    if plen > bytes.len() {
        return false;
    }
    let pb = prefix.as_bytes();
    for i in 0..=bytes.len() - plen {
        if i != 0 && bytes[i - 1] != b'/' {
            continue;
        }
        if &bytes[i..i + plen] != pb {
            continue;
        }
        let after = i + plen;
        if after >= bytes.len() {
            return true;
        }
        let next = bytes[after];
        if next == b'.' || next == b'-' {
            return true;
        }
    }
    false
}

/// `/(^|\/)suffix$/` — `lower` ends with `suffix`, and the suffix begins at the
/// start of the string or just after a `/`.
fn matches_at_end(lower: &str, suffix: &str) -> bool {
    let bytes = lower.as_bytes();
    let slen = suffix.len();
    if slen > bytes.len() {
        return false;
    }
    let start = bytes.len() - slen;
    if &bytes[start..] != suffix.as_bytes() {
        return false;
    }
    start == 0 || bytes[start - 1] == b'/'
}
