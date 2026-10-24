use serde_json::{Value, json};

use crate::google::convert::get_google_model_capabilities;

/// The capabilities as the upstream `{ supportsGemini2Tools, ... }` object.
fn capabilities_json(model_id: &str) -> Value {
    let capabilities = get_google_model_capabilities(model_id);
    json!({
        "supportsGemini2Tools": capabilities.supports_gemini_2_tools,
        "supportsFileSearch": capabilities.supports_file_search,
        "usesGemini3Features": capabilities.uses_gemini_3_features,
    })
}

mod get_google_model_capabilities {
    use super::*;

    /// upstream: @ai-sdk/google src/google-model-capabilities.test.ts › getGoogleModelCapabilities › classifies $modelId without falling back to legacy behavior
    #[test]
    fn classifies_model_id_without_falling_back_to_legacy_behavior() {
        let cases = [
            (
                "gemini-pro",
                json!({ "supportsGemini2Tools": false, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
            (
                "gemini-pro-vision",
                json!({ "supportsGemini2Tools": false, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
            (
                "gemini-1.5-flash",
                json!({ "supportsGemini2Tools": false, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
            (
                "gemini-robotics-er-1.5-preview",
                json!({ "supportsGemini2Tools": false, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
            (
                "gemini-2.0-flash",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
            (
                "gemini-2.5-flash",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": true, "usesGemini3Features": false }),
            ),
            (
                "gemini-3.1-pro-preview",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": true, "usesGemini3Features": true }),
            ),
            (
                // Use a deliberately distant generation so this remains an unknown
                // future-model fixture as new Gemini generations are released.
                "gemini-99-pro-preview",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": true, "usesGemini3Features": true }),
            ),
            (
                "gemini-ultra-latest",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": true, "usesGemini3Features": true }),
            ),
            (
                "nano-banana-pro-preview",
                json!({ "supportsGemini2Tools": true, "supportsFileSearch": false, "usesGemini3Features": false }),
            ),
        ];

        for (model_id, expected) in cases {
            assert_eq!(capabilities_json(model_id), expected, "{model_id}");
        }
    }
}
