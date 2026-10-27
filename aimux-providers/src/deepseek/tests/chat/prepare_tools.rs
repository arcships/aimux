//! Port of `src/chat/deepseek-prepare-tools.test.ts` (`@ai-sdk/deepseek` 3.0.56).
//!
//! Upstream's `supportsStrictToolCalls` is three-state: `undefined` skips both
//! strict checks. The chat model always passes a boolean derived from the base
//! URL (`baseURL.endsWith('/beta')`), so there is no `undefined` to pass. The
//! upstream case "should pass through strict mode for multiple tools with
//! different strict settings" depends on that state and is not ported (no
//! boolean reproduces it; see the report).

use aimux_core::tool::Tool;
use serde_json::{Value, json};

use crate::deepseek::chat::prepare_tools::prepare_tools;

use super::support::*;

/// The tools of a request, read as the request path reads them.
fn tools_from(value: Value) -> Vec<Tool> {
    serde_json::from_value(value).expect("the tools literal is valid")
}

// The describe level `prepareTools` keeps its name; the file is `prepare_tools.rs`.
#[allow(clippy::module_inception)]
mod prepare_tools {
    use super::*;

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should pass through strict mode when strict is true
    #[test]
    fn should_pass_through_strict_mode_when_strict_is_true() {
        let tools = tools_from(json!([{
            "type": "function",
            "name": "testFunction",
            "description": "A test function",
            "inputSchema": { "type": "object", "properties": {} },
            "strict": true,
        }]));

        // Upstream passes no endpoint flag. `true` gives the same result here:
        // an all-strict request raises neither beta-only rejection.
        let result =
            prepare_tools(Some(&tools), None, true).expect("all-strict tools are accepted");

        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": true,
                },
            })])
        );
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should reject strict tools when the endpoint does not support them
    #[test]
    fn should_reject_strict_tools_when_the_endpoint_does_not_support_them() {
        let tools = tools_from(json!([{
            "type": "function",
            "name": "strictTool",
            "inputSchema": { "type": "object", "properties": {} },
            "strict": true,
        }]));

        let error = prepare_tools(Some(&tools), None, false)
            .err()
            .expect("strict tools are rejected without the beta endpoint");

        assert!(
            error
                .to_string()
                .contains("DeepSeek strict tool calls require a beta base URL ending in `/beta`."),
            "{error}"
        );
    }

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should reject mixed strict and non-strict tools on the beta endpoint
    #[test]
    fn should_reject_mixed_strict_and_non_strict_tools_on_the_beta_endpoint() {
        let tools = tools_from(json!([
            {
                "type": "function",
                "name": "strictTool",
                "inputSchema": { "type": "object", "properties": {} },
                "strict": true,
            },
            {
                "type": "function",
                "name": "nonStrictTool",
                "inputSchema": { "type": "object", "properties": {} },
            },
        ]));

        let error = prepare_tools(Some(&tools), None, true)
            .err()
            .expect("mixed strictness is rejected on the beta endpoint");

        assert!(
            error.to_string().contains(
                "DeepSeek strict mode requires every function tool in the request to set `strict: true`."
            ),
            "{error}"
        );
    }

    /// upstream: @ai-sdk/deepseek src/chat/deepseek-prepare-tools.test.ts › prepareTools › should accept all-strict tools on the beta endpoint
    #[test]
    fn should_accept_all_strict_tools_on_the_beta_endpoint() {
        let tools = tools_from(json!([
            {
                "type": "function",
                "name": "firstStrictTool",
                "inputSchema": { "type": "object", "properties": {} },
                "strict": true,
            },
            {
                "type": "function",
                "name": "secondStrictTool",
                "inputSchema": { "type": "object", "properties": {} },
                "strict": true,
            },
        ]));

        let result = prepare_tools(Some(&tools), None, true)
            .expect("all-strict tools are accepted on the beta endpoint");

        let prepared = result.tools.expect("the tools are present");
        assert!(
            prepared
                .iter()
                .all(|tool| tool["function"]["strict"] == json!(true)),
            "every tool keeps strict: true"
        );
    }

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
