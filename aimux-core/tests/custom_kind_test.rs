//! The custom content `kind` is a `${string}.${string}` identifier at every
//! layer, as in the AI SDK:
//! - prompt part: `@ai-sdk/provider@4.0.21`
//!   `src/language-model/v4/language-model-v4-prompt.ts:138`;
//! - model output: `language-model-v4-custom-content.ts:13`;
//! - user message part: `@ai-sdk/provider-utils@5.0.53`
//!   `src/types/content-part.ts:133`;
//! - core result and stream parts: `ai@7.0.127`
//!   `src/generate-text/content-part.ts:16`, `stream-text-result.ts:475`.

use aimux_core::content::AssistantContentPart;
use aimux_core::language_model_message::{
    AssistantPart, CustomPart, LanguageModelMessage, convert_to_language_model_prompt,
};
use aimux_core::message::{AssistantContent, ModelMessage};
use aimux_core::result::{ContentPart, GenerateContent};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::NamespacedKind;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// `value` deserializes as `T` and serializes back to itself.
fn round_trips<T: Serialize + DeserializeOwned>(value: &Value) {
    let parsed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{value} should deserialize: {error}"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), *value);
}

/// `value` does not deserialize as `T`, and the error names the shape.
fn is_rejected<T: DeserializeOwned + std::fmt::Debug>(value: &Value) {
    let error = serde_json::from_value::<T>(value.clone()).unwrap_err();
    assert!(
        error.to_string().contains("`<provider-id>.<name>`"),
        "{value}: {error}"
    );
}

#[test]
fn a_custom_kind_needs_a_namespace() {
    let user = |kind: &str| json!({ "type": "custom", "kind": kind });
    let prompt = |kind: &str| json!({ "kind": kind });
    let generated = |kind: &str| json!({ "type": "custom", "kind": kind });

    round_trips::<AssistantContentPart>(&user("openai.compaction"));
    round_trips::<CustomPart>(&prompt("openai.compaction"));
    round_trips::<GenerateContent>(&generated("openai.compaction"));
    round_trips::<ContentPart>(&generated("openai.compaction"));
    round_trips::<StreamPart>(&generated("openai.compaction"));

    is_rejected::<AssistantContentPart>(&user("compaction"));
    is_rejected::<CustomPart>(&prompt("compaction"));
    is_rejected::<GenerateContent>(&generated("compaction"));
    is_rejected::<ContentPart>(&generated("compaction"));
    is_rejected::<StreamPart>(&generated("compaction"));
}

#[test]
fn a_message_with_an_unnamespaced_custom_kind_does_not_deserialize() {
    let message = |kind: &str| {
        json!({
            "role": "assistant",
            "content": [{ "type": "custom", "kind": kind }],
        })
    };
    assert!(serde_json::from_value::<ModelMessage>(message("openai.compaction")).is_ok());
    assert!(serde_json::from_value::<ModelMessage>(message("compaction")).is_err());
}

#[test]
fn a_custom_part_reaches_the_provider_with_its_kind() {
    let kind = NamespacedKind::new("openai.compaction").unwrap();
    let messages = [ModelMessage::Assistant {
        content: AssistantContent::Parts(vec![AssistantContentPart::Custom {
            kind: kind.clone(),
            provider_options: None,
        }]),
        provider_options: None,
    }];

    let prompt = convert_to_language_model_prompt(&messages, None, None).unwrap();

    let [LanguageModelMessage::Assistant { content, .. }] = prompt.as_slice() else {
        panic!("expected one assistant message: {prompt:?}");
    };
    assert_eq!(
        content.as_slice(),
        [AssistantPart::Custom(CustomPart {
            kind,
            provider_options: None,
        })]
    );
}
