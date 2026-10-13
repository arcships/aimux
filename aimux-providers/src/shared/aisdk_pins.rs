//! The `@ai-sdk/*` package versions whose user-agent each provider sends
//! (`ai-sdk-{package}/{version}`). Each must equal its pin in
//! `fixtures/aisdk/VERSIONS.json`; the test below fails on drift. They are
//! constants rather than read at build time because the published crate does
//! not ship `fixtures/`.

pub(crate) const ANTHROPIC: &str = "4.0.71";
pub(crate) const GOOGLE: &str = "4.0.87";
pub(crate) const OPENAI: &str = "4.0.83";
pub(crate) const OPENAI_COMPATIBLE: &str = "3.0.62";
pub(crate) const AMAZON_BEDROCK: &str = "5.0.100";
pub(crate) const AZURE: &str = "4.0.84";
pub(crate) const COHERE: &str = "4.0.52";
pub(crate) const DEEPSEEK: &str = "3.0.56";
pub(crate) const GOOGLE_VERTEX: &str = "5.0.98";
pub(crate) const GROQ: &str = "4.0.52";
pub(crate) const MISTRAL: &str = "4.0.54";
pub(crate) const XAI: &str = "5.0.12";

#[cfg(test)]
#[path = "../../../fixtures/aisdk/pinned_versions.rs"]
mod pinned_versions;

#[cfg(test)]
mod tests {
    use super::pinned_versions;

    #[test]
    fn versions_equal_the_aisdk_pins() {
        for (package, version) in [
            ("@ai-sdk/anthropic", super::ANTHROPIC),
            ("@ai-sdk/google", super::GOOGLE),
            ("@ai-sdk/openai", super::OPENAI),
            ("@ai-sdk/openai-compatible", super::OPENAI_COMPATIBLE),
            ("@ai-sdk/amazon-bedrock", super::AMAZON_BEDROCK),
            ("@ai-sdk/azure", super::AZURE),
            ("@ai-sdk/cohere", super::COHERE),
            ("@ai-sdk/deepseek", super::DEEPSEEK),
            ("@ai-sdk/google-vertex", super::GOOGLE_VERTEX),
            ("@ai-sdk/groq", super::GROQ),
            ("@ai-sdk/mistral", super::MISTRAL),
            ("@ai-sdk/xai", super::XAI),
        ] {
            assert_eq!(pinned_versions::pinned(package), version, "{package}");
        }
    }

    /// Every VERSIONS.json entry is the `{version, integrity}` object form.
    #[test]
    fn every_versions_entry_is_object_form() {
        // `all()` panics on any other shape.
        assert!(pinned_versions::all().len() >= 15);
    }
}
