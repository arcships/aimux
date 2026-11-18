//! `openaiLanguageModelChatOptions` (openai-chat-language-model-options.ts)
//! as `parse_chat_options` reads it. Not upstream cases: upstream checks the
//! schema through zod, which has no test file of its own.

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;

use crate::openai::chat::chat_language_model_options::parse_chat_options;

fn parse(options: Value) -> Result<Value, AiMuxError> {
    parse_chat_options(options.as_object().unwrap()).map(Value::Object)
}

#[test]
fn rejects_values_outside_the_schema_with_the_serde_cause() {
    for options in [
        json!({ "reasoningEffort": "extreme" }),
        json!({ "serviceTier": null }),
        json!({ "logprobs": "yes" }),
        json!({ "metadata": { "key": 1 } }),
        json!({ "metadata": { ("k".repeat(65)): "v" } }),
        json!({ "promptCacheOptions": { "ttl": "1h" } }),
        json!({ "promptCacheRetention": "forever" }),
        json!({ "store": "true" }),
        // `z.record(z.coerce.number<string>(), z.number())`: a value that is
        // not a number, a key `Number()` reads as `NaN` or `Infinity`, and
        // `null` (`.optional()`) all fail in the schema, with its cause.
        json!({ "logitBias": { "50256": "-100" } }),
        json!({ "logitBias": { "50256": null } }),
        json!({ "logitBias": { "token": -100 } }),
        json!({ "logitBias": { "0x": -100 } }),
        json!({ "logitBias": { "1e999": -100 } }),
        json!({ "logitBias": null }),
        json!({ "logitBias": [1] }),
    ] {
        let error = parse(options.clone()).unwrap_err();
        let AiMuxError::InvalidArgument { message, cause } = &error else {
            panic!("expected InvalidArgument for {options}, got {error:?}");
        };
        assert_eq!(
            message,
            "Invalid argument for parameter providerOptions: invalid openai provider options",
            "{options}"
        );
        assert!(cause.is_some(), "{options}");
    }
}

/// Unknown keys are dropped, at the top level and in `promptCacheOptions`;
/// `logitBias` keys are written back as numbers.
#[test]
fn keeps_the_declared_keys() {
    let parsed = parse(json!({
        "reasoningEffort": "none",
        "logprobs": 2,
        "promptCacheOptions": { "mode": "explicit", "ttl": "30m", "extra": true },
        "logitBias": { "0x10": 5 },
        "unknown": 1,
    }))
    .unwrap();

    assert_eq!(
        parsed,
        json!({
            "reasoningEffort": "none",
            "logprobs": 2,
            "promptCacheOptions": { "mode": "explicit", "ttl": "30m" },
            "logitBias": { "16": 5 },
        })
    );
}
