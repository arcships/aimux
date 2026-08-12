// Panic convert wrappers are #[deprecated]; these tests still use them.
#![allow(deprecated)]
//! Rust translations of the AI SDK OpenAI provider pure-function tests.
//!
//! Sources (TS → Rust):
//! - `packages/openai/src/chat/convert-to-openai-chat-messages.test.ts`
//!   → `convert_prompt_to_openai_messages`
//! - `packages/openai/src/chat/openai-chat-prepare-tools.test.ts`
//!   → `prepare_tools`
//! - `packages/openai/src/chat/openai-chat-language-model.test.ts`
//!   (`doGenerate` `requestBodyJson` assertions) → `build_request_body`
//!
//! Tests that depend on features absent from the Rust data model
//! (`providerOptions`, `systemMessageMode`, file URL/reference/audio/PDF
//! parts, reasoning models, etc.) are documented at the bottom of each
//! module rather than translated — see "Remaining untranslated cases".

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::tool::FunctionTool;
use aimux_providers::openai::convert::{
    build_request_body, convert_prompt_to_openai_messages, prepare_tools,
};
use serde_json::{Value, json};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// Build `CallOptions` with everything unset except `prompt`.
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// The JSON schema used across the TS `requestBodyJson` tool tests.
fn value_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"],
        "additionalProperties": false,
        "$schema": "http://json-schema.org/draft-07/schema#"
    })
}

fn sys(content: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::System,
        content: vec![ContentPart::text(content)],
        provider_options: None,
    }
}

fn user_parts(parts: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::User,
        content: parts,
        provider_options: None,
    }
}

fn assistant_text(content: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::Assistant,
        content: vec![ContentPart::text(content)],
        provider_options: None,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convert_prompt_to_openai_messages
// (convert-to-openai-chat-messages.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod convert_messages {
    use super::*;
    use serde_json::json;

    // ── system messages ──────────────────────────────────────────────────────

    /// TS: "should forward system messages"
    #[test]
    fn forwards_system_messages() {
        let prompt = vec![sys("You are a helpful assistant.")];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{ "role": "system", "content": "You are a helpful assistant." }])
        );
    }

    // ── user messages ────────────────────────────────────────────────────────

    /// TS: "should convert messages with only a text part to a string content"
    #[test]
    fn text_only_user_message_becomes_string_content() {
        let prompt = vec![user_parts(vec![ContentPart::text("Hello")])];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{ "role": "user", "content": "Hello" }])
        );
    }

    /// TS: "should convert messages with image parts"
    #[test]
    fn converts_image_parts_to_data_url() {
        let prompt = vec![user_parts(vec![
            ContentPart::text("Hello"),
            ContentPart::image(vec![0, 1, 2, 3], "image/png".to_string()),
        ])];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Hello" },
                    {
                        "type": "image_url",
                        "image_url": { "url": "data:image/png;base64,AAECAw==" }
                    }
                ]
            }])
        );
    }

    /// TS: "should convert messages with Uint8Array image parts to data URLs"
    #[test]
    fn converts_uint8array_image_parts_to_data_urls() {
        let prompt = vec![user_parts(vec![ContentPart::image(
            vec![0xff, 0xd8, 0xff, 0xe0],
            "image/jpeg".to_string(),
        )])];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{
                "role": "user",
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": "data:image/jpeg;base64,/9j/4A==" }
                }]
            }])
        );
    }

    // ── tool calls ───────────────────────────────────────────────────────────

    /// TS: "should stringify arguments to tool calls"
    #[test]
    fn stringifies_tool_call_arguments() {
        let prompt: LanguageModelPrompt = vec![
            LanguageModelPromptMessage {
                role: Role::Assistant,
                content: vec![ContentPart::tool_call(
                    "quux".to_string(),
                    "thwomp".to_string(),
                    json!({ "foo": "bar123" }),
                )],
                ..Default::default()
            },
            LanguageModelPromptMessage {
                role: Role::Tool,
                content: vec![ContentPart::tool_result(
                    "quux".to_string(),
                    json!({ "oof": "321rab" }),
                )],
                ..Default::default()
            },
        ];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "type": "function",
                        "id": "quux",
                        "function": {
                            "name": "thwomp",
                            "arguments": "{\"foo\":\"bar123\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "content": "{\"oof\":\"321rab\"}",
                    "tool_call_id": "quux"
                }
            ])
        );
    }

    /// TS: "should send empty string content for assistant messages with no
    /// tool calls"
    #[test]
    fn assistant_with_empty_text_sends_empty_string_content() {
        let prompt = vec![assistant_text("")];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{ "role": "assistant", "content": "" }])
        );
    }

    /// TS: "should default missing tool call input to an empty object"
    #[test]
    fn missing_tool_call_input_defaults_to_empty_object() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::tool_call(
                "quux".to_string(),
                "thwomp".to_string(),
                Value::Null,
            )],
            ..Default::default()
        }];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([{
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "type": "function",
                    "id": "quux",
                    "function": { "name": "thwomp", "arguments": "{}" }
                }]
            }])
        );
    }

    /// TS: "should handle different tool output types"
    ///
    /// A single tool-role message with two tool-result parts expands into two
    /// separate OpenAI `tool` messages.
    #[test]
    fn handles_different_tool_output_types() {
        let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Tool,
            content: vec![
                ContentPart::tool_result(
                    "text-tool".to_string(),
                    Value::String("Hello world".to_string()),
                ),
                ContentPart::tool_result(
                    "error-tool".to_string(),
                    Value::String("Something went wrong".to_string()),
                ),
            ],
            ..Default::default()
        }];
        let result = convert_prompt_to_openai_messages(&prompt);
        assert_eq!(
            Value::Array(result),
            json!([
                {
                    "role": "tool",
                    "content": "Hello world",
                    "tool_call_id": "text-tool"
                },
                {
                    "role": "tool",
                    "content": "Something went wrong",
                    "tool_call_id": "error-tool"
                }
            ])
        );
    }

    // ── Extended tests in `convert_messages_extended` module below ───────────
}

