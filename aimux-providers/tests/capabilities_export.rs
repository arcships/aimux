//! The OpenAI model-capability classifier is part of the public API
//! (arcships/aimux#490): callers wiring their own OpenAI-shaped endpoints
//! through `create_openai_compatible` reuse it instead of copying the table.

use aimux_providers::openai::language_model_capabilities::{
    GptVersion, ModelCapabilities, SystemMessageMode, get_gpt_version, get_model_capabilities,
};

#[test]
fn the_model_capability_classifier_is_reachable() {
    let reasoning: ModelCapabilities = get_model_capabilities("gpt-5");
    assert!(reasoning.is_reasoning_model);
    assert_eq!(reasoning.system_message_mode, SystemMessageMode::Developer);

    let plain = get_model_capabilities("gpt-4o");
    assert!(!plain.is_reasoning_model);
    assert_eq!(plain.system_message_mode, SystemMessageMode::System);

    assert_eq!(
        get_gpt_version("gpt-5.1-codex"),
        Some(GptVersion {
            major: 5,
            minor: Some(1),
            variant: Some("codex".into())
        })
    );
}
