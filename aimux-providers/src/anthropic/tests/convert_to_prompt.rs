//! Port of `@ai-sdk/anthropic` 4.0.71 `src/convert-to-anthropic-prompt.test.ts`.
//!
//! Upstream calls `convertToAnthropicPrompt` directly; the Rust counterpart is
//! `convert_prompt_to_anthropic_full_with_tools`, which returns the
//! `system` + `messages` prompt together with the betas and the warnings
//! (upstream pushes those into the `warnings` array it passes in, and the
//! cache-control validator's warnings are merged into the same list).
//! Prompts are written as the V4 JSON they are in upstream and deserialized
//! into `LanguageModelPrompt`; `new URL(x)` is the URL string and
//! `undefined`-valued keys are omitted.

use std::collections::{BTreeSet, HashMap};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;
use serde_json::{Value, json};

use crate::anthropic::convert_to_prompt::{
    AnthropicPromptConversion, convert_prompt_to_anthropic_full_with_tools,
};
use crate::anthropic::language_model::anthropic_tool_name_mapping;
use crate::anthropic::language_model_options::CANONICAL;
use aimux_provider_utils::ToolNameMapping;

/// The V4 prompt written as upstream's JSON.
fn prompt(value: Value) -> LanguageModelPrompt {
    serde_json::from_value(value).expect("valid LanguageModelV4Prompt")
}

/// `createToolNameMapping({ tools, providerToolNames })` for the given V4
/// tools (the provider tool names are the language model's Anthropic table).
fn mapping(tools: Value) -> ToolNameMapping {
    let tools: Vec<Tool> = serde_json::from_value(tools).expect("valid tools");
    anthropic_tool_name_mapping(Some(&tools))
}

fn try_convert_with(
    prompt_json: Value,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    convert_prompt_to_anthropic_full_with_tools(
        &prompt(prompt_json),
        send_reasoning,
        tool_names,
        toolset_names,
    )
}

fn try_convert(
    prompt_json: Value,
    send_reasoning: bool,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    try_convert_with(
        prompt_json,
        send_reasoning,
        &ToolNameMapping::default(),
        &HashMap::new(),
    )
}

/// `convertToAnthropicPrompt` with the default (empty) tool name mapping.
fn convert(prompt_json: Value, send_reasoning: bool) -> AnthropicPromptConversion {
    try_convert(prompt_json, send_reasoning).expect("conversion succeeds")
}

fn convert_with(
    prompt_json: Value,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
) -> AnthropicPromptConversion {
    try_convert_with(prompt_json, send_reasoning, tool_names, toolset_names)
        .expect("conversion succeeds")
}

/// A `Set` of betas. Upstream compares them with `toEqual(new Set(...))`,
/// which ignores insertion order, so both sides are sorted (see [`betas`]).
fn set(items: &[&str]) -> Value {
    let items: BTreeSet<&str> = items.iter().copied().collect();
    json!(items)
}

/// The conversion's betas, sorted like [`set`].
fn betas(conversion: &AnthropicPromptConversion) -> Value {
    let items: BTreeSet<&str> = conversion.betas.iter().map(String::as_str).collect();
    json!(items)
}

/// Upstream's `{ prompt: { system, messages }, betas }` result.
fn result(conversion: &AnthropicPromptConversion) -> Value {
    let mut prompt = json!({ "messages": conversion.messages });
    if let Some(system) = &conversion.system {
        prompt["system"] = json!(system);
    }
    json!({ "prompt": prompt, "betas": betas(conversion) })
}

fn messages(conversion: &AnthropicPromptConversion) -> Value {
    json!(conversion.messages)
}

fn system(conversion: &AnthropicPromptConversion) -> Option<Value> {
    conversion.system.as_ref().map(|system| json!(system))
}

fn warnings(conversion: &AnthropicPromptConversion) -> Value {
    serde_json::to_value(&conversion.warnings).expect("warnings serialize")
}

fn has_beta(conversion: &AnthropicPromptConversion, beta: &str) -> bool {
    conversion.betas.contains(beta)
}

/// `expect(actual).toMatchObject(expected)`: every key of `expected` is in
/// `actual` with a matching value (arrays match element-wise, same length).
fn matches_object(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual| matches_object(actual, value))
        }),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| matches_object(actual, expected))
        }
        _ => actual == expected,
    }
}

#[track_caller]
fn assert_matches_object(actual: &Value, expected: &Value) {
    assert!(
        matches_object(actual, expected),
        "expected {actual:#} to match {expected:#}"
    );
}

/// The block of `content` whose `id` is `id`.
fn find_by_id<'a>(content: &'a Value, id: &str) -> Option<&'a Value> {
    content
        .as_array()
        .expect("content array")
        .iter()
        .find(|part| part.get("id").and_then(Value::as_str) == Some(id))
}

fn other_warning(message: &str) -> Warning {
    Warning::Other {
        message: message.to_string(),
    }
}

mod orphaned_programmatic_callers {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › orphaned programmatic callers › should normalize historical %s callers without changing calls or results
    #[test]
    fn should_normalize_historical_callers_without_changing_calls_or_results() {
        for caller_type in ["code_execution_20250825", "code_execution_20260120"] {
            let prompt_json = json!([
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "lookup-call",
                        "toolName": "lookup",
                        "input": { "ticker": "AAPL" },
                        "providerOptions": {
                            CANONICAL: {
                                "caller": { "type": caller_type, "toolId": "pruned-source" },
                                "cacheControl": { "type": "ephemeral" },
                            },
                        },
                    }],
                },
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "lookup-call",
                        "toolName": "lookup",
                        "output": { "type": "json", "value": { "price": 185.42 } },
                    }],
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "The price is $185.42." }],
                },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Now look up MSFT." }],
                },
            ]);
            let original_prompt = prompt(prompt_json.clone());
            let input = original_prompt.clone();
            let result = convert_prompt_to_anthropic_full_with_tools(
                &input,
                false,
                &ToolNameMapping::default(),
                &HashMap::new(),
            )
            .unwrap();

            assert_eq!(
                messages(&result),
                json!([
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": "lookup-call",
                            "name": "lookup",
                            "input": { "ticker": "AAPL" },
                            "cache_control": { "type": "ephemeral" },
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "lookup-call",
                            "content": "{\"price\":185.42}",
                        }],
                    },
                    {
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "The price is $185.42." }],
                    },
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "Now look up MSFT." }],
                    },
                ]),
                "{caller_type}"
            );
            assert_eq!(
                result.warnings,
                vec![other_warning(
                    "Omitted caller metadata for tool lookup-call because source code execution tool pruned-source is missing from the conversation history."
                )],
                "{caller_type}"
            );
            assert_eq!(input, original_prompt, "{caller_type}");
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › orphaned programmatic callers › should preserve callers in an active continuation (source present: %s)
    #[test]
    fn should_preserve_callers_in_an_active_continuation_source_present() {
        for include_source in [false, true] {
            let mut assistant_content = Vec::new();
            if include_source {
                assistant_content.push(json!({
                    "type": "tool-call",
                    "toolCallId": "source-call",
                    "toolName": "code_execution",
                    "providerExecuted": true,
                    "input": { "type": "programmatic-tool-call", "code": "await lookup({})" },
                }));
            }
            assistant_content.push(json!({
                "type": "tool-call",
                "toolCallId": "lookup-call",
                "toolName": "lookup",
                "input": {},
                "providerOptions": {
                    CANONICAL: {
                        "caller": { "type": "code_execution_20260120", "toolId": "source-call" },
                    },
                },
            }));
            let result = convert(
                json!([
                    { "role": "user", "content": [{ "type": "text", "text": "Look up AAPL." }] },
                    { "role": "assistant", "content": assistant_content },
                    {
                        "role": "tool",
                        "content": [{
                            "type": "tool-result",
                            "toolCallId": "lookup-call",
                            "toolName": "lookup",
                            "output": { "type": "text", "value": "185.42" },
                        }],
                    },
                ]),
                false,
            );

            let last = result.messages[1]["content"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .clone();
            assert_matches_object(
                &last,
                &json!({
                    "type": "tool_use",
                    "caller": { "type": "code_execution_20260120", "tool_id": "source-call" },
                }),
            );
            assert_eq!(warnings(&result), json!([]), "{include_source}");
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › orphaned programmatic callers › should resolve historical sources from emitted code execution blocks: $toolName
    #[test]
    fn should_resolve_historical_sources_from_emitted_code_execution_blocks() {
        for (tool_name, provider_executed, keeps_caller) in [
            ("run_code", true, true),
            ("code_execution", false, false),
            ("web_search", true, false),
            ("unsupported", true, false),
        ] {
            let result = convert_with(
                json!([
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolCallId": "source-call",
                            "toolName": tool_name,
                            "providerExecuted": provider_executed,
                            "input": { "type": "programmatic-tool-call", "code": "await lookup({})" },
                            "providerOptions": { CANONICAL: { "caller": { "type": "direct" } } },
                        }],
                    },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolCallId": "lookup-call",
                            "toolName": "lookup",
                            "input": {},
                            "providerOptions": {
                                CANONICAL: {
                                    "caller": { "type": "code_execution_20260120", "toolId": "source-call" },
                                },
                            },
                        }],
                    },
                    { "role": "user", "content": [{ "type": "text", "text": "Continue." }] },
                ]),
                false,
                &mapping(json!([{
                    "type": "provider",
                    "id": "anthropic.code_execution_20260120",
                    "name": "run_code",
                    "args": {},
                }])),
                &HashMap::new(),
            );

            let content = &result.messages[0]["content"];
            let lookup = find_by_id(content, "lookup-call").expect(tool_name);
            assert_matches_object(lookup, &json!({ "type": "tool_use", "id": "lookup-call" }));
            if keeps_caller {
                assert_eq!(
                    lookup.get("caller"),
                    Some(&json!({ "type": "code_execution_20260120", "tool_id": "source-call" })),
                    "{tool_name}"
                );
                assert_eq!(warnings(&result), json!([]), "{tool_name}");
            } else {
                assert!(lookup.get("caller").is_none(), "{tool_name}: {lookup}");
                assert!(
                    result.warnings.contains(&other_warning(
                        "Omitted caller metadata for tool lookup-call because source code execution tool source-call is missing from the conversation history."
                    )),
                    "{tool_name}: {:?}",
                    result.warnings
                );
            }
            if tool_name != "unsupported" {
                assert_eq!(
                    find_by_id(content, "source-call").and_then(|part| part.get("caller")),
                    Some(&json!({ "type": "direct" })),
                    "{tool_name}"
                );
            }
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › orphaned programmatic callers › should normalize callers on historical server calls and results
    #[test]
    fn should_normalize_callers_on_historical_server_calls_and_results() {
        let caller = json!({
            CANONICAL: {
                "caller": { "type": "code_execution_20260120", "toolId": "pruned-source" },
            },
        });
        let result = convert(
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "search-call",
                            "toolName": "web_search",
                            "providerExecuted": true,
                            "input": { "query": "AI SDK" },
                            "providerOptions": caller,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "search-call",
                            "toolName": "web_search",
                            "output": { "type": "json", "value": [] },
                            "providerOptions": caller,
                        },
                    ],
                },
                { "role": "user", "content": [{ "type": "text", "text": "Tell me more." }] },
            ]),
            false,
        );

        assert_eq!(
            result.messages[0]["content"],
            json!([
                {
                    "type": "server_tool_use",
                    "id": "search-call",
                    "name": "web_search",
                    "input": { "query": "AI SDK" },
                },
                {
                    "type": "web_search_tool_result",
                    "tool_use_id": "search-call",
                    "content": [],
                },
            ])
        );
        assert_eq!(
            result.warnings,
            vec![other_warning(
                "Omitted caller metadata for tool search-call because source code execution tool pruned-source is missing from the conversation history."
            )]
        );
    }
}

