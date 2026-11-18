use serde_json::{Value, json};

use crate::openai_compatible::completion::convert_completion_usage::usage;

/// `convertOpenAICompatibleCompletionUsage`(usage), as its serialized form.
fn usage_json(raw: &Value) -> Value {
    serde_json::to_value(usage(raw.as_object().cloned())).expect("usage serializes")
}

mod convert_open_ai_compatible_completion_usage {
    use super::*;

    #[test]
    fn keeps_unreported_counts_unknown() {
        // #478 (beyond the AI SDK): a count the provider did not report, absent
        // or null, stays unknown; a reported zero stays zero. The response
        // schema rejects such an object, so only a direct call reaches this.
        for (usage, mut expected) in [
            (json!({}), json!({ "inputTokens": {}, "outputTokens": {} })),
            (
                json!({ "prompt_tokens": 2, "completion_tokens": null }),
                json!({ "inputTokens": { "total": 2, "noCache": 2 }, "outputTokens": {} }),
            ),
            (
                json!({ "completion_tokens": 0 }),
                json!({ "inputTokens": {}, "outputTokens": { "total": 0, "text": 0 } }),
            ),
        ] {
            expected["raw"] = usage.clone();
            let converted = usage_json(&usage);
            assert_eq!(converted, expected, "{usage}");
        }
    }
}
