//! Ports of `src/chat/deepseek-prepare-tools.test.ts` from `@ai-sdk/deepseek@3.0.56`,
//! observed through the request body.

use super::support::*;

// The file and its describe block are both named prepareTools upstream.
#[allow(clippy::module_inception)]
mod prepare_tools {
    use super::*;

    /// A tool with `strict: false` should produce `"strict": false` in the
    /// serialized tool definition.
    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should pass through strict mode when strict is false
    #[tokio::test]
    async fn should_pass_through_strict_mode_when_strict_is_false() {
        let tool = FunctionTool {
            name: "testFunction".to_string(),
            description: Some("A test function".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: Some(false),
            provider_options: None,
            input_examples: None,
        };
        let mut options = options();
        options.tools = Some(vec![Tool::from(tool)]);
        let body = generate_body("deepseek-chat", &options).await;

        assert_eq!(
            body["tools"],
            json!([{
                "type": "function",
                "function": {
                    "description": "A test function",
                    "name": "testFunction",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": false
                }
            }])
        );
    }

    /// A tool without `strict` set should NOT have a `"strict"` field in the
    /// serialized tool definition.
    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should not include strict mode when strict is undefined
    #[tokio::test]
    async fn should_not_include_strict_mode_when_strict_is_undefined() {
        let tool = FunctionTool {
            name: "testFunction".to_string(),
            description: Some("A test function".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: None,
            provider_options: None,
            input_examples: None,
        };
        let mut options = options();
        options.tools = Some(vec![Tool::from(tool)]);
        let body = generate_body("deepseek-chat", &options).await;

        assert_eq!(
            body["tools"],
            json!([{
                "type": "function",
                "function": {
                    "description": "A test function",
                    "name": "testFunction",
                    "parameters": { "type": "object", "properties": {} }
                }
            }])
        );
        // Verify strict is absent.
        assert!(
            body["tools"][0]["function"].get("strict").is_none(),
            "strict should be absent"
        );
    }
}
