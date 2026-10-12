//! Models that support Groq's browser search tool.
//!
//! Mirrors `groq-browser-search-models.ts`. Based on
//! <https://console.groq.com/docs/browser-search>.

/// Models that support browser search functionality.
pub(crate) const BROWSER_SEARCH_SUPPORTED_MODELS: [&str; 2] =
    ["openai/gpt-oss-20b", "openai/gpt-oss-120b"];

/// Whether a model supports browser search functionality.
pub(crate) fn is_browser_search_supported_model(model_id: &str) -> bool {
    BROWSER_SEARCH_SUPPORTED_MODELS.contains(&model_id)
}

/// A formatted list of the supported models for warning messages.
pub(crate) fn get_supported_models_string() -> String {
    BROWSER_SEARCH_SUPPORTED_MODELS.join(", ")
}