mod system_messages {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should convert a single system message into an anthropic system message
    #[test]
    fn should_convert_a_single_system_message_into_an_anthropic_system_message() {
        let result = convert(
            json!([{ "role": "system", "content": "This is a system message" }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [],
                    "system": [{ "type": "text", "text": "This is a system message" }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should convert multiple system messages into an anthropic system message
    #[test]
    fn should_convert_multiple_system_messages_into_an_anthropic_system_message() {
        let result = convert(
            json!([
                { "role": "system", "content": "This is a system message" },
                { "role": "system", "content": "This is another system message" },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [],
                    "system": [
                        { "type": "text", "text": "This is a system message" },
                        { "type": "text", "text": "This is another system message" },
                    ],
                },
                "betas": set(&[]),
            })
        );
    }

    mod effort_only_system_messages {
        use super::*;

        fn low_effort() -> Value {
            json!({ "role": "system", "content": "", "providerOptions": { CANONICAL: { "effort": "low" } } })
        }

        fn high_effort() -> Value {
            json!({ "role": "system", "content": "", "providerOptions": { CANONICAL: { "effort": "high" } } })
        }

        fn instruction() -> Value {
            json!({ "role": "system", "content": "initial" })
        }

        fn user() -> Value {
            json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] })
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › effort-only system messages › should preserve initial effort messages $name
        #[test]
        fn should_preserve_initial_effort_messages() {
            let initial_text = json!([{ "type": "text", "text": "initial" }]);
            for (name, initial, text, efforts) in [
                ("alone", vec![low_effort()], json!([]), vec!["low"]),
                (
                    "after initial instructions",
                    vec![instruction(), low_effort()],
                    initial_text.clone(),
                    vec!["low"],
                ),
                (
                    "before initial instructions",
                    vec![low_effort(), instruction()],
                    initial_text.clone(),
                    vec!["low"],
                ),
                (
                    "consecutively",
                    vec![low_effort(), high_effort()],
                    json!([]),
                    vec!["low", "high"],
                ),
                (
                    "around initial instructions",
                    vec![low_effort(), instruction(), high_effort()],
                    initial_text.clone(),
                    vec!["low", "high"],
                ),
            ] {
                let mut prompt_json = initial;
                prompt_json.push(user());
                let result = convert(Value::Array(prompt_json), true);

                let mut expected_messages: Vec<Value> = efforts
                    .iter()
                    .map(|effort| {
                        json!({ "role": "system", "content": [], "output_config": { "effort": effort } })
                    })
                    .collect();
                expected_messages.push(user());
                assert_eq!(
                    super::result(&result),
                    json!({
                        "prompt": { "system": text, "messages": expected_messages },
                        "betas": set(&["mid-conversation-output-config-2026-07-01"]),
                    }),
                    "{name}"
                );
                assert_eq!(warnings(&result), json!([]), "{name}");
            }
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › effort-only system messages › should preserve later consecutive effort messages with initial instructions: %s
        #[test]
        fn should_preserve_later_consecutive_effort_messages_with_initial_instructions() {
            for has_initial in [false, true] {
                let mut prompt_json = Vec::new();
                if has_initial {
                    prompt_json.push(instruction());
                }
                prompt_json.extend([
                    user(),
                    low_effort(),
                    high_effort(),
                    json!({ "role": "system", "content": "later instructions" }),
                ]);
                let result = convert(Value::Array(prompt_json), true);

                assert_eq!(
                    system(&result),
                    has_initial.then(|| json!([{ "type": "text", "text": "initial" }])),
                    "{has_initial}"
                );
                assert_eq!(
                    messages(&result),
                    json!([
                        user(),
                        { "role": "system", "content": [], "output_config": { "effort": "low" } },
                        { "role": "system", "content": [], "output_config": { "effort": "high" } },
                        {
                            "role": "system",
                            "content": [{ "type": "text", "text": "later instructions" }],
                        },
                    ]),
                    "{has_initial}"
                );
                assert_eq!(
                    betas(&result),
                    set(&[
                        "mid-conversation-system-2026-04-07",
                        "mid-conversation-output-config-2026-07-01",
                    ]),
                    "{has_initial}"
                );
                assert_eq!(warnings(&result), json!([]), "{has_initial}");
            }
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › effort-only system messages › should warn and ignore unsupported initial system options: $options
        #[test]
        fn should_warn_and_ignore_unsupported_initial_system_options() {
            for (content, options) in [
                ("initial", json!({ "effort": "low" })),
                ("", json!({ "clearAt": "next_user_message" })),
                (
                    "",
                    json!({ "clearAt": "next_user_message", "effort": "low" }),
                ),
            ] {
                let result = convert(
                    json!([
                        {
                            "role": "system",
                            "content": content,
                            "providerOptions": { CANONICAL: options },
                        },
                        user(),
                    ]),
                    true,
                );

                assert_eq!(messages(&result), json!([user()]), "{options}");
                assert_eq!(betas(&result), set(&[]), "{options}");
                assert!(
                    matches!(
                        result.warnings.as_slice(),
                        [Warning::Other { message }] if message.contains("These options have been ignored.")
                    ),
                    "{options}: {:?}",
                    result.warnings
                );
            }
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should emit a mid-conversation system message inline and add the beta
    #[test]
    fn should_emit_a_mid_conversation_system_message_inline_and_add_the_beta() {
        let result = convert(
            json!([
                { "role": "system", "content": "initial" },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] },
                { "role": "system", "content": "switch tone" },
                { "role": "user", "content": [{ "type": "text", "text": "go" }] },
            ]),
            true,
        );

        assert_eq!(
            system(&result),
            Some(json!([{ "type": "text", "text": "initial" }]))
        );
        assert!(result.messages.contains(&json!({
            "role": "system",
            "content": [{ "type": "text", "text": "switch tone" }],
        })));
        assert!(has_beta(&result, "mid-conversation-system-2026-04-07"));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should serialize effort updates and turn-scoped reminders on separate system messages
    #[test]
    fn should_serialize_effort_updates_and_turn_scoped_reminders_on_separate_system_messages() {
        let result = convert(
            json!([
                { "role": "system", "content": "initial" },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] },
                {
                    "role": "system",
                    "content": "",
                    "providerOptions": { CANONICAL: { "effort": "high" } },
                },
                { "role": "user", "content": [{ "type": "text", "text": "go" }] },
                {
                    "role": "system",
                    "content": "this instruction applies to the current turn",
                    "providerOptions": { CANONICAL: { "clearAt": "next_user_message" } },
                },
                { "role": "system", "content": "this instruction persists" },
            ]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] },
                { "role": "system", "content": [], "output_config": { "effort": "high" } },
                { "role": "user", "content": [{ "type": "text", "text": "go" }] },
                {
                    "role": "system",
                    "content": [{
                        "type": "text",
                        "text": "this instruction applies to the current turn",
                    }],
                    "clear_at": "next_user_message",
                },
                {
                    "role": "system",
                    "content": [{ "type": "text", "text": "this instruction persists" }],
                },
            ])
        );
        assert!(has_beta(
            &result,
            "mid-conversation-system-clear-at-2026-08-21"
        ));
        assert!(has_beta(
            &result,
            "mid-conversation-output-config-2026-07-01"
        ));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should emit tool change blocks on a mid-conversation system message and add the beta
    #[test]
    fn should_emit_tool_change_blocks_on_a_mid_conversation_system_message() {
        let result = convert(
            json!([
                { "role": "system", "content": "initial" },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] },
                {
                    "role": "system",
                    "content": "tools have changed",
                    "providerOptions": {
                        CANONICAL: {
                            "toolChanges": [
                                { "type": "tool_addition", "toolName": "get_forecast" },
                                { "type": "tool_removal", "toolName": "get_weather" },
                            ],
                        },
                    },
                },
                { "role": "user", "content": [{ "type": "text", "text": "go" }] },
            ]),
            true,
        );

        assert!(result.messages.contains(&json!({
            "role": "system",
            "content": [
                { "type": "text", "text": "tools have changed" },
                {
                    "type": "tool_addition",
                    "tool": { "type": "tool_reference", "name": "get_forecast" },
                },
                {
                    "type": "tool_removal",
                    "tool": { "type": "tool_reference", "name": "get_weather" },
                },
            ],
        })));
        assert!(has_beta(&result, "mid-conversation-system-2026-04-07"));
        assert!(has_beta(
            &result,
            "mid-conversation-tool-changes-2026-07-01"
        ));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should not emit an empty text block for a system message that only carries tool changes
    #[test]
    fn should_not_emit_an_empty_text_block_for_a_system_message_with_only_tool_changes() {
        let result = convert(
            json!([
                { "role": "system", "content": "initial" },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "hello" }] },
                {
                    "role": "system",
                    "content": "",
                    "providerOptions": {
                        CANONICAL: {
                            "toolChanges": [{ "type": "tool_removal", "toolName": "get_weather" }],
                        },
                    },
                },
            ]),
            true,
        );

        assert!(result.messages.contains(&json!({
            "role": "system",
            "content": [{
                "type": "tool_removal",
                "tool": { "type": "tool_reference", "name": "get_weather" },
            }],
        })));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › system messages › should warn and drop tool changes on the initial system message
    #[test]
    fn should_warn_and_drop_tool_changes_on_the_initial_system_message() {
        let result = convert(
            json!([
                {
                    "role": "system",
                    "content": "initial",
                    "providerOptions": {
                        CANONICAL: {
                            "toolChanges": [{ "type": "tool_addition", "toolName": "get_forecast" }],
                        },
                    },
                },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
            ]),
            true,
        );

        assert_eq!(
            system(&result),
            Some(json!([{ "type": "text", "text": "initial" }]))
        );
        assert!(!has_beta(
            &result,
            "mid-conversation-tool-changes-2026-07-01"
        ));
        assert!(
            result.warnings.iter().any(|warning| matches!(
                warning,
                Warning::Other { message } if message.contains("initial system message")
            )),
            "{:?}",
            result.warnings
        );
    }
}

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should add image parts for UInt8Array images
    #[test]
    fn should_add_image_parts_for_uint8array_images() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "data", "data": "AAECAw==" },
                    "mediaType": "image/png",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "image",
                            "source": { "data": "AAECAw==", "media_type": "image/png", "type": "base64" },
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should add image parts for URL images
    #[test]
    fn should_add_image_parts_for_url_images() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/image.png" },
                    "mediaType": "image/*",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "image",
                            "source": { "type": "url", "url": "https://example.com/image.png" },
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    mod top_level_only_media_type_resolution {
        use super::*;

        /// `Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).toString('base64')`.
        const PNG_BASE64: &str = "iVBORw0KGgo=";
        /// `Buffer.from([0x25, 0x50, 0x44, 0x46, 0x2d]).toString('base64')`.
        const PDF_BASE64: &str = "JVBERi0=";

        fn file(data: Value, media_type: &str) -> AnthropicPromptConversion {
            convert(
                json!([{
                    "role": "user",
                    "content": [{ "type": "file", "data": data, "mediaType": media_type }],
                }]),
                true,
            )
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › detects image subtype from inline bytes for top-level "image"
        #[test]
        fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
            let result = file(json!({ "type": "data", "data": PNG_BASE64 }), "image");
            assert_eq!(
                result.messages[0]["content"][0],
                json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": PNG_BASE64 },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › normalizes image/* via detection
        #[test]
        fn normalizes_image_via_detection() {
            let result = file(json!({ "type": "data", "data": PNG_BASE64 }), "image/*");
            assert_eq!(
                result.messages[0]["content"][0]["source"]["media_type"],
                json!("image/png")
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › passes through URL for top-level-only image (Anthropic accepts URL source)
        #[test]
        fn passes_through_url_for_top_level_only_image() {
            let result = file(
                json!({ "type": "url", "url": "https://example.com/x.png" }),
                "image",
            );
            assert_eq!(
                result.messages[0]["content"][0],
                json!({
                    "type": "image",
                    "source": { "type": "url", "url": "https://example.com/x.png" },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › detects PDF subtype from inline bytes for top-level "application"
        #[test]
        fn detects_pdf_subtype_from_inline_bytes_for_top_level_application() {
            let result = file(json!({ "type": "data", "data": PDF_BASE64 }), "application");
            assert_eq!(
                result.messages[0]["content"][0]["source"]["media_type"],
                json!("application/pdf")
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › preserves full image/png pass-through
        #[test]
        fn preserves_full_image_png_pass_through() {
            let result = file(json!({ "type": "data", "data": PNG_BASE64 }), "image/png");
            assert_eq!(
                result.messages[0]["content"][0]["source"]["media_type"],
                json!("image/png")
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › top-level-only media type resolution › still routes text/plain inline text through document source
        #[test]
        fn still_routes_text_plain_inline_text_through_document_source() {
            let result = file(json!({ "type": "text", "text": "hello" }), "text/plain");
            assert_matches_object(
                &result.messages[0]["content"][0],
                &json!({
                    "type": "document",
                    "source": { "type": "text", "media_type": "text/plain", "data": "hello" },
                }),
            );
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should treat URL strings in image file data as URLs, not base64)
    #[test]
    fn should_treat_url_strings_in_image_file_data_as_urls_not_base64() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/image.png" },
                    "mediaType": "image/png",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "image",
                            "source": { "type": "url", "url": "https://example.com/image.png" },
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should treat URL strings in PDF file data as URLs, not base64)
    #[test]
    fn should_treat_url_strings_in_pdf_file_data_as_urls_not_base64() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/document.pdf" },
                    "mediaType": "application/pdf",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": { "type": "url", "url": "https://example.com/document.pdf" },
                        }],
                    }],
                },
                "betas": set(&["pdfs-2024-09-25"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should add PDF file parts for base64 PDFs
    #[test]
    fn should_add_pdf_file_parts_for_base64_pdfs() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "data", "data": "base64PDFdata" },
                    "mediaType": "application/pdf",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": {
                                "type": "base64",
                                "media_type": "application/pdf",
                                "data": "base64PDFdata",
                            },
                        }],
                    }],
                },
                "betas": set(&["pdfs-2024-09-25"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should add PDF file parts for URL PDFs
    #[test]
    fn should_add_pdf_file_parts_for_url_pdfs() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/document.pdf" },
                    "mediaType": "application/pdf",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": { "type": "url", "url": "https://example.com/document.pdf" },
                        }],
                    }],
                },
                "betas": set(&["pdfs-2024-09-25"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should add text file parts for text/plain documents
    #[test]
    fn should_add_text_file_parts_for_text_plain_documents() {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode("sample text content");
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "data", "data": data },
                    "mediaType": "text/plain",
                    "filename": "sample.txt",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": {
                                "type": "text",
                                "media_type": "text/plain",
                                "data": "sample text content",
                            },
                            "title": "sample.txt",
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should map inline text file parts to inline text document source
    #[test]
    fn should_map_inline_text_file_parts_to_inline_text_document_source() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "text", "text": "inline text content" },
                    "mediaType": "text/plain",
                    "filename": "inline.txt",
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": {
                                "type": "text",
                                "media_type": "text/plain",
                                "data": "inline text content",
                            },
                            "title": "inline.txt",
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should throw error for unsupported file types
    #[test]
    fn should_throw_error_for_unsupported_file_types() {
        let error = try_convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "data": { "type": "data", "data": "base64data" },
                    "mediaType": "video/mp4",
                }],
            }]),
            true,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("media type: video/mp4"),
            "{error}"
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should convert messages with image file parts using provider reference
    #[test]
    fn should_convert_messages_with_image_file_parts_using_provider_reference() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "image/png",
                    "data": { "type": "reference", "reference": { CANONICAL: "file-img-12345" } },
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "image",
                            "source": { "type": "file", "file_id": "file-img-12345" },
                        }],
                    }],
                },
                "betas": set(&["files-api-2025-04-14"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should convert messages with PDF file parts using provider reference
    #[test]
    fn should_convert_messages_with_pdf_file_parts_using_provider_reference() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "application/pdf",
                    "data": { "type": "reference", "reference": { CANONICAL: "file-pdf-12345" } },
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": { "type": "file", "file_id": "file-pdf-12345" },
                        }],
                    }],
                },
                "betas": set(&["files-api-2025-04-14"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should convert messages with text/plain file parts using provider reference
    #[test]
    fn should_convert_messages_with_text_plain_file_parts_using_provider_reference() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "text/plain",
                    "data": { "type": "reference", "reference": { CANONICAL: "file-txt-12345" } },
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "document",
                            "source": { "type": "file", "file_id": "file-txt-12345" },
                        }],
                    }],
                },
                "betas": set(&["files-api-2025-04-14"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should convert provider referenced file parts to container uploads when requested
    #[test]
    fn should_convert_provider_referenced_file_parts_to_container_uploads_when_requested() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Analyze this data." },
                    {
                        "type": "file",
                        "mediaType": "text/csv",
                        "data": { "type": "reference", "reference": { CANONICAL: "file-csv-12345" } },
                        "providerOptions": { CANONICAL: { "containerUpload": true } },
                    },
                ],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "Analyze this data." },
                            { "type": "container_upload", "file_id": "file-csv-12345" },
                        ],
                    }],
                },
                "betas": set(&["files-api-2025-04-14"]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › user messages › should throw when provider reference does not contain anthropic key
    #[test]
    fn should_throw_when_provider_reference_does_not_contain_anthropic_key() {
        let error = try_convert(
            json!([{
                "role": "user",
                "content": [{
                    "type": "file",
                    "mediaType": "application/pdf",
                    "data": { "type": "reference", "reference": { "openai": "file-xyz" } },
                }],
            }]),
            true,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains(
                "No provider reference found for provider 'anthropic'. Available providers: openai"
            ),
            "{error}"
        );
    }
}