// ════════════════════════════════════════════════════════════════════════════
// prepare_tools
// (openai-chat-prepare-tools.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod prepare_tools_tests {
    use super::*;
    use serde_json::json;

    /// A single function tool reused across the tool-choice / strict tests.
    fn function_tool(name: &str, desc: &str, strict: Option<bool>) -> FunctionTool {
        FunctionTool {
            name: name.to_string(),
            description: Some(desc.to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict,
            provider_options: None,
            input_examples: None,
        }
    }

    /// TS: "should return undefined tools and toolChoice when tools are null"
    #[test]
    fn null_tools_returns_none() {
        let result = prepare_tools(&None, None);
        assert_eq!(result.tools, None);
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }

    /// TS: "should return undefined tools and toolChoice when tools are empty"
    #[test]
    fn empty_tools_returns_none() {
        let result = prepare_tools(&Some(vec![]), None);
        assert_eq!(result.tools, None);
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }

    /// TS: "should correctly prepare function tools"
    #[test]
    fn prepares_function_tools() {
        let tools = Some(vec![FunctionTool {
            name: "testFunction".to_string(),
            description: Some("A test function".to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
            strict: None,
            provider_options: None,
            input_examples: None,
        }]);
        let result = prepare_tools(&tools, None);

        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} }
                }
            })])
        );
    }

    /// TS: "should handle tool choice 'auto'"
    #[test]
    fn tool_choice_auto() {
        let tools = Some(vec![function_tool("testFunction", "Test", None)]);
        let result = prepare_tools(&tools, Some(&ToolChoice::Auto));
        assert_eq!(result.tool_choice, Some(json!("auto")));
    }

    /// TS: "should handle tool choice 'required'"
    #[test]
    fn tool_choice_required() {
        let tools = Some(vec![function_tool("testFunction", "Test", None)]);
        let result = prepare_tools(&tools, Some(&ToolChoice::Required));
        assert_eq!(result.tool_choice, Some(json!("required")));
    }

    /// TS: "should handle tool choice 'none'"
    #[test]
    fn tool_choice_none() {
        let tools = Some(vec![function_tool("testFunction", "Test", None)]);
        let result = prepare_tools(&tools, Some(&ToolChoice::None));
        assert_eq!(result.tool_choice, Some(json!("none")));
    }

    /// TS: "should handle tool choice 'tool'"
    #[test]
    fn tool_choice_tool() {
        let tools = Some(vec![function_tool("testFunction", "Test", None)]);
        let result = prepare_tools(
            &tools,
            Some(&ToolChoice::Tool {
                tool_name: "testFunction".to_string(),
            }),
        );
        assert_eq!(
            result.tool_choice,
            Some(json!({ "type": "function", "function": { "name": "testFunction" } }))
        );
    }

    /// TS: "should pass through strict mode when strict is true"
    #[test]
    fn strict_true_is_passed_through() {
        let tools = Some(vec![function_tool(
            "testFunction",
            "A test function",
            Some(true),
        )]);
        let result = prepare_tools(&tools, None);
        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": true
                }
            })])
        );
    }

    /// TS: "should pass through strict mode when strict is false"
    #[test]
    fn strict_false_is_passed_through() {
        let tools = Some(vec![function_tool(
            "testFunction",
            "A test function",
            Some(false),
        )]);
        let result = prepare_tools(&tools, None);
        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} },
                    "strict": false
                }
            })])
        );
    }

    /// TS: "should not include strict mode when strict is undefined"
    #[test]
    fn strict_undefined_is_omitted() {
        let tools = Some(vec![function_tool("testFunction", "A test function", None)]);
        let result = prepare_tools(&tools, None);
        assert_eq!(
            result.tools,
            Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "testFunction",
                    "description": "A test function",
                    "parameters": { "type": "object", "properties": {} }
                }
            })])
        );
    }

    /// TS: "should pass through strict mode for multiple tools with different
    /// strict settings"
    #[test]
    fn multiple_tools_with_different_strict_settings() {
        let tools = Some(vec![
            function_tool("strictTool", "A strict tool", Some(true)),
            function_tool("nonStrictTool", "A non-strict tool", Some(false)),
            function_tool("defaultTool", "A tool without strict setting", None),
        ]);
        let result = prepare_tools(&tools, None);
        assert_eq!(
            result.tools,
            Some(vec![
                json!({
                    "type": "function",
                    "function": {
                        "name": "strictTool",
                        "description": "A strict tool",
                        "parameters": { "type": "object", "properties": {} },
                        "strict": true
                    }
                }),
                json!({
                    "type": "function",
                    "function": {
                        "name": "nonStrictTool",
                        "description": "A non-strict tool",
                        "parameters": { "type": "object", "properties": {} },
                        "strict": false
                    }
                }),
                json!({
                    "type": "function",
                    "function": {
                        "name": "defaultTool",
                        "description": "A tool without strict setting",
                        "parameters": { "type": "object", "properties": {} }
                    }
                }),
            ])
        );
    }

    // ── Remaining untranslated cases ─────────────────────────────────────────
    //
    //  * "should add warnings for unsupported tools" — the Rust `FunctionTool`
    //    has no `type` discriminator (it is always a function tool), so there
    //    is no `provider`-type tool to reject and warn about.
}

