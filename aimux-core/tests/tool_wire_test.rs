//! V4 tool types that constrain their JSON beyond its shape.

use aimux_core::tool::{ProviderTool, Tool};
use aimux_core::types::NamespacedKind;
use serde_json::json;

/// `LanguageModelV4ProviderTool.id` is `${string}.${string}`
/// (`@ai-sdk/provider@4.0.21
/// src/language-model/v4/language-model-v4-provider-tool.ts:17`).
#[test]
fn a_provider_tool_id_needs_a_namespace() {
    let tool: Tool = serde_json::from_value(json!({
        "type": "provider",
        "id": "anthropic.web_search_20250305",
        "name": "web_search",
        "args": {},
    }))
    .unwrap();
    let Tool::Provider(ProviderTool { id, .. }) = &tool else {
        panic!("expected a provider tool");
    };
    assert_eq!(id, "anthropic.web_search_20250305");
    assert_eq!(
        serde_json::to_value(&tool).unwrap()["id"],
        "anthropic.web_search_20250305"
    );

    let err = serde_json::from_value::<Tool>(json!({
        "type": "provider",
        "id": "web_search",
        "name": "web_search",
        "args": {},
    }))
    .unwrap_err();
    assert!(err.to_string().contains("<provider-id>.<name>"), "{err}");
}

/// The template literal type admits any string containing a `.`, empty
/// segments included.
#[test]
fn a_namespaced_kind_is_any_text_with_a_dot() {
    for kind in ["openai.web_search", "a.b.c", ".", "x.", ".y"] {
        assert_eq!(NamespacedKind::new(kind).unwrap().as_str(), kind);
    }
    for kind in ["", "web_search"] {
        assert!(NamespacedKind::new(kind).is_err(), "{kind:?}");
        assert!(serde_json::from_value::<NamespacedKind>(json!(kind)).is_err());
    }
}