mod tool_messages {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should convert a single tool result into an anthropic user message
    #[test]
    fn should_convert_a_single_tool_result_into_an_anthropic_user_message() {
        let result = convert(
            json!([{
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolName": "tool-1",
                    "toolCallId": "tool-call-1",
                    "output": { "type": "json", "value": { "test": "This is a tool message" } },
                }],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "tool-call-1",
                            "content": "{\"test\":\"This is a tool message\"}",
                        }],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should convert multiple tool results into an anthropic user message
    #[test]
    fn should_convert_multiple_tool_results_into_an_anthropic_user_message() {
        let result = convert(
            json!([{
                "role": "tool",
                "content": [
                    {
                        "type": "tool-result",
                        "toolName": "tool-1",
                        "toolCallId": "tool-call-1",
                        "output": { "type": "json", "value": { "test": "This is a tool message" } },
                    },
                    {
                        "type": "tool-result",
                        "toolName": "tool-2",
                        "toolCallId": "tool-call-2",
                        "output": { "type": "json", "value": { "something": "else" } },
                    },
                ],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [
                            {
                                "type": "tool_result",
                                "tool_use_id": "tool-call-1",
                                "content": "{\"test\":\"This is a tool message\"}",
                            },
                            {
                                "type": "tool_result",
                                "tool_use_id": "tool-call-2",
                                "content": "{\"something\":\"else\"}",
                            },
                        ],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should combine user and tool messages
    #[test]
    fn should_combine_user_and_tool_messages() {
        let result = convert(
            json!([
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolName": "tool-1",
                        "toolCallId": "tool-call-1",
                        "output": { "type": "json", "value": { "test": "This is a tool message" } },
                    }],
                },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "This is a user message" }],
                },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "user",
                        "content": [
                            {
                                "type": "tool_result",
                                "tool_use_id": "tool-call-1",
                                "content": "{\"test\":\"This is a tool message\"}",
                            },
                            { "type": "text", "text": "This is a user message" },
                        ],
                    }],
                },
                "betas": set(&[]),
            })
        );
    }

    /// One tool message holding one `content` tool result.
    fn content_result(tool_name: &str, tool_call_id: &str, value: Value) -> Value {
        json!([{
            "role": "tool",
            "content": [{
                "type": "tool-result",
                "toolName": tool_name,
                "toolCallId": tool_call_id,
                "output": { "type": "content", "value": value },
            }],
        }])
    }

    /// The expected result of [`content_result`].
    fn expected_content_result(tool_call_id: &str, content: Value, betas: &[&str]) -> Value {
        json!({
            "betas": set(betas),
            "prompt": {
                "messages": [{
                    "content": [{
                        "content": content,
                        "tool_use_id": tool_call_id,
                        "type": "tool_result",
                    }],
                    "role": "user",
                }],
            },
        })
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should handle tool result with content parts
    #[test]
    fn should_handle_tool_result_with_content_parts() {
        let result = convert(
            content_result(
                "image-generator",
                "image-gen-1",
                json!([
                    { "type": "text", "text": "Image generated successfully" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "AAECAw==" },
                        "mediaType": "image/png",
                    },
                ]),
            ),
            true,
        );

        assert_eq!(
            super::result(&result),
            expected_content_result(
                "image-gen-1",
                json!([
                    { "text": "Image generated successfully", "type": "text" },
                    {
                        "source": { "data": "AAECAw==", "media_type": "image/png", "type": "base64" },
                        "type": "image",
                    },
                ]),
                &[],
            )
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should handle tool result with PDF content
    #[test]
    fn should_handle_tool_result_with_pdf_content() {
        let result = convert(
            content_result(
                "pdf-generator",
                "pdf-gen-1",
                json!([
                    { "type": "text", "text": "PDF generated successfully" },
                    {
                        "type": "file",
                        "data": { "type": "data", "data": "JVBERi0xLjQKJeLjz9MKNCAwIG9iago=" },
                        "mediaType": "application/pdf",
                    },
                ]),
            ),
            true,
        );

        assert_eq!(
            super::result(&result),
            expected_content_result(
                "pdf-gen-1",
                json!([
                    { "text": "PDF generated successfully", "type": "text" },
                    {
                        "source": {
                            "data": "JVBERi0xLjQKJeLjz9MKNCAwIG9iago=",
                            "media_type": "application/pdf",
                            "type": "base64",
                        },
                        "type": "document",
                    },
                ]),
                &["pdfs-2024-09-25"],
            )
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should handle tool result with custom tool-reference content for custom tool search
    #[test]
    fn should_handle_tool_result_with_custom_tool_reference_content() {
        let result = convert(
            content_result(
                "searchTools",
                "search-1",
                json!([
                    {
                        "type": "custom",
                        "providerOptions": {
                            CANONICAL: { "type": "tool-reference", "toolName": "get_weather" },
                        },
                    },
                    {
                        "type": "custom",
                        "providerOptions": {
                            CANONICAL: { "type": "tool-reference", "toolName": "get_forecast" },
                        },
                    },
                ]),
            ),
            true,
        );

        assert_eq!(
            super::result(&result),
            expected_content_result(
                "search-1",
                json!([
                    { "tool_name": "get_weather", "type": "tool_reference" },
                    { "tool_name": "get_forecast", "type": "tool_reference" },
                ]),
                &[],
            )
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should handle tool result with url-based PDF content
    #[test]
    fn should_handle_tool_result_with_url_based_pdf_content() {
        let result = convert(
            content_result(
                "get-pdf",
                "get-pdf-1",
                json!([{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/document.pdf" },
                    "mediaType": "application/pdf",
                }]),
            ),
            true,
        );

        assert_eq!(
            super::result(&result),
            expected_content_result(
                "get-pdf-1",
                json!([{
                    "source": { "type": "url", "url": "https://example.com/document.pdf" },
                    "type": "document",
                }]),
                &[],
            )
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › tool messages › should handle tool result with url-based image content
    #[test]
    fn should_handle_tool_result_with_url_based_image_content() {
        let result = convert(
            content_result(
                "image-generator",
                "image-gen-1",
                json!([{
                    "type": "file",
                    "data": { "type": "url", "url": "https://example.com/image.png" },
                    "mediaType": "image/png",
                }]),
            ),
            true,
        );

        assert_eq!(
            super::result(&result),
            expected_content_result(
                "image-gen-1",
                json!([{
                    "source": { "type": "url", "url": "https://example.com/image.png" },
                    "type": "image",
                }]),
                &[],
            )
        );
    }
}

mod assistant_messages {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should preserve fallback boundaries between reasoning blocks
    #[test]
    fn should_preserve_fallback_boundaries_between_reasoning_blocks() {
        let result = convert(
            json!([{
                "role": "assistant",
                "content": [
                    {
                        "type": "reasoning",
                        "text": "Primary model thinking",
                        "providerOptions": { CANONICAL: { "signature": "primary-signature" } },
                    },
                    {
                        "type": "custom",
                        "kind": "anthropic.fallback",
                        "providerOptions": {
                            CANONICAL: {
                                "type": "fallback",
                                "from": { "model": "claude-opus-5-5" },
                                "to": { "model": "claude-opus-4-8" },
                            },
                        },
                    },
                    {
                        "type": "reasoning",
                        "text": "Fallback model thinking",
                        "providerOptions": { CANONICAL: { "signature": "fallback-signature" } },
                    },
                ],
            }]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([{
                "role": "assistant",
                "content": [
                    {
                        "type": "thinking",
                        "thinking": "Primary model thinking",
                        "signature": "primary-signature",
                    },
                    {
                        "type": "fallback",
                        "from": { "model": "claude-opus-5-5" },
                        "to": { "model": "claude-opus-4-8" },
                    },
                    {
                        "type": "thinking",
                        "thinking": "Fallback model thinking",
                        "signature": "fallback-signature",
                    },
                ],
            }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should warn and omit fallback boundaries with invalid metadata
    #[test]
    fn should_warn_and_omit_fallback_boundaries_with_invalid_metadata() {
        let result = convert(
            json!([{
                "role": "assistant",
                "content": [{
                    "type": "custom",
                    "kind": "anthropic.fallback",
                    "providerOptions": {
                        CANONICAL: { "type": "fallback", "from": { "model": "claude-opus-5-5" } },
                    },
                }],
            }]),
            true,
        );

        assert_eq!(messages(&result), json!([]));
        assert_eq!(
            result.warnings,
            vec![other_warning(
                "anthropic fallback metadata must include from.model and to.model"
            )]
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should omit empty compaction blocks
    #[test]
    fn should_omit_empty_compaction_blocks() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "text",
                            "text": "",
                            "providerOptions": { CANONICAL: { "type": "compaction" } },
                        },
                        { "type": "text", "text": "assistant content" },
                    ],
                },
            ]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "assistant content" }] },
            ])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should omit an assistant message that only contains an empty compaction block
    #[test]
    fn should_omit_an_assistant_message_that_only_contains_an_empty_compaction_block() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "first user message" }] },
                {
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "",
                        "providerOptions": { CANONICAL: { "type": "compaction" } },
                    }],
                },
                { "role": "user", "content": [{ "type": "text", "text": "second user message" }] },
            ]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "first user message" }] },
                { "role": "user", "content": [{ "type": "text", "text": "second user message" }] },
            ])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should preserve non-empty compaction blocks
    #[test]
    fn should_preserve_non_empty_compaction_blocks() {
        let result = convert(
            json!([{
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "Summary of the conversation",
                    "providerOptions": { CANONICAL: { "type": "compaction" } },
                }],
            }]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([{
                "role": "assistant",
                "content": [{ "type": "compaction", "content": "Summary of the conversation" }],
            }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should preserve citations on assistant text
    #[test]
    fn should_preserve_citations_on_assistant_text() {
        let result = convert(
            json!([
                {
                    "role": "assistant",
                    "content": [{
                        "type": "text",
                        "text": "The Federal Reserve held rates steady.",
                        "providerOptions": {
                            CANONICAL: {
                                "citations": [{
                                    "type": "web_search_result_location",
                                    "cited_text": "The Committee decided to maintain the rate.",
                                    "url": "https://example.com/fed-decision",
                                    "title": "Federal Reserve decision",
                                    "encrypted_index": "encrypted-index",
                                }],
                            },
                        },
                    }],
                },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "What happened before that?" }],
                },
            ]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([
                {
                    "content": [{
                        "citations": [{
                            "cited_text": "The Committee decided to maintain the rate.",
                            "encrypted_index": "encrypted-index",
                            "title": "Federal Reserve decision",
                            "type": "web_search_result_location",
                            "url": "https://example.com/fed-decision",
                        }],
                        "text": "The Federal Reserve held rates steady.",
                        "type": "text",
                    }],
                    "role": "assistant",
                },
                {
                    "content": [{ "text": "What happened before that?", "type": "text" }],
                    "role": "user",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should remove trailing whitespace from last assistant message when there is no further user message
    #[test]
    fn should_remove_trailing_whitespace_from_last_assistant_message() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "assistant content  " }] },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                        {
                            "role": "assistant",
                            "content": [{ "type": "text", "text": "assistant content" }],
                        },
                    ],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should remove trailing whitespace from last assistant message with multi-part content when there is no further user message
    #[test]
    fn should_remove_trailing_whitespace_from_last_assistant_message_with_multi_part_content() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "assistant " },
                        { "type": "text", "text": "content  " },
                    ],
                },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                        {
                            "role": "assistant",
                            "content": [
                                { "type": "text", "text": "assistant " },
                                { "type": "text", "text": "content" },
                            ],
                        },
                    ],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should keep trailing whitespace from assistant message when there is a further user message
    #[test]
    fn should_keep_trailing_whitespace_from_assistant_message_with_a_further_user_message() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "assistant content  " }] },
                { "role": "user", "content": [{ "type": "text", "text": "user content 2" }] },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "user content" }] },
                        {
                            "role": "assistant",
                            "content": [{ "type": "text", "text": "assistant content  " }],
                        },
                        { "role": "user", "content": [{ "type": "text", "text": "user content 2" }] },
                    ],
                },
                "betas": set(&[]),
            })
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should combine multiple sequential assistant messages into a single message
    #[test]
    fn should_combine_multiple_sequential_assistant_messages_into_a_single_message() {
        let result = convert(
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "Hi!" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "Hello" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "World" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "!" }] },
            ]),
            true,
        );

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [
                        { "role": "user", "content": [{ "type": "text", "text": "Hi!" }] },
                        {
                            "role": "assistant",
                            "content": [
                                { "type": "text", "text": "Hello" },
                                { "type": "text", "text": "World" },
                                { "type": "text", "text": "!" },
                            ],
                        },
                    ],
                },
                "betas": set(&[]),
            })
        );
    }

    fn strawberry(signature: Option<&str>) -> Value {
        let mut reasoning = json!({
            "type": "reasoning",
            "text": "I need to count the number of \"r\"s in the word \"strawberry\".",
        });
        if let Some(signature) = signature {
            reasoning["providerOptions"] = json!({ CANONICAL: { "signature": signature } });
        }
        json!([{
            "role": "assistant",
            "content": [
                reasoning,
                { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
            ],
        }])
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert assistant message reasoning parts with signature into thinking parts when sendReasoning is true
    #[test]
    fn should_convert_reasoning_parts_with_signature_into_thinking_parts_when_send_reasoning() {
        let result = convert(strawberry(Some("test-signature")), true);

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "assistant",
                        "content": [
                            {
                                "type": "thinking",
                                "thinking": "I need to count the number of \"r\"s in the word \"strawberry\".",
                                "signature": "test-signature",
                            },
                            { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
                        ],
                    }],
                },
                "betas": set(&[]),
            })
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should ignore reasoning parts without signature into thinking parts when sendReasoning is true
    #[test]
    fn should_ignore_reasoning_parts_without_signature_when_send_reasoning_is_true() {
        let result = convert(strawberry(None), true);

        assert_eq!(
            super::result(&result),
            json!({
                "betas": set(&[]),
                "prompt": {
                    "messages": [{
                        "content": [{
                            "text": "The word \"strawberry\" has 2 \"r\"s.",
                            "type": "text",
                        }],
                        "role": "assistant",
                    }],
                },
            })
        );
        assert_eq!(
            warnings(&result),
            json!([{ "message": "unsupported reasoning metadata", "type": "other" }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should omit assistant message reasoning parts with signature when sendReasoning is false
    #[test]
    fn should_omit_reasoning_parts_with_signature_when_send_reasoning_is_false() {
        let result = convert(strawberry(Some("test-signature")), false);

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." }],
                    }],
                },
                "betas": set(&[]),
            })
        );
        assert_eq!(
            result.warnings,
            vec![other_warning(
                "sending reasoning content is disabled for this model"
            )]
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should omit reasoning parts without signature when sendReasoning is false
    #[test]
    fn should_omit_reasoning_parts_without_signature_when_send_reasoning_is_false() {
        let result = convert(strawberry(None), false);

        assert_eq!(
            super::result(&result),
            json!({
                "prompt": {
                    "messages": [{
                        "role": "assistant",
                        "content": [{ "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." }],
                    }],
                },
                "betas": set(&[]),
            })
        );
        assert_eq!(
            result.warnings,
            vec![other_warning(
                "sending reasoning content is disabled for this model"
            )]
        );
    }

    /// One assistant message with a provider-executed call and its result.
    fn server_call_and_result(
        tool_call_id: &str,
        tool_name: &str,
        input: Value,
        output: Value,
    ) -> Value {
        json!([{
            "role": "assistant",
            "content": [
                {
                    "input": input,
                    "providerExecuted": true,
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                    "type": "tool-call",
                },
                {
                    "output": output,
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                    "type": "tool-result",
                },
            ],
        }])
    }

    /// The expected result of [`server_call_and_result`].
    fn expected_server_call_and_result(
        tool_call_id: &str,
        tool_name: &str,
        input: Value,
        result_type: &str,
        content: Value,
    ) -> Value {
        json!({
            "betas": set(&[]),
            "prompt": {
                "messages": [{
                    "content": [
                        {
                            "id": tool_call_id,
                            "input": input,
                            "name": tool_name,
                            "type": "server_tool_use",
                        },
                        {
                            "content": content,
                            "tool_use_id": tool_call_id,
                            "type": result_type,
                        },
                    ],
                    "role": "assistant",
                }],
            },
        })
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_search tool call and result parts
    #[test]
    fn should_convert_anthropic_web_search_tool_call_and_result_parts() {
        let input = json!({ "query": "San Francisco major news events June 22 2025" });
        let result = convert(
            server_call_and_result(
                "srvtoolu_011cNtbtzFARKPcAcp7w4nh9",
                "web_search",
                input.clone(),
                json!({
                    "type": "json",
                    "value": [{
                        "url": "https://patch.com/california/san-francisco/calendar",
                        "title": "San Francisco Calendar",
                        "pageAge": null,
                        "encryptedContent": "encrypted-content",
                        "type": "web_search_result",
                    }],
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_011cNtbtzFARKPcAcp7w4nh9",
                "web_search",
                input,
                "web_search_tool_result",
                json!([{
                    "encrypted_content": "encrypted-content",
                    "page_age": null,
                    "title": "San Francisco Calendar",
                    "type": "web_search_result",
                    "url": "https://patch.com/california/san-francisco/calendar",
                }]),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should move regular tool_use blocks after provider-executed web_search results
    #[test]
    fn should_move_regular_tool_use_blocks_after_provider_executed_web_search_results() {
        let result = convert(
            json!([
                {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "I will save a note and search the web." },
                        {
                            "type": "tool-call",
                            "toolCallId": "toolu_regular",
                            "toolName": "saveNote",
                            "input": { "note": "Searching for basketball news" },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "srvtoolu_web_search",
                            "toolName": "web_search",
                            "providerExecuted": true,
                            "input": { "query": "basketball news today" },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "srvtoolu_web_search",
                            "toolName": "web_search",
                            "output": {
                                "type": "json",
                                "value": [{
                                    "url": "https://www.nba.com/news",
                                    "title": "NBA News",
                                    "pageAge": "1 hour ago",
                                    "encryptedContent": "encrypted-content",
                                    "type": "web_search_result",
                                }],
                            },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "toolu_regular",
                        "toolName": "saveNote",
                        "output": { "type": "json", "value": { "success": true } },
                    }],
                },
            ]),
            false,
        );

        assert_eq!(
            messages(&result),
            json!([
                {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "I will save a note and search the web." },
                        {
                            "type": "server_tool_use",
                            "id": "srvtoolu_web_search",
                            "name": "web_search",
                            "input": { "query": "basketball news today" },
                        },
                        {
                            "type": "web_search_tool_result",
                            "tool_use_id": "srvtoolu_web_search",
                            "content": [{
                                "url": "https://www.nba.com/news",
                                "title": "NBA News",
                                "page_age": "1 hour ago",
                                "encrypted_content": "encrypted-content",
                                "type": "web_search_result",
                            }],
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_regular",
                            "name": "saveNote",
                            "input": { "note": "Searching for basketball news" },
                        },
                    ],
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_regular",
                        "content": "{\"success\":true}",
                    }],
                },
            ])
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should not move regular tool_use blocks across thinking blocks
    #[test]
    fn should_not_move_regular_tool_use_blocks_across_thinking_blocks() {
        let result = convert(
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "reasoning",
                            "text": "Think before the initial note.",
                            "providerOptions": { CANONICAL: { "signature": "test-signature-1" } },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "toolu_initial",
                            "toolName": "saveNote",
                            "input": { "note": "phase 1: initial plan" },
                        },
                        {
                            "type": "reasoning",
                            "text": "Think before the revised note.",
                            "providerOptions": { CANONICAL: { "signature": "test-signature-2" } },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "toolu_revised",
                            "toolName": "saveNote",
                            "input": { "note": "phase 2: revised plan" },
                        },
                    ],
                },
                {
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolCallId": "toolu_initial",
                            "toolName": "saveNote",
                            "output": { "type": "json", "value": { "success": true } },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "toolu_revised",
                            "toolName": "saveNote",
                            "output": { "type": "json", "value": { "success": true } },
                        },
                    ],
                },
            ]),
            true,
        );

        assert_eq!(
            messages(&result),
            json!([
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "thinking",
                            "thinking": "Think before the initial note.",
                            "signature": "test-signature-1",
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_initial",
                            "name": "saveNote",
                            "input": { "note": "phase 1: initial plan" },
                        },
                        {
                            "type": "thinking",
                            "thinking": "Think before the revised note.",
                            "signature": "test-signature-2",
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_revised",
                            "name": "saveNote",
                            "input": { "note": "phase 2: revised plan" },
                        },
                    ],
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_initial",
                            "content": "{\"success\":true}",
                        },
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_revised",
                            "content": "{\"success\":true}",
                        },
                    ],
                },
            ])
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_search tool call with error result (error-json string)
    #[test]
    fn should_convert_anthropic_web_search_tool_call_with_error_result_error_json_string() {
        let input = json!({ "query": "test query" });
        let result = convert(
            server_call_and_result(
                "srvtoolu_error1",
                "web_search",
                input.clone(),
                json!({
                    "type": "error-json",
                    "value": "{\"type\":\"web_search_tool_result_error\",\"errorCode\":\"invalid_tool_input\"}",
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_error1",
                "web_search",
                input,
                "web_search_tool_result",
                json!({ "error_code": "invalid_tool_input", "type": "web_search_tool_result_error" }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_search tool call with error result (error-json object)
    #[test]
    fn should_convert_anthropic_web_search_tool_call_with_error_result_error_json_object() {
        let input = json!({ "query": "test query" });
        let result = convert(
            server_call_and_result(
                "srvtoolu_error2",
                "web_search",
                input.clone(),
                json!({
                    "type": "error-json",
                    "value": { "type": "web_search_tool_result_error", "errorCode": "max_uses_exceeded" },
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_error2",
                "web_search",
                input,
                "web_search_tool_result",
                json!({ "error_code": "max_uses_exceeded", "type": "web_search_tool_result_error" }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_fetch tool call and result parts
    #[test]
    fn should_convert_anthropic_web_fetch_tool_call_and_result_parts() {
        let url = "https://raw.githubusercontent.com/vercel/ai/blob/main/examples/ai-functions/data/ai.pdf";
        let input = json!({ "url": url });
        let result = convert(
            server_call_and_result(
                "srvtoolu_011cNtbtzFARKPcAcp7w4nh9",
                "web_fetch",
                input.clone(),
                json!({
                    "type": "json",
                    "value": {
                        "type": "web_fetch_result",
                        "url": url,
                        "retrievedAt": "2025-01-01T00:00:00.000Z",
                        "content": {
                            "type": "document",
                            "title": "AI.pdf",
                            "citations": { "enabled": true },
                            "source": {
                                "type": "text",
                                "mediaType": "text/plain",
                                "data": "The PDF says about AI.",
                            },
                        },
                    },
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_011cNtbtzFARKPcAcp7w4nh9",
                "web_fetch",
                input,
                "web_fetch_tool_result",
                json!({
                    "content": {
                        "citations": { "enabled": true },
                        "source": {
                            "data": "The PDF says about AI.",
                            "media_type": "text/plain",
                            "type": "text",
                        },
                        "title": "AI.pdf",
                        "type": "document",
                    },
                    "retrieved_at": "2025-01-01T00:00:00.000Z",
                    "type": "web_fetch_result",
                    "url": url,
                }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_fetch tool call with error result
    #[test]
    fn should_convert_anthropic_web_fetch_tool_call_with_error_result() {
        let input = json!({ "url": "https://httpbin.org/status/500" });
        let result = convert(
            server_call_and_result(
                "srvtoolu_016yTvwN6L1sDdjdPUzPbZRV",
                "web_fetch",
                input.clone(),
                json!({
                    "type": "error-json",
                    "value": "{\"type\":\"web_fetch_tool_result_error\",\"errorCode\":\"url_not_accessible\"}",
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_016yTvwN6L1sDdjdPUzPbZRV",
                "web_fetch",
                input,
                "web_fetch_tool_result",
                json!({ "error_code": "url_not_accessible", "type": "web_fetch_tool_result_error" }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_fetch tool call with error result as object
    #[test]
    fn should_convert_anthropic_web_fetch_tool_call_with_error_result_as_object() {
        let input =
            json!({ "url": "https://www.fotball.no/fotballdata/turnering/hjem/?fiksId=193156" });
        let result = convert(
            server_call_and_result(
                "srvtoolu_01JteKo9VRHDKZ1rdMXywnwD",
                "web_fetch",
                input.clone(),
                json!({
                    "type": "error-json",
                    "value": { "type": "web_fetch_tool_result_error", "errorCode": "url_not_allowed" },
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_01JteKo9VRHDKZ1rdMXywnwD",
                "web_fetch",
                input,
                "web_fetch_tool_result",
                json!({ "error_code": "url_not_allowed", "type": "web_fetch_tool_result_error" }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic web_fetch tool call with error result as malformed string
    #[test]
    fn should_convert_anthropic_web_fetch_tool_call_with_error_result_as_malformed_string() {
        let result = convert(
            server_call_and_result(
                "srvtoolu_test123",
                "web_fetch",
                json!({ "url": "https://example.com" }),
                json!({ "type": "error-json", "value": "not valid json at all" }),
            ),
            false,
        );

        assert_eq!(
            result.messages[0]["content"][1],
            json!({
                "content": { "error_code": "unavailable", "type": "web_fetch_tool_result_error" },
                "tool_use_id": "srvtoolu_test123",
                "type": "web_fetch_tool_result",
            })
        );
        assert_eq!(warnings(&result), json!([]));
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › should convert anthropic tool_search_tool_regex tool call and result parts
    #[test]
    fn should_convert_anthropic_tool_search_tool_regex_tool_call_and_result_parts() {
        let input = json!({ "pattern": "weather|forecast", "limit": 10 });
        let result = convert(
            server_call_and_result(
                "srvtoolu_01SACvPAnp6ucMJsstB5qb3f",
                "tool_search_tool_regex",
                input.clone(),
                json!({
                    "type": "json",
                    "value": [{ "type": "tool_reference", "toolName": "get_weather" }],
                }),
            ),
            false,
        );

        assert_eq!(
            super::result(&result),
            expected_server_call_and_result(
                "srvtoolu_01SACvPAnp6ucMJsstB5qb3f",
                "tool_search_tool_regex",
                input,
                "tool_search_tool_result",
                json!({
                    "tool_references": [{ "tool_name": "get_weather", "type": "tool_reference" }],
                    "type": "tool_search_tool_search_result",
                }),
            )
        );
        assert_eq!(warnings(&result), json!([]));
    }

    mod advisor_20260301_multi_turn_round_trip {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › advisor 20260301 multi-turn round-trip › should convert advisor server_tool_use + advisor_result back to the API shape
        #[test]
        fn should_convert_advisor_server_tool_use_advisor_result_back_to_the_api_shape() {
            let text = "Use a channel-based coordination pattern. Close the input channel first, then wait on a WaitGroup.";
            let result = convert(
                server_call_and_result(
                    "srvtoolu_advisor_abc123",
                    "advisor",
                    json!({}),
                    json!({
                        "type": "json",
                        "value": { "type": "advisor_result", "text": text, "stopReason": "max_tokens" },
                    }),
                ),
                false,
            );

            assert_eq!(
                super::result(&result),
                expected_server_call_and_result(
                    "srvtoolu_advisor_abc123",
                    "advisor",
                    json!({}),
                    "advisor_tool_result",
                    json!({ "stop_reason": "max_tokens", "text": text, "type": "advisor_result" }),
                )
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › advisor 20260301 multi-turn round-trip › should round-trip advisor_redacted_result verbatim across turns
        #[test]
        fn should_round_trip_advisor_redacted_result_verbatim_across_turns() {
            let result = convert(
                server_call_and_result(
                    "srvtoolu_advisor_redacted",
                    "advisor",
                    json!({}),
                    json!({
                        "type": "json",
                        "value": {
                            "type": "advisor_redacted_result",
                            "encryptedContent": "opaque-encrypted-blob-xyz",
                            "stopReason": "end_turn",
                        },
                    }),
                ),
                false,
            );

            assert_eq!(
                result.messages[0]["content"][1],
                json!({
                    "content": {
                        "encrypted_content": "opaque-encrypted-blob-xyz",
                        "stop_reason": "end_turn",
                        "type": "advisor_redacted_result",
                    },
                    "tool_use_id": "srvtoolu_advisor_redacted",
                    "type": "advisor_tool_result",
                })
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › advisor 20260301 multi-turn round-trip › should convert advisor_tool_result_error back to the API shape
        #[test]
        fn should_convert_advisor_tool_result_error_back_to_the_api_shape() {
            let result = convert(
                server_call_and_result(
                    "srvtoolu_advisor_err",
                    "advisor",
                    json!({}),
                    json!({
                        "type": "json",
                        "value": { "type": "advisor_tool_result_error", "errorCode": "max_uses_exceeded" },
                    }),
                ),
                false,
            );

            assert_eq!(
                result.messages[0]["content"][1],
                json!({
                    "content": { "error_code": "max_uses_exceeded", "type": "advisor_tool_result_error" },
                    "tool_use_id": "srvtoolu_advisor_err",
                    "type": "advisor_tool_result",
                })
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › advisor 20260301 multi-turn round-trip › should preserve multiple advisor turns interleaved with text
        #[test]
        fn should_preserve_multiple_advisor_turns_interleaved_with_text() {
            let result = convert(
                json!([
                    {
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": "Build a concurrent worker pool in Go with graceful shutdown.",
                        }],
                    },
                    {
                        "role": "assistant",
                        "content": [
                            { "type": "text", "text": "Let me consult the advisor on this." },
                            {
                                "input": {},
                                "providerExecuted": true,
                                "toolCallId": "srvtoolu_advisor_1",
                                "toolName": "advisor",
                                "type": "tool-call",
                            },
                            {
                                "output": {
                                    "type": "json",
                                    "value": { "type": "advisor_result", "text": "Use channels and a WaitGroup." },
                                },
                                "toolCallId": "srvtoolu_advisor_1",
                                "toolName": "advisor",
                                "type": "tool-result",
                            },
                            { "type": "text", "text": "Here is the implementation." },
                        ],
                    },
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "Now add a max-in-flight limit of 10." }],
                    },
                ]),
                false,
            );

            assert_eq!(
                messages(&result),
                json!([
                    {
                        "content": [{
                            "text": "Build a concurrent worker pool in Go with graceful shutdown.",
                            "type": "text",
                        }],
                        "role": "user",
                    },
                    {
                        "content": [
                            { "text": "Let me consult the advisor on this.", "type": "text" },
                            {
                                "id": "srvtoolu_advisor_1",
                                "input": {},
                                "name": "advisor",
                                "type": "server_tool_use",
                            },
                            {
                                "content": { "text": "Use channels and a WaitGroup.", "type": "advisor_result" },
                                "tool_use_id": "srvtoolu_advisor_1",
                                "type": "advisor_tool_result",
                            },
                            { "text": "Here is the implementation.", "type": "text" },
                        ],
                        "role": "assistant",
                    },
                    {
                        "content": [{ "text": "Now add a max-in-flight limit of 10.", "type": "text" }],
                        "role": "user",
                    },
                ])
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › advisor 20260301 multi-turn round-trip › should warn (and not emit a result block) when output type is unsupported
        #[test]
        fn should_warn_and_not_emit_a_result_block_when_output_type_is_unsupported() {
            let result = convert(
                server_call_and_result(
                    "srvtoolu_advisor_bad",
                    "advisor",
                    json!({}),
                    json!({ "type": "text", "value": "should be json" }),
                ),
                false,
            );

            assert_eq!(result.messages[0]["content"].as_array().unwrap().len(), 1);
            assert_eq!(
                warnings(&result),
                json!([{
                    "message": "provider executed tool result output type text for tool advisor is not supported",
                    "type": "other",
                }])
            );
        }
    }

    mod code_execution_20250522 {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20250522 › should convert anthropic code_execution tool call and result parts
        #[test]
        fn should_convert_anthropic_code_execution_tool_call_and_result_parts() {
            let input = json!({ "code": "print(\"Hello, world!\")" });
            let result = convert(
                server_call_and_result(
                    "srvtoolu_01XyZ1234567890",
                    "code_execution",
                    input.clone(),
                    json!({
                        "type": "json",
                        "value": {
                            "type": "code_execution_result",
                            "stdout": "Hello, world!",
                            "stderr": "",
                            "return_code": 0,
                        },
                    }),
                ),
                false,
            );

            assert_eq!(
                super::result(&result),
                expected_server_call_and_result(
                    "srvtoolu_01XyZ1234567890",
                    "code_execution",
                    input,
                    "code_execution_tool_result",
                    json!({
                        "content": [],
                        "return_code": 0,
                        "stderr": "",
                        "stdout": "Hello, world!",
                        "type": "code_execution_result",
                    }),
                )
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20250522 › should pass back encrypted_code_execution_result for multi-turn (web_fetch_20260209/web_search_20260209)
        #[test]
        fn should_pass_back_encrypted_code_execution_result_for_multi_turn() {
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "srvtoolu_webfetch_01",
                            "toolName": "web_fetch",
                            "input": { "url": "https://example.com" },
                            "providerExecuted": true,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "srvtoolu_webfetch_01",
                            "toolName": "web_fetch",
                            "output": {
                                "type": "json",
                                "value": {
                                    "type": "web_fetch_result",
                                    "url": "https://example.com",
                                    "retrievedAt": "2026-01-01T00:00:00Z",
                                    "content": {
                                        "type": "document",
                                        "title": "Example",
                                        "source": { "type": "text", "mediaType": "text/plain", "data": "hello" },
                                    },
                                },
                            },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "srvtoolu_codeexec_01",
                            "toolName": "code_execution",
                            "input": { "code": "print(\"done\")" },
                            "providerExecuted": true,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "srvtoolu_codeexec_01",
                            "toolName": "code_execution",
                            "output": {
                                "type": "json",
                                "value": {
                                    "type": "encrypted_code_execution_result",
                                    "encrypted_stdout": "enc_abc123",
                                    "stderr": "",
                                    "return_code": 0,
                                    "content": [],
                                },
                            },
                        },
                    ],
                }]),
                false,
            );

            let assistant_message = &result.messages[0];
            assert_eq!(assistant_message["role"], "assistant");
            let code_exec_result = assistant_message["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["type"] == "code_execution_tool_result")
                .expect("code_execution_tool_result block");
            assert_eq!(
                code_exec_result["content"],
                json!({
                    "type": "encrypted_code_execution_result",
                    "encrypted_stdout": "enc_abc123",
                    "stderr": "",
                    "return_code": 0,
                    "content": [],
                })
            );
            assert_eq!(warnings(&result), json!([]));
        }
    }

    /// The text-editor + bash code execution exchange of the 20250825 and
    /// 20260120 "should convert anthropic code_execution tool call and
    /// result parts" cases.
    fn code_execution_exchange() -> (Value, Value) {
        let prompt_json = json!([{
            "role": "assistant",
            "content": [
                {
                    "type": "tool-call",
                    "toolCallId": "srvtoolu_01Hq9rR6fZwwDGHkTYRafn7k",
                    "toolName": "code_execution",
                    "input": {
                        "type": "text_editor_code_execution",
                        "command": "create",
                        "path": "/tmp/fibonacci.py",
                        "file_text": "def..",
                    },
                    "providerExecuted": true,
                },
                {
                    "type": "tool-result",
                    "toolCallId": "srvtoolu_01Hq9rR6fZwwDGHkTYRafn7k",
                    "toolName": "code_execution",
                    "output": {
                        "type": "json",
                        "value": { "type": "text_editor_code_execution_create_result", "is_file_update": false },
                    },
                },
                {
                    "type": "tool-call",
                    "toolCallId": "srvtoolu_0193G3ttnkiTfZASwHQSKc2V",
                    "toolName": "code_execution",
                    "input": { "type": "bash_code_execution", "command": "python /tmp/fibonacci.py" },
                    "providerExecuted": true,
                },
                {
                    "type": "tool-result",
                    "toolCallId": "srvtoolu_0193G3ttnkiTfZASwHQSKc2V",
                    "toolName": "code_execution",
                    "output": {
                        "type": "json",
                        "value": {
                            "type": "bash_code_execution_result",
                            "content": [],
                            "stdout": "The 10th Fibonacci number is: 34\n",
                            "stderr": "",
                            "return_code": 0,
                        },
                    },
                },
            ],
        }]);
        let expected = json!({
            "betas": set(&[]),
            "prompt": {
                "messages": [{
                    "content": [
                        {
                            "id": "srvtoolu_01Hq9rR6fZwwDGHkTYRafn7k",
                            "input": { "command": "create", "file_text": "def..", "path": "/tmp/fibonacci.py" },
                            "name": "text_editor_code_execution",
                            "type": "server_tool_use",
                        },
                        {
                            "content": { "is_file_update": false, "type": "text_editor_code_execution_create_result" },
                            "tool_use_id": "srvtoolu_01Hq9rR6fZwwDGHkTYRafn7k",
                            "type": "text_editor_code_execution_tool_result",
                        },
                        {
                            "id": "srvtoolu_0193G3ttnkiTfZASwHQSKc2V",
                            "input": { "command": "python /tmp/fibonacci.py" },
                            "name": "bash_code_execution",
                            "type": "server_tool_use",
                        },
                        {
                            "content": {
                                "content": [],
                                "return_code": 0,
                                "stderr": "",
                                "stdout": "The 10th Fibonacci number is: 34\n",
                                "type": "bash_code_execution_result",
                            },
                            "tool_use_id": "srvtoolu_0193G3ttnkiTfZASwHQSKc2V",
                            "type": "bash_code_execution_tool_result",
                        },
                    ],
                    "role": "assistant",
                }],
            },
        });
        (prompt_json, expected)
    }

    mod code_execution_20250825 {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20250825 › should convert anthropic code_execution tool call and result parts
        #[test]
        fn should_convert_anthropic_code_execution_tool_call_and_result_parts() {
            let (prompt_json, expected) = code_execution_exchange();
            let result = convert(prompt_json, false);

            assert_eq!(super::result(&result), expected);
            assert_eq!(warnings(&result), json!([]));
        }
    }

    mod code_execution_20260120 {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20260120 › should preserve caller metadata and response order for dynamic filtering
        #[test]
        fn should_preserve_caller_metadata_and_response_order_for_dynamic_filtering() {
            let caller = json!({
                CANONICAL: {
                    "caller": { "type": "code_execution_20260120", "toolId": "code-execution-call" },
                },
            });
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "code-execution-call",
                            "toolName": "code_execution",
                            "input": {
                                "type": "programmatic-tool-call",
                                "code": "await web_search({ query: \"AI SDK\" })",
                            },
                            "providerExecuted": true,
                            "providerOptions": { CANONICAL: { "caller": { "type": "direct" } } },
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "web-search-call-1",
                            "toolName": "web_search",
                            "input": { "query": "AI SDK" },
                            "providerExecuted": true,
                            "providerOptions": caller,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "web-search-call-1",
                            "toolName": "web_search",
                            "output": { "type": "json", "value": [] },
                            "providerOptions": caller,
                        },
                        {
                            "type": "tool-call",
                            "toolCallId": "web-search-call-2",
                            "toolName": "web_search",
                            "input": { "query": "Anthropic" },
                            "providerExecuted": true,
                            "providerOptions": caller,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "web-search-call-2",
                            "toolName": "web_search",
                            "output": { "type": "json", "value": [] },
                            "providerOptions": caller,
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "code-execution-call",
                            "toolName": "code_execution",
                            "output": {
                                "type": "json",
                                "value": {
                                    "type": "encrypted_code_execution_result",
                                    "encrypted_stdout": "encrypted-output",
                                    "stderr": "",
                                    "return_code": 0,
                                    "content": [],
                                },
                            },
                        },
                    ],
                }]),
                false,
            );

            let code_caller =
                json!({ "type": "code_execution_20260120", "tool_id": "code-execution-call" });
            assert_eq!(result.messages.len(), 1);
            assert_matches_object(
                &result.messages[0],
                &json!({
                    "role": "assistant",
                    "content": [
                        { "type": "server_tool_use", "id": "code-execution-call", "caller": { "type": "direct" } },
                        { "type": "server_tool_use", "id": "web-search-call-1", "caller": code_caller },
                        { "type": "web_search_tool_result", "tool_use_id": "web-search-call-1", "caller": code_caller },
                        { "type": "server_tool_use", "id": "web-search-call-2", "caller": code_caller },
                        { "type": "web_search_tool_result", "tool_use_id": "web-search-call-2", "caller": code_caller },
                        { "type": "code_execution_tool_result", "tool_use_id": "code-execution-call" },
                    ],
                }),
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20260120 › should preserve a deferred server result across a client tool continuation
        #[test]
        fn should_preserve_a_deferred_server_result_across_a_client_tool_continuation() {
            let result = convert(
                json!([
                    {
                        "role": "assistant",
                        "content": [
                            {
                                "type": "tool-call",
                                "toolCallId": "code-execution-call",
                                "toolName": "code_execution",
                                "input": {
                                    "type": "programmatic-tool-call",
                                    "code": "await fetch_url({ url: \"https://example.com\" })",
                                },
                                "providerExecuted": true,
                                "providerOptions": { CANONICAL: { "caller": { "type": "direct" } } },
                            },
                            {
                                "type": "tool-call",
                                "toolCallId": "client-tool-call",
                                "toolName": "fetch_url",
                                "input": { "url": "https://example.com" },
                                "providerOptions": {
                                    CANONICAL: {
                                        "caller": { "type": "code_execution_20260120", "toolId": "code-execution-call" },
                                    },
                                },
                            },
                        ],
                    },
                    {
                        "role": "tool",
                        "content": [{
                            "type": "tool-result",
                            "toolCallId": "client-tool-call",
                            "toolName": "fetch_url",
                            "output": { "type": "text", "value": "Example Domain" },
                        }],
                    },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-result",
                            "toolCallId": "code-execution-call",
                            "toolName": "code_execution",
                            "output": {
                                "type": "json",
                                "value": {
                                    "type": "encrypted_code_execution_result",
                                    "encrypted_stdout": "encrypted-output",
                                    "stderr": "",
                                    "return_code": 0,
                                    "content": [],
                                },
                            },
                        }],
                    },
                ]),
                false,
            );

            assert_eq!(result.messages.len(), 3);
            assert_matches_object(
                &messages(&result),
                &json!([
                    {
                        "role": "assistant",
                        "content": [
                            { "type": "server_tool_use", "id": "code-execution-call", "caller": { "type": "direct" } },
                            {
                                "type": "tool_use",
                                "id": "client-tool-call",
                                "caller": { "type": "code_execution_20260120", "tool_id": "code-execution-call" },
                            },
                        ],
                    },
                    {
                        "role": "user",
                        "content": [{ "type": "tool_result", "tool_use_id": "client-tool-call" }],
                    },
                    {
                        "role": "assistant",
                        "content": [{ "type": "code_execution_tool_result", "tool_use_id": "code-execution-call" }],
                    },
                ]),
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20260120 › should replay server_tool_use input without the internal discriminator
        #[test]
        fn should_replay_server_tool_use_input_without_the_internal_discriminator() {
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "srvtoolu_cache_replay",
                        "toolName": "code_execution",
                        "input": { "type": "bash_code_execution", "command": "echo cache-compatible" },
                        "providerExecuted": true,
                    }],
                }]),
                false,
            );

            assert_eq!(
                messages(&result),
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "server_tool_use",
                        "id": "srvtoolu_cache_replay",
                        "name": "bash_code_execution",
                        "input": { "command": "echo cache-compatible" },
                    }],
                }])
            );
            assert_eq!(warnings(&result), json!([]));
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › code_execution 20260120 › should convert anthropic code_execution tool call and result parts
        #[test]
        fn should_convert_anthropic_code_execution_tool_call_and_result_parts() {
            let (prompt_json, expected) = code_execution_exchange();
            let result = convert(prompt_json, false);

            assert_eq!(super::result(&result), expected);
            assert_eq!(warnings(&result), json!([]));
        }
    }

    mod mcp_tool_use {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › assistant messages › mcp tool use › should convert anthropic mcp tool use parts
        #[test]
        fn should_convert_anthropic_mcp_tool_use_parts() {
            let text = "The echo tool responded back with \"hello world\" - it simply echoed the message I sent to it!";
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool-call",
                            "toolCallId": "mcptoolu_01HXPYHs79HH36fBbKHysCrp",
                            "toolName": "echo",
                            "input": {},
                            "providerExecuted": true,
                            "providerOptions": { CANONICAL: { "type": "mcp-tool-use", "serverName": "echo" } },
                        },
                        {
                            "type": "tool-result",
                            "toolCallId": "mcptoolu_01HXPYHs79HH36fBbKHysCrp",
                            "toolName": "echo",
                            "output": {
                                "type": "json",
                                "value": [{ "type": "text", "text": "Tool echo: hello world" }],
                            },
                        },
                        { "type": "text", "text": text },
                    ],
                }]),
                false,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "betas": set(&[]),
                    "prompt": {
                        "messages": [{
                            "content": [
                                {
                                    "id": "mcptoolu_01HXPYHs79HH36fBbKHysCrp",
                                    "input": {},
                                    "name": "echo",
                                    "server_name": "echo",
                                    "type": "mcp_tool_use",
                                },
                                {
                                    "content": [{ "text": "Tool echo: hello world", "type": "text" }],
                                    "is_error": false,
                                    "tool_use_id": "mcptoolu_01HXPYHs79HH36fBbKHysCrp",
                                    "type": "mcp_tool_result",
                                },
                                { "text": text, "type": "text" },
                            ],
                            "role": "assistant",
                        }],
                    },
                })
            );
            assert_eq!(
                warnings(&result),
                json!([{
                    "message": "provider executed tool result for tool echo is not supported",
                    "type": "other",
                }])
            );
        }
    }
}

/// The cache-control validator's warnings. Upstream reads them with
/// `cacheControlValidator.getWarnings()`; aimux merges them into the
/// conversion's warnings, where they are the `unsupported` ones.
fn cache_control_warnings(conversion: &AnthropicPromptConversion) -> Value {
    let warnings: Vec<&Warning> = conversion
        .warnings
        .iter()
        .filter(|warning| matches!(warning, Warning::Unsupported { .. }))
        .collect();
    serde_json::to_value(warnings).expect("warnings serialize")
}

mod cache_control {
    use super::*;

    mod system_message {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › system message › should set cache_control on system message with message cache control
        #[test]
        fn should_set_cache_control_on_system_message_with_message_cache_control() {
            let result = convert(
                json!([{
                    "role": "system",
                    "content": "system message",
                    "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [],
                        "system": [{
                            "type": "text",
                            "text": "system message",
                            "cache_control": { "type": "ephemeral" },
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }
    }

    mod user_message {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › user message › should set cache_control on user message part with part cache control
        #[test]
        fn should_set_cache_control_on_user_message_part_with_part_cache_control() {
            let result = convert(
                json!([{
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "test",
                        "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [{
                                "type": "text",
                                "text": "test",
                                "cache_control": { "type": "ephemeral" },
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › user message › should set cache_control on last user message part with message cache control
        #[test]
        fn should_set_cache_control_on_last_user_message_part_with_message_cache_control() {
            let result = convert(
                json!([{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "part1" },
                        { "type": "text", "text": "part2" },
                    ],
                    "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [
                                { "type": "text", "text": "part1" },
                                {
                                    "type": "text",
                                    "text": "part2",
                                    "cache_control": { "type": "ephemeral" },
                                },
                            ],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }
    }

    mod assistant_message {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › assistant message › should set cache_control on assistant message text part with part cache control
        #[test]
        fn should_set_cache_control_on_assistant_message_text_part_with_part_cache_control() {
            let result = convert(
                json!([
                    { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "text",
                            "text": "test",
                            "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                        }],
                    },
                ]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [
                            { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                            {
                                "role": "assistant",
                                "content": [{
                                    "type": "text",
                                    "text": "test",
                                    "cache_control": { "type": "ephemeral" },
                                }],
                            },
                        ],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › assistant message › should set cache_control on assistant tool call part with part cache control
        #[test]
        fn should_set_cache_control_on_assistant_tool_call_part_with_part_cache_control() {
            let result = convert(
                json!([
                    { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolCallId": "test-id",
                            "toolName": "test-tool",
                            "input": { "some": "arg" },
                            "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                        }],
                    },
                ]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [
                            { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                            {
                                "role": "assistant",
                                "content": [{
                                    "type": "tool_use",
                                    "name": "test-tool",
                                    "id": "test-id",
                                    "input": { "some": "arg" },
                                    "cache_control": { "type": "ephemeral" },
                                }],
                            },
                        ],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › assistant message › should wrap non-object (invalid) tool call input in an object
        #[test]
        fn should_wrap_non_object_invalid_tool_call_input_in_an_object() {
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "call-1",
                        "toolName": "cityAttractions",
                        "input": "{ \"city\": \"San Francisco\", }",
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "assistant",
                            "content": [{
                                "type": "tool_use",
                                "name": "cityAttractions",
                                "id": "call-1",
                                "input": { "rawInvalidInput": "{ \"city\": \"San Francisco\", }" },
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › assistant message › should set cache_control on last assistant message part with message cache control
        #[test]
        fn should_set_cache_control_on_last_assistant_message_part_with_message_cache_control() {
            let result = convert(
                json!([
                    { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                    {
                        "role": "assistant",
                        "content": [
                            { "type": "text", "text": "part1" },
                            { "type": "text", "text": "part2" },
                        ],
                        "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                    },
                ]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [
                            { "role": "user", "content": [{ "type": "text", "text": "user-content" }] },
                            {
                                "role": "assistant",
                                "content": [
                                    { "type": "text", "text": "part1" },
                                    {
                                        "type": "text",
                                        "text": "part2",
                                        "cache_control": { "type": "ephemeral" },
                                    },
                                ],
                            },
                        ],
                    },
                    "betas": set(&[]),
                })
            );
        }
    }

    mod tool_message {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › tool message › should set cache_control on tool result message part with part cache control
        #[test]
        fn should_set_cache_control_on_tool_result_message_part_with_part_cache_control() {
            let result = convert(
                json!([{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolName": "test",
                        "toolCallId": "test",
                        "output": { "type": "json", "value": { "test": "test" } },
                        "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "content": "{\"test\":\"test\"}",
                                "tool_use_id": "test",
                                "cache_control": { "type": "ephemeral" },
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › tool message › should set cache_control on tool result with output cache control
        #[test]
        fn should_set_cache_control_on_tool_result_with_output_cache_control() {
            let result = convert(
                json!([{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolName": "test",
                        "toolCallId": "test",
                        "output": {
                            "type": "text",
                            "value": "test",
                            "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                        },
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "content": "test",
                                "tool_use_id": "test",
                                "cache_control": { "type": "ephemeral" },
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › tool message › should set cache_control on tool result with content output cache control
        #[test]
        fn should_set_cache_control_on_tool_result_with_content_output_cache_control() {
            let result = convert(
                json!([{
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolName": "test",
                        "toolCallId": "test",
                        "output": {
                            "type": "content",
                            "value": [{
                                "type": "text",
                                "text": "test",
                                "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                            }],
                        },
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "content": [{ "type": "text", "text": "test" }],
                                "tool_use_id": "test",
                                "cache_control": { "type": "ephemeral" },
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › tool message › should set cache_control on last tool result message part with message cache control
        #[test]
        fn should_set_cache_control_on_last_tool_result_message_part_with_message_cache_control() {
            let result = convert(
                json!([{
                    "role": "tool",
                    "content": [
                        {
                            "type": "tool-result",
                            "toolName": "test",
                            "toolCallId": "part1",
                            "output": { "type": "json", "value": { "test": "part1" } },
                        },
                        {
                            "type": "tool-result",
                            "toolName": "test",
                            "toolCallId": "part2",
                            "output": { "type": "json", "value": { "test": "part2" } },
                        },
                    ],
                    "providerOptions": { CANONICAL: { "cacheControl": { "type": "ephemeral" } } },
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "user",
                            "content": [
                                {
                                    "type": "tool_result",
                                    "tool_use_id": "part1",
                                    "content": "{\"test\":\"part1\"}",
                                },
                                {
                                    "type": "tool_result",
                                    "tool_use_id": "part2",
                                    "content": "{\"test\":\"part2\"}",
                                    "cache_control": { "type": "ephemeral" },
                                },
                            ],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
        }
    }

    mod cache_control_validation {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › cache control validation › should reject cache_control on thinking blocks
        #[test]
        fn should_reject_cache_control_on_thinking_blocks() {
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "reasoning",
                        "text": "thinking content",
                        "providerOptions": {
                            CANONICAL: { "signature": "test-sig", "cacheControl": { "type": "ephemeral" } },
                        },
                    }],
                }]),
                true,
            );

            assert_eq!(
                super::result(&result),
                json!({
                    "prompt": {
                        "messages": [{
                            "role": "assistant",
                            "content": [{
                                "type": "thinking",
                                "thinking": "thinking content",
                                "signature": "test-sig",
                            }],
                        }],
                    },
                    "betas": set(&[]),
                })
            );
            assert_eq!(
                cache_control_warnings(&result),
                json!([{
                    "details": "cache_control cannot be set on thinking block. It will be ignored.",
                    "feature": "cache_control on non-cacheable context",
                    "type": "unsupported",
                }])
            );
        }

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › cache control validation › should reject cache_control on redacted thinking blocks
        #[test]
        fn should_reject_cache_control_on_redacted_thinking_blocks() {
            let result = convert(
                json!([{
                    "role": "assistant",
                    "content": [{
                        "type": "reasoning",
                        "text": "redacted",
                        "providerOptions": {
                            CANONICAL: { "redactedData": "abc123", "cacheControl": { "type": "ephemeral" } },
                        },
                    }],
                }]),
                true,
            );

            assert!(
                result.messages[0]["content"][0]
                    .get("cache_control")
                    .is_none()
            );
            assert_eq!(
                cache_control_warnings(&result),
                json!([{
                    "details": "cache_control cannot be set on redacted thinking block. It will be ignored.",
                    "feature": "cache_control on non-cacheable context",
                    "type": "unsupported",
                }])
            );
        }
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › cache control › should limit cache breakpoints to 4
    #[test]
    fn should_limit_cache_breakpoints_to_4() {
        let cache = json!({ CANONICAL: { "cacheControl": { "type": "ephemeral" } } });
        let result = convert(
            json!([
                { "role": "system", "content": "system 1", "providerOptions": cache },
                { "role": "system", "content": "system 2", "providerOptions": cache },
                {
                    "role": "user",
                    "content": [{ "type": "text", "text": "user 1", "providerOptions": cache }],
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "assistant 1", "providerOptions": cache }],
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "user 2 (should be rejected)",
                        "providerOptions": cache,
                    }],
                },
            ]),
            true,
        );

        let ephemeral = json!({ "type": "ephemeral" });
        let system = result.system.as_ref().expect("system");
        assert_eq!(system[0]["cache_control"], ephemeral);
        assert_eq!(system[1]["cache_control"], ephemeral);
        assert_eq!(result.messages[0]["content"][0]["cache_control"], ephemeral);
        assert_eq!(result.messages[1]["content"][0]["cache_control"], ephemeral);
        assert!(
            result.messages[2]["content"][0]
                .get("cache_control")
                .is_none()
        );
        assert_eq!(
            cache_control_warnings(&result),
            json!([{
                "details": "Maximum 4 cache breakpoints exceeded (found 5). This breakpoint will be ignored.",
                "feature": "cacheControl breakpoint limit",
                "type": "unsupported",
            }])
        );
    }
}

mod citations {
    use super::*;

    fn pdf(data: &str, filename: Option<&str>, anthropic: Option<Value>) -> Value {
        let mut part = json!({
            "type": "file",
            "data": { "type": "data", "data": data },
            "mediaType": "application/pdf",
        });
        if let Some(filename) = filename {
            part["filename"] = json!(filename);
        }
        if let Some(anthropic) = anthropic {
            part["providerOptions"] = json!({ CANONICAL: anthropic });
        }
        part
    }

    fn user_result(content: Value) -> Value {
        json!({
            "betas": set(&["pdfs-2024-09-25"]),
            "prompt": { "messages": [{ "content": content, "role": "user" }] },
        })
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › citations › should not include citations by default
    #[test]
    fn should_not_include_citations_by_default() {
        let result = convert(
            json!([{ "role": "user", "content": [pdf("base64PDFdata", None, None)] }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            user_result(json!([{
                "source": { "data": "base64PDFdata", "media_type": "application/pdf", "type": "base64" },
                "type": "document",
            }]))
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › citations › should include citations when enabled on file part
    #[test]
    fn should_include_citations_when_enabled_on_file_part() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [pdf(
                    "base64PDFdata",
                    None,
                    Some(json!({ "citations": { "enabled": true } })),
                )],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            user_result(json!([{
                "citations": { "enabled": true },
                "source": { "data": "base64PDFdata", "media_type": "application/pdf", "type": "base64" },
                "type": "document",
            }]))
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › citations › should include custom title and context when provided
    #[test]
    fn should_include_custom_title_and_context_when_provided() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [pdf(
                    "base64PDFdata",
                    Some("original-name.pdf"),
                    Some(json!({
                        "title": "Custom Document Title",
                        "context": "This is metadata about the document",
                        "citations": { "enabled": true },
                    })),
                )],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            user_result(json!([{
                "citations": { "enabled": true },
                "context": "This is metadata about the document",
                "source": { "data": "base64PDFdata", "media_type": "application/pdf", "type": "base64" },
                "title": "Custom Document Title",
                "type": "document",
            }]))
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › citations › should handle multiple documents with consistent citation settings
    #[test]
    fn should_handle_multiple_documents_with_consistent_citation_settings() {
        let result = convert(
            json!([{
                "role": "user",
                "content": [
                    pdf(
                        "base64PDFdata1",
                        Some("doc1.pdf"),
                        Some(json!({ "citations": { "enabled": true }, "title": "Custom Title 1" })),
                    ),
                    pdf(
                        "base64PDFdata2",
                        Some("doc2.pdf"),
                        Some(json!({
                            "citations": { "enabled": true },
                            "title": "Custom Title 2",
                            "context": "Additional context for document 2",
                        })),
                    ),
                    { "type": "text", "text": "Analyze both documents" },
                ],
            }]),
            true,
        );

        assert_eq!(
            super::result(&result),
            user_result(json!([
                {
                    "citations": { "enabled": true },
                    "source": { "data": "base64PDFdata1", "media_type": "application/pdf", "type": "base64" },
                    "title": "Custom Title 1",
                    "type": "document",
                },
                {
                    "citations": { "enabled": true },
                    "context": "Additional context for document 2",
                    "source": { "data": "base64PDFdata2", "media_type": "application/pdf", "type": "base64" },
                    "title": "Custom Title 2",
                    "type": "document",
                },
                { "text": "Analyze both documents", "type": "text" },
            ]))
        );
    }

    mod message_sequences {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › citations › message sequences › should convert user-assistant-tool-assistant-user message sequence with multiple tool calls
        #[test]
        fn should_convert_user_assistant_tool_assistant_user_sequence_with_multiple_tool_calls() {
            let result = convert(
                json!([
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "weather for berlin, london and paris" }],
                    },
                    {
                        "role": "assistant",
                        "content": [
                            {
                                "type": "text",
                                "text": "I will use the weather tool to get the weather for berlin, london and paris",
                            },
                            {
                                "type": "tool-call",
                                "toolName": "weather",
                                "toolCallId": "weather-call-1",
                                "input": { "location": "berlin" },
                            },
                            {
                                "type": "tool-call",
                                "toolName": "weather",
                                "toolCallId": "weather-call-2",
                                "input": { "location": "london" },
                            },
                            {
                                "type": "tool-call",
                                "toolName": "weather",
                                "toolCallId": "weather-call-3",
                                "input": { "location": "paris" },
                            },
                        ],
                    },
                    {
                        "role": "tool",
                        "content": [
                            {
                                "type": "tool-result",
                                "toolName": "weather",
                                "toolCallId": "weather-call-1",
                                "output": { "type": "json", "value": { "weather": "sunny" } },
                            },
                            {
                                "type": "tool-result",
                                "toolName": "weather",
                                "toolCallId": "weather-call-2",
                                "output": { "type": "json", "value": { "weather": "cloudy" } },
                            },
                            {
                                "type": "tool-result",
                                "toolName": "weather",
                                "toolCallId": "weather-call-3",
                                "output": { "type": "json", "value": { "weather": "rainy" } },
                            },
                        ],
                    },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "text",
                            "text": "The weather for berlin is sunny, the weather for london is cloudy, and the weather for paris is rainy",
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{ "type": "text", "text": "and for new york?" }],
                    },
                ]),
                true,
            );

            assert_eq!(
                super::result(&result)["prompt"],
                json!({
                    "messages": [
                        {
                            "content": [{ "text": "weather for berlin, london and paris", "type": "text" }],
                            "role": "user",
                        },
                        {
                            "content": [
                                {
                                    "text": "I will use the weather tool to get the weather for berlin, london and paris",
                                    "type": "text",
                                },
                                {
                                    "id": "weather-call-1",
                                    "input": { "location": "berlin" },
                                    "name": "weather",
                                    "type": "tool_use",
                                },
                                {
                                    "id": "weather-call-2",
                                    "input": { "location": "london" },
                                    "name": "weather",
                                    "type": "tool_use",
                                },
                                {
                                    "id": "weather-call-3",
                                    "input": { "location": "paris" },
                                    "name": "weather",
                                    "type": "tool_use",
                                },
                            ],
                            "role": "assistant",
                        },
                        {
                            "content": [
                                {
                                    "content": "{\"weather\":\"sunny\"}",
                                    "tool_use_id": "weather-call-1",
                                    "type": "tool_result",
                                },
                                {
                                    "content": "{\"weather\":\"cloudy\"}",
                                    "tool_use_id": "weather-call-2",
                                    "type": "tool_result",
                                },
                                {
                                    "content": "{\"weather\":\"rainy\"}",
                                    "tool_use_id": "weather-call-3",
                                    "type": "tool_result",
                                },
                            ],
                            "role": "user",
                        },
                        {
                            "content": [{
                                "text": "The weather for berlin is sunny, the weather for london is cloudy, and the weather for paris is rainy",
                                "type": "text",
                            }],
                            "role": "assistant",
                        },
                        {
                            "content": [{ "text": "and for new york?", "type": "text" }],
                            "role": "user",
                        },
                    ],
                })
            );
        }
    }
}

mod toolsets {
    use super::*;

    /// Upstream's `toolsetToolNameMapping`.
    fn toolset_tool_name_mapping() -> ToolNameMapping {
        let tools: Vec<Tool> = serde_json::from_value(json!([{
            "type": "provider",
            "id": "anthropic.computer_toolset_20260801",
            "name": "computer",
            "args": {},
        }]))
        .expect("valid tools");
        aimux_provider_utils::create_tool_name_mapping(
            Some(&tools),
            &[("anthropic.computer_toolset_20260801", "computer")],
        )
    }

    /// `toolsetNames: { [name]: 'computer' }`.
    fn toolset_names(name: &str) -> HashMap<String, String> {
        HashMap::from([(name.to_owned(), "computer".to_owned())])
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › toolsets › should serialize toolset tool calls and results with toolset_name
    #[test]
    fn should_serialize_toolset_tool_calls_and_results_with_toolset_name() {
        let result = convert_with(
            json!([
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "toolu_click",
                        "toolName": "computer",
                        "input": { "action": "left_click", "coordinate": [640, 60] },
                    }],
                },
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "toolu_click",
                        "toolName": "computer",
                        "output": { "type": "text", "value": "OK" },
                    }],
                },
            ]),
            true,
            &toolset_tool_name_mapping(),
            &toolset_names("computer"),
        );

        assert_eq!(
            messages(&result),
            json!([
                {
                    "content": [{
                        "id": "toolu_click",
                        "input": { "action": "left_click", "coordinate": [640, 60] },
                        "name": "left_click",
                        "toolset_name": "computer",
                        "type": "tool_use",
                    }],
                    "role": "assistant",
                },
                {
                    "content": [{
                        "content": "OK",
                        "tool_use_id": "toolu_click",
                        "toolset_name": "computer",
                        "type": "tool_result",
                    }],
                    "role": "user",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › toolsets › should detect toolset tool calls through provider metadata when the tool is not passed
    #[test]
    fn should_detect_toolset_tool_calls_through_provider_metadata_when_not_passed() {
        let result = convert(
            json!([
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool-call",
                        "toolCallId": "toolu_screenshot",
                        "toolName": "desktop",
                        "input": { "action": "screenshot" },
                        "providerOptions": { CANONICAL: { "toolsetName": "computer" } },
                    }],
                },
                {
                    "role": "tool",
                    "content": [{
                        "type": "tool-result",
                        "toolCallId": "toolu_screenshot",
                        "toolName": "desktop",
                        "output": { "type": "text", "value": "OK" },
                        "providerOptions": { CANONICAL: { "toolsetName": "computer" } },
                    }],
                },
            ]),
            true,
        );

        assert_eq!(
            result.messages[0]["content"],
            json!([{
                "type": "tool_use",
                "id": "toolu_screenshot",
                "name": "screenshot",
                "toolset_name": "computer",
                "input": { "action": "screenshot" },
            }])
        );
        assert_eq!(
            result.messages[1]["content"],
            json!([{
                "type": "tool_result",
                "tool_use_id": "toolu_screenshot",
                "toolset_name": "computer",
                "content": "OK",
            }])
        );
    }

    /// upstream: @ai-sdk/anthropic src/convert-to-anthropic-prompt.test.ts › toolsets › should retain malformed toolset calls and their existing error results: %j
    #[test]
    fn should_retain_malformed_toolset_calls_and_their_existing_error_results() {
        for (input, tool_name) in [
            (json!({ "coordinate": [1, 2] }), "computer"),
            (json!({ "action": null, "coordinate": [1, 2] }), "computer"),
            (json!("invalid JSON"), "computer"),
            (json!({ "action": null }), "desktop"),
        ] {
            let result = convert_with(
                json!([
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool-call",
                            "toolCallId": "toolu_bad",
                            "toolName": tool_name,
                            "input": input,
                        }],
                    },
                    {
                        "role": "tool",
                        "content": [{
                            "type": "tool-result",
                            "toolCallId": "toolu_bad",
                            "toolName": tool_name,
                            "output": { "type": "error-text", "value": "Invalid input for tool computer" },
                        }],
                    },
                ]),
                true,
                &toolset_tool_name_mapping(),
                &toolset_names(tool_name),
            );

            let expected_input = if input.is_string() {
                json!({ "rawInvalidInput": input })
            } else {
                input.clone()
            };
            assert_eq!(
                messages(&result),
                json!([
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": "toolu_bad",
                            "name": "computer",
                            "toolset_name": "computer",
                            "input": expected_input,
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "toolu_bad",
                            "toolset_name": "computer",
                            "content": "Invalid input for tool computer",
                            "is_error": true,
                        }],
                    },
                ]),
                "{input}"
            );
            assert_eq!(
                result.warnings,
                vec![other_warning(&format!(
                    "toolset tool call for tool {tool_name} is missing the action"
                ))],
                "{input}"
            );
        }
    }
}

// ── Older ports (previously `tests/anthropic_remaining_test.rs`) ──
//
// Remaining Anthropic provider tests — ported from the TS SDK suite.
//
// Sources:
//
// - `anthropic-provider.test.ts` — provider configuration (baseURL, auth,
//   custom provider name, `supportedUrls`).
// - `anthropic-unknown-model-max-output-tokens.test.ts` — unknown-model
//   `max_tokens` defaulting + compatibility warning.
// - `sanitize-json-schema.test.ts` — JSON Schema sanitization.
// - `anthropic-language-model.test.ts` → `mid-conversation tool changes`
//   describe block.

mod older_remaining {

    use aimux_core::language_model_message::LanguageModelPrompt;
    use aimux_core::types::Warning;
    use serde_json::json;

    /// Upstream drops the caller of a historical tool call whose source code execution call was pruned, with a warning.
    #[test]
    fn omits_orphaned_historical_programmatic_caller_with_warning() {
        let prompt: LanguageModelPrompt = serde_json::from_value(json!([
            {
                "role": "assistant",
                "content": [{
                    "type": "tool-call",
                    "toolCallId": "lookup-call",
                    "toolName": "lookup",
                    "input": { "ticker": "AAPL" },
                    "providerOptions": { "anthropic": {
                        "caller": { "type": "code_execution_20260120", "toolId": "pruned-source" }
                    } },
                }],
            },
            {
                "role": "tool",
                "content": [{
                    "type": "tool-result",
                    "toolCallId": "lookup-call",
                    "toolName": "lookup",
                    "output": { "type": "text", "value": "185.42" },
                }],
            },
            { "role": "user", "content": [{ "type": "text", "text": "Now look up MSFT." }] },
        ]))
        .unwrap();
        let result =
            crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
                &prompt, false,
            )
            .unwrap();
        assert_eq!(
            result.messages[0]["content"],
            json!([{ "type": "tool_use", "id": "lookup-call", "name": "lookup", "input": { "ticker": "AAPL" } }])
        );
        assert_eq!(
        result.warnings,
        vec![Warning::Other {
            message: "Omitted caller metadata for tool lookup-call because source code execution tool pruned-source is missing from the conversation history.".into()
        }]
    );
    }

    /// Upstream names the unsupported output type when an advisor result cannot be sent back.
    #[test]
    fn advisor_result_with_text_output_warns_with_output_type() {
        let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
            "role": "assistant",
            "content": [
                {
                    "type": "tool-call",
                    "toolCallId": "srvtoolu_advisor",
                    "toolName": "advisor",
                    "input": {},
                    "providerExecuted": true,
                },
                {
                    "type": "tool-result",
                    "toolCallId": "srvtoolu_advisor",
                    "toolName": "advisor",
                    "output": { "type": "text", "value": "should be json" },
                },
            ],
        }]))
        .unwrap();
        let result =
            crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
                &prompt, false,
            )
            .unwrap();
        assert_eq!(result.messages[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(
            result.warnings,
            vec![Warning::Other {
            message:
                "provider executed tool result output type text for tool advisor is not supported"
                    .into()
        }]
        );
    }

    /// Upstream sends an MCP tool result back as `mcp_tool_result` and still warns that the provider-executed result is not supported.
    #[test]
    fn mcp_tool_result_round_trip_warns_like_upstream() {
        let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
        "role": "assistant",
        "content": [
            {
                "type": "tool-call",
                "toolCallId": "mcptoolu_1",
                "toolName": "echo",
                "input": {},
                "providerExecuted": true,
                "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "echo" } },
            },
            {
                "type": "tool-result",
                "toolCallId": "mcptoolu_1",
                "toolName": "echo",
                "output": { "type": "json", "value": [{ "type": "text", "text": "hello" }] },
            },
        ],
    }]))
    .unwrap();
        let result =
            crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
                &prompt, false,
            )
            .unwrap();
        assert_eq!(
            result.messages[0]["content"][1],
            json!({
                "type": "mcp_tool_result",
                "tool_use_id": "mcptoolu_1",
                "is_error": false,
                "content": [{ "type": "text", "text": "hello" }],
            })
        );
        assert_eq!(
            result.warnings,
            vec![Warning::Other {
                message: "provider executed tool result for tool echo is not supported".into()
            }]
        );
    }

    /// An MCP tool named like a provider tool (`web_search` here) gets no
    /// unsupported-result warning: upstream's MCP branch has no `break` and falls
    /// into `if (providerToolName === 'web_search')`, which pushes a
    /// `web_search_tool_result` after the `mcp_tool_result` and breaks before the
    /// warning (convert-to-anthropic-prompt.ts:1026-1046, 1284-1336, 1435-1438).
    #[test]
    fn mcp_tool_named_like_a_provider_tool_does_not_warn() {
        let prompt: LanguageModelPrompt = serde_json::from_value(json!([{
            "role": "assistant",
            "content": [
                {
                    "type": "tool-call",
                    "toolCallId": "mcptoolu_2",
                    "toolName": "web_search",
                    "input": {},
                    "providerExecuted": true,
                    "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "search" } },
                },
                {
                    "type": "tool-result",
                    "toolCallId": "mcptoolu_2",
                    "toolName": "web_search",
                    "output": { "type": "json", "value": [{
                        "url": "https://example.com", "title": "Example", "pageAge": null,
                        "encryptedContent": "abc", "type": "web_search_result",
                    }] },
                },
            ],
        }]))
        .unwrap();
        let result =
            crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
                &prompt, false,
            )
            .unwrap();
        assert_eq!(
            result.messages[0]["content"],
            json!([
                {
                    "type": "mcp_tool_use",
                    "id": "mcptoolu_2",
                    "name": "web_search",
                    "server_name": "search",
                    "input": {},
                },
                {
                    "type": "mcp_tool_result",
                    "tool_use_id": "mcptoolu_2",
                    "is_error": false,
                    "content": [{
                        "url": "https://example.com", "title": "Example", "pageAge": null,
                        "encryptedContent": "abc", "type": "web_search_result",
                    }],
                },
                {
                    "type": "web_search_tool_result",
                    "tool_use_id": "mcptoolu_2",
                    "content": [{
                        "url": "https://example.com", "title": "Example", "page_age": null,
                        "encrypted_content": "abc", "type": "web_search_result",
                    }],
                },
            ])
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    /// An `error-json` result of an MCP tool named like a tool search tool is sent
    /// as an `mcp_tool_result` and warned about: upstream's MCP branch falls into
    /// the tool search check, which takes only `json`
    /// (convert-to-anthropic-prompt.ts:1026-1046, 1338-1350). A `json` result adds
    /// the `tool_search_tool_result` with no warning.
    #[test]
    fn mcp_tool_named_like_tool_search_warns_for_error_json() {
        let prompt = |output: serde_json::Value| -> LanguageModelPrompt {
            serde_json::from_value(json!([{
                "role": "assistant",
                "content": [
                    {
                        "type": "tool-call",
                        "toolCallId": "mcptoolu_3",
                        "toolName": "tool_search_tool_regex",
                        "input": {},
                        "providerExecuted": true,
                        "providerOptions": { "anthropic": { "type": "mcp-tool-use", "serverName": "search" } },
                    },
                    {
                        "type": "tool-result",
                        "toolCallId": "mcptoolu_3",
                        "toolName": "tool_search_tool_regex",
                        "output": output,
                    },
                ],
            }]))
            .unwrap()
        };

        let result =
            crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
                &prompt(json!({ "type": "error-json", "value": { "errorCode": "unavailable" } })),
                false,
            )
            .unwrap();
        assert_eq!(
            result.messages[0]["content"].as_array().unwrap()[1..],
            [json!({
                "type": "mcp_tool_result",
                "tool_use_id": "mcptoolu_3",
                "is_error": true,
                "content": { "errorCode": "unavailable" },
            })]
        );
        assert_eq!(
            result.warnings,
            vec![Warning::Other {
                message: "provider executed tool result output type error-json for tool tool_search_tool_regex is not supported".into()
            }]
        );

        let result = crate::anthropic::convert_to_prompt::convert_prompt_to_anthropic_full_fallible(
            &prompt(json!({ "type": "json", "value": [{ "type": "tool-reference", "toolName": "get_weather" }] })),
            false,
        )
        .unwrap();
        assert_eq!(
            result.messages[0]["content"][2],
            json!({
                "type": "tool_search_tool_result",
                "tool_use_id": "mcptoolu_3",
                "content": {
                    "type": "tool_search_tool_search_result",
                    "tool_references": [{ "type": "tool_reference", "tool_name": "get_weather" }],
                },
            })
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }
}