// ════════════════════════════════════════════════════════════════════════════
// build_request_body
// (openai-chat-language-model.test.ts — doGenerate requestBodyJson assertions)
// ════════════════════════════════════════════════════════════════════════════

mod build_request_body_tests {
    use super::*;
    use serde_json::json;

    /// TS: "should pass the model and the messages"
    #[test]
    fn passes_model_and_messages() {
        let options = default_options(test_prompt());
        let body = build_request_body("gpt-3.5-turbo", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-3.5-turbo",
                "messages": [{ "role": "user", "content": "Hello" }]
            })
        );
    }

    /// RFC-0016 M3: `logprobs` / `topLogprobs` provider options reach the
    /// request body — previously they were silently dropped by the
    /// provider_options whitelist (the only "quietly no-op" option).
    #[test]
    fn passes_logprobs_and_top_logprobs() {
        let mut provider_options = std::collections::HashMap::new();
        provider_options.insert(
            "openai".to_string(),
            json!({ "logprobs": true, "topLogprobs": 3 }),
        );
        let options = CallOptions {
            provider_options: Some(provider_options),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &options, false).unwrap();
        assert_eq!(body["logprobs"], json!(true));
        assert_eq!(body["top_logprobs"], json!(3));
    }

    /// TS: "should pass tools and toolChoice"
    #[test]
    fn passes_tools_and_tool_choice() {
        let options = CallOptions {
            tools: Some(vec![Tool::Function(FunctionTool {
                name: "test-tool".to_string(),
                description: None,
                input_schema: value_schema(),
                strict: None,
                provider_options: None,
                input_examples: None,
            })]),
            tool_choice: ToolChoice::Tool {
                tool_name: "test-tool".to_string(),
            },
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-3.5-turbo",
                "messages": [{ "role": "user", "content": "Hello" }],
                "tool_choice": {
                    "type": "function",
                    "function": { "name": "test-tool" }
                },
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "test-tool",
                        "parameters": value_schema()
                    }
                }]
            })
        );
    }

    /// TS: "should not send a response_format when response format is text"
    #[test]
    fn text_response_format_is_omitted() {
        let options = CallOptions {
            response_format: Some(ResponseFormat::Text),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }]
            })
        );
    }

    /// TS: "should forward json response format as 'json_object' without schema"
    #[test]
    fn json_without_schema_becomes_json_object() {
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: None,
                name: None,
                description: None,
            }),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "response_format": { "type": "json_object" }
            })
        );
    }

    /// TS: "should forward json response format as 'json_object' and include
    /// schema"
    #[test]
    fn json_with_schema_uses_json_schema() {
        let schema = value_schema();
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(schema.clone()),
                name: None,
                description: None,
            }),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "schema": schema,
                        "name": "response",
                        "strict": true
                    }
                }
            })
        );
    }

    /// TS: "should use json_schema & strict with responseFormat json"
    ///
    /// Same shape as above; the TS repeats the assertion to confirm strict
    /// mode is used even without an explicit name/description.
    #[test]
    fn json_schema_strict_without_name() {
        let schema = value_schema();
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(schema.clone()),
                name: None,
                description: None,
            }),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body["response_format"],
            json!({
                "type": "json_schema",
                "json_schema": {
                    "schema": schema,
                    "name": "response",
                    "strict": true
                }
            })
        );
    }

    /// TS: "should set name & description with responseFormat json"
    #[test]
    fn json_schema_sets_name_and_description() {
        let schema = value_schema();
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: Some(schema.clone()),
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "schema": schema,
                        "name": "test-name",
                        "description": "test description",
                        "strict": true
                    }
                }
            })
        );
    }

    /// TS: "should allow for undefined schema with responseFormat json when
    /// structuredOutputs are enabled"
    #[test]
    fn json_without_schema_even_with_name_and_description() {
        let options = CallOptions {
            response_format: Some(ResponseFormat::Json {
                schema: None,
                name: Some("test-name".to_string()),
                description: Some("test description".to_string()),
            }),
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "response_format": { "type": "json_object" }
            })
        );
    }

    /// TS: "should set strict with tool call"
    ///
    /// Tools with `toolChoice: required` and a description.
    #[test]
    fn strict_with_tool_call_required() {
        let options = CallOptions {
            tools: Some(vec![Tool::Function(FunctionTool {
                name: "test-tool".to_string(),
                description: Some("test description".to_string()),
                input_schema: value_schema(),
                strict: None,
                provider_options: None,
                input_examples: None,
            })]),
            tool_choice: ToolChoice::Required,
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "tool_choice": "required",
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "test-tool",
                        "description": "test description",
                        "parameters": value_schema()
                    }
                }]
            })
        );
    }

    /// TS: "should set strict for tool usage"
    ///
    /// Tools with `toolChoice: tool` (no description).
    #[test]
    fn strict_for_tool_usage() {
        let options = CallOptions {
            tools: Some(vec![Tool::Function(FunctionTool {
                name: "test-tool".to_string(),
                description: None,
                input_schema: value_schema(),
                strict: None,
                provider_options: None,
                input_examples: None,
            })]),
            tool_choice: ToolChoice::Tool {
                tool_name: "test-tool".to_string(),
            },
            ..default_options(test_prompt())
        };
        let body = build_request_body("gpt-4o-2024-08-06", &options, false).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4o-2024-08-06",
                "messages": [{ "role": "user", "content": "Hello" }],
                "tool_choice": {
                    "type": "function",
                    "function": { "name": "test-tool" }
                },
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "test-tool",
                        "parameters": value_schema()
                    }
                }]
            })
        );
    }

    // ── Remaining untranslated requestBodyJson cases ────────────────────────
    //
    //  * "should pass settings" (logitBias, parallelToolCalls, user) — these
    //    come from `providerOptions.openai`, which the Rust `CallOptions` does
    //    not surface as individual fields.
    //  * reasoning_effort / reasoning model tests — the Rust `CallOptions` has
    //    no `reasoning` field and `build_request_body` has no reasoning-model
    //    detection (which would remap `max_tokens` → `max_completion_tokens`
    //    and strip temperature/topP/frequencyPenalty/presencePenalty).
    //  * textVerbosity — `providerOptions.openai.textVerbosity`.
    //  * systemMessageMode 'developer' for o1 / forceReasoning models.
}
