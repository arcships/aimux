//! User-facing messages by role and their conversion to the provider prompt,
//! checked against the AI SDK (`ai@7.0.127`
//! `prompt/convert-to-language-model-prompt.ts`, `@ai-sdk/provider-utils@5.0.53`
//! `types/*-model-message.ts`).

use aimux_core::AiMuxError;
use aimux_core::content::{
    AssistantContentPart, ToolContentPart, ToolResultOutput, ToolResultPart, UserContentPart,
};
use aimux_core::language_model_message::{
    self as v4, AssistantPart, LanguageModelMessage, ToolPart, UserPart,
    convert_to_language_model_prompt,
};
use aimux_core::message::{ModelMessage, UserContent};
use aimux_core::shared::provider_namespace;
use serde_json::{Value, json};

fn messages(value: Value) -> Vec<ModelMessage> {
    serde_json::from_value(value).expect("valid messages")
}

fn convert(value: Value) -> Vec<LanguageModelMessage> {
    convert_to_language_model_prompt(&messages(value), None, None).expect("converts")
}

// C32: every message role carries `providerOptions`, and the conversion
// copies it onto the provider message (`:211-216`, `:219-245`, `:252-258`,
// `:362`, `:405`).
#[test]
fn message_provider_options_reach_the_provider_prompt() {
    let options = json!({ "anthropic": { "cacheControl": { "type": "ephemeral" } } });
    let prompt = convert(json!([
        { "role": "system", "content": "be brief", "providerOptions": options },
        { "role": "user", "content": "hi", "providerOptions": options },
        { "role": "assistant", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": {} }
        ], "providerOptions": options },
        { "role": "tool", "content": [
            { "type": "tool-result", "toolCallId": "c1", "output": { "type": "text", "value": "ok" } }
        ], "providerOptions": options },
    ]));
    let expected = provider_namespace(
        "anthropic",
        json!({ "cacheControl": { "type": "ephemeral" } }),
    )
    .unwrap();
    assert_eq!(prompt.len(), 4);
    for message in &prompt {
        let provider_options = match message {
            LanguageModelMessage::System {
                provider_options, ..
            }
            | LanguageModelMessage::User {
                provider_options, ..
            }
            | LanguageModelMessage::Assistant {
                provider_options, ..
            }
            | LanguageModelMessage::Tool {
                provider_options, ..
            } => provider_options,
        };
        assert_eq!(provider_options.as_ref(), Some(&expected), "{message:?}");
    }
    // The option survives a JSON round trip of the message.
    let message =
        &messages(json!([{ "role": "user", "content": "hi", "providerOptions": options }]))[0];
    assert_eq!(
        serde_json::to_value(message).unwrap(),
        json!({ "role": "user", "content": "hi", "providerOptions": options })
    );
}

// C31: content parts are a union per role (`UserContent`,
// `user-model-message.ts:22`; `AssistantContent`,
// `assistant-model-message.ts:32`; `ToolContent`, `tool-model-message.ts:23`).
#[test]
fn content_parts_are_checked_by_role() {
    let rejected = [
        // A tool call is not user content.
        json!({ "role": "user", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": {} }
        ] }),
        // An image part is user content only.
        json!({ "role": "assistant", "content": [
            { "type": "image", "image": "AQ==", "mediaType": "image/png" }
        ] }),
        // A tool message holds tool results and approval responses only.
        json!({ "role": "tool", "content": [{ "type": "text", "text": "x" }] }),
        json!({ "role": "tool", "content": "x" }),
        // System content is a string.
        json!({ "role": "system", "content": [{ "type": "text", "text": "x" }] }),
    ];
    for message in rejected {
        assert!(
            serde_json::from_value::<ModelMessage>(message.clone()).is_err(),
            "accepted {message}"
        );
    }

    let user = &messages(json!([{ "role": "user", "content": [
        { "type": "text", "text": "look" },
        { "type": "image", "image": "AQI=", "mediaType": "image/png" },
    ] }]))[0];
    assert!(matches!(
        user,
        ModelMessage::User { content: UserContent::Parts(parts), .. }
            if matches!(parts[1], UserContentPart::Image { .. })
    ));
}

// Empty text parts: a user message drops every one (`:242-243`); an
// assistant message drops one only when it has no provider options
// (`:263-269`). String content is never filtered (`:220-225`, `:253-258`).
#[test]
fn empty_text_parts_are_dropped_as_upstream_drops_them() {
    let options = json!({ "openai": { "x": 1 } });
    let prompt = convert(json!([
        { "role": "user", "content": [
            { "type": "text", "text": "" },
            { "type": "text", "text": "", "providerOptions": options },
            { "type": "text", "text": "q" },
        ] },
        { "role": "assistant", "content": [
            { "type": "text", "text": "" },
            { "type": "text", "text": "", "providerOptions": options },
        ] },
        { "role": "user", "content": "" },
    ]));
    let LanguageModelMessage::User { content, .. } = &prompt[0] else {
        panic!()
    };
    assert_eq!(content.len(), 1);
    let LanguageModelMessage::Assistant { content, .. } = &prompt[1] else {
        panic!()
    };
    assert!(
        matches!(&content[..], [AssistantPart::Text(v4::TextPart { text, provider_options: Some(_) })] if text.is_empty())
    );
    let LanguageModelMessage::User { content, .. } = &prompt[2] else {
        panic!()
    };
    assert!(matches!(&content[..], [UserPart::Text(v4::TextPart { text, .. })] if text.is_empty()));
}

// C36: a tool approval request stays out of the provider prompt
// (`:270-281`); a tool approval response reaches the provider only for a
// provider-executed tool, without `providerExecuted` (`:374-378`,
// `:395-402`); a tool message left empty is dropped (`:177-183`).
#[test]
fn tool_approvals_reach_the_provider_only_for_provider_executed_tools() {
    let prompt = convert(json!([
        { "role": "user", "content": "search" },
        { "role": "assistant", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "web_search",
              "input": {}, "providerExecuted": true },
            { "type": "tool-approval-request", "approvalId": "a1", "toolCallId": "c1" },
            { "type": "tool-call", "toolCallId": "c2", "toolName": "delete", "input": {} },
            { "type": "tool-approval-request", "approvalId": "a2", "toolCallId": "c2" },
        ] },
        { "role": "tool", "content": [
            { "type": "tool-approval-response", "approvalId": "a1", "approved": true,
              "reason": "fine", "providerExecuted": true },
        ] },
        { "role": "tool", "content": [
            { "type": "tool-approval-response", "approvalId": "a2", "approved": false },
        ] },
    ]));
    assert_eq!(
        prompt.len(),
        3,
        "the tool message left empty is dropped: {prompt:#?}"
    );
    let LanguageModelMessage::Assistant { content, .. } = &prompt[1] else {
        panic!()
    };
    assert_eq!(content.len(), 2);
    assert!(
        content
            .iter()
            .all(|part| matches!(part, AssistantPart::ToolCall(_)))
    );
    let LanguageModelMessage::Tool { content, .. } = &prompt[2] else {
        panic!()
    };
    assert_eq!(
        content,
        &[ToolPart::ToolApprovalResponse(
            v4::ToolApprovalResponsePart {
                approval_id: "a1".into(),
                approved: true,
                reason: Some("fine".into()),
                provider_options: None,
            }
        )]
    );
}

// The upstream `ToolResultPart` carries a typed `output`
// (`content-part.ts:218-247`), which the conversion hands to the provider
// unchanged (`mapToolResultOutput`, `:610-626`).
#[test]
fn typed_tool_result_output_reaches_the_provider() {
    let prompt = convert(json!([
        { "role": "assistant", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "delete", "input": {} }
        ] },
        { "role": "tool", "content": [
            { "type": "tool-result", "toolCallId": "c1", "toolName": "delete",
              "output": { "type": "execution-denied", "reason": "no" } }
        ] },
    ]));
    let LanguageModelMessage::Tool { content, .. } = &prompt[1] else {
        panic!()
    };
    let [ToolPart::ToolResult(result)] = &content[..] else {
        panic!()
    };
    assert_eq!(
        result.output,
        v4::ToolResultOutput::ExecutionDenied {
            reason: Some("no".into()),
            provider_options: None
        }
    );
}

// A tool result is the upstream `ToolResultPart`
// (`@ai-sdk/provider-utils@5.0.53` `types/content-part.ts:218-242`): a typed
// `output` (`:247-300`), read and written as is. The earlier `result` form
// and a bare `output` value are not `ToolResultOutput`s and are rejected.
#[test]
fn tool_result_json_is_the_ai_sdks() {
    let text = |value: &str| ToolResultOutput::Text {
        value: value.into(),
        provider_options: None,
    };
    let written = |output: ToolResultOutput| {
        serde_json::to_value(ToolContentPart::tool_result("c1", "t", output)).unwrap()
    };
    let cases = [
        (
            text("ok"),
            json!({ "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                    "output": { "type": "text", "value": "ok" } }),
        ),
        (
            ToolResultOutput::ErrorText {
                value: "boom".into(),
                provider_options: None,
            },
            json!({ "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                    "output": { "type": "error-text", "value": "boom" } }),
        ),
        (
            ToolResultOutput::Json {
                value: Value::Null,
                provider_options: None,
            },
            json!({ "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                    "output": { "type": "json", "value": null } }),
        ),
        (
            ToolResultOutput::ExecutionDenied {
                reason: None,
                provider_options: None,
            },
            json!({ "type": "tool-result", "toolCallId": "c1", "toolName": "t",
                    "output": { "type": "execution-denied" } }),
        ),
    ];
    for (output, expected) in cases {
        assert_eq!(written(output.clone()), expected);
        let ToolContentPart::ToolResult(part) =
            serde_json::from_value::<ToolContentPart>(expected.clone()).unwrap()
        else {
            panic!("{expected}")
        };
        assert_eq!(part.output, output, "{expected}");
    }

    for invalid in [
        json!({ "type": "tool-result", "toolCallId": "c1" }),
        json!({ "type": "tool-result", "toolCallId": "c1", "result": "ok" }),
        json!({ "type": "tool-result", "toolCallId": "c1", "output": "bare string" }),
        json!({ "type": "tool-result", "toolCallId": "c1", "output": { "type": "text" } }),
    ] {
        assert!(
            serde_json::from_value::<ToolContentPart>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

// A provider-executed tool result replays in the assistant message.
#[test]
fn assistant_tool_result_converts_like_a_tool_message_result() {
    let call = match AssistantContentPart::tool_call("c1", "web_search", json!({})) {
        AssistantContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_options,
            ..
        } => AssistantContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_executed: Some(true),
            provider_options,
        },
        _ => unreachable!(),
    };
    let message = ModelMessage::assistant_parts(vec![
        call,
        AssistantContentPart::ToolResult(ToolResultPart {
            tool_call_id: "c1".into(),
            tool_name: None,
            output: ToolResultOutput::Json {
                value: json!([1]),
                provider_options: None,
            },
            provider_options: None,
        }),
    ]);
    let prompt = convert_to_language_model_prompt(&[message], None, None).unwrap();
    let LanguageModelMessage::Assistant { content, .. } = &prompt[0] else {
        panic!()
    };
    let AssistantPart::ToolResult(result) = &content[1] else {
        panic!()
    };
    assert_eq!(result.tool_name, "web_search");
}

// C38: consecutive tool messages become one tool message (`:104-130`). The
// earlier message's `providerOptions` merge into its last part with
// `mergeObjects`, the part's own options winning, and the combined message
// takes the later message's `providerOptions`.
#[test]
fn consecutive_tool_messages_are_combined() {
    let prompt = convert(json!([
        { "role": "assistant", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "t", "input": {} },
            { "type": "tool-call", "toolCallId": "c2", "toolName": "t", "input": {} },
        ] },
        { "role": "tool", "content": [
            { "type": "tool-result", "toolCallId": "c1", "output": { "type": "text", "value": "one" },
              "providerOptions": { "openai": { "b": 2, "nested": { "y": 2 } } } },
        ], "providerOptions": {
            "anthropic": { "cacheControl": { "type": "ephemeral" } },
            "openai": { "a": 1, "b": 1, "nested": { "x": 1 } },
        } },
        { "role": "tool", "content": [
            { "type": "tool-result", "toolCallId": "c2", "output": { "type": "text", "value": "two" } },
        ], "providerOptions": { "openai": { "c": 3 } } },
    ]));
    assert_eq!(prompt.len(), 2, "{prompt:#?}");
    let LanguageModelMessage::Tool {
        content,
        provider_options,
    } = &prompt[1]
    else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(provider_options).unwrap(),
        json!({ "openai": { "c": 3 } })
    );
    let [ToolPart::ToolResult(one), ToolPart::ToolResult(two)] = &content[..] else {
        panic!("{content:#?}")
    };
    assert_eq!(
        serde_json::to_string(&one.provider_options).unwrap(),
        serde_json::to_string(&json!({
            "anthropic": { "cacheControl": { "type": "ephemeral" } },
            "openai": { "a": 1, "b": 2, "nested": { "x": 1, "y": 2 } },
        }))
        .unwrap()
    );
    assert_eq!(two.provider_options, None);
}

// C38: a tool call the provider did not execute needs a tool result, or a
// tool approval response (approved or denied), before the next user or
// system message and by the end of the prompt (`:57-87`, `:132-175`).
#[test]
fn tool_calls_without_results_are_rejected() {
    let error = |value: Value| {
        convert_to_language_model_prompt(&messages(value), None, None)
            .expect_err("missing tool results")
    };
    let call =
        |id: &str| json!({ "type": "tool-call", "toolCallId": id, "toolName": "t", "input": {} });
    let result = |id: &str| json!({ "type": "tool-result", "toolCallId": id, "output": { "type": "text", "value": "ok" } });

    let before_user = error(json!([
        { "role": "assistant", "content": [call("c1"), call("c2")] },
        { "role": "tool", "content": [result("c2")] },
        { "role": "user", "content": "next" },
        { "role": "assistant", "content": [call("c3")] },
    ]));
    assert!(
        matches!(&before_user, AiMuxError::MissingToolResults { tool_call_ids } if tool_call_ids == &["c1"]),
        "{before_user:?}"
    );
    assert_eq!(
        before_user.to_string(),
        "Tool result is missing for tool call c1."
    );

    let at_end = error(json!([
        { "role": "user", "content": "go" },
        { "role": "assistant", "content": [call("c1"), call("c2")] },
    ]));
    assert_eq!(
        at_end.to_string(),
        "Tool results are missing for tool calls c1, c2."
    );

    // A provider-executed call needs no result; an approval response counts
    // whether it approves or denies.
    convert(json!([
        { "role": "user", "content": "go" },
        { "role": "assistant", "content": [
            { "type": "tool-call", "toolCallId": "c1", "toolName": "web_search",
              "input": {}, "providerExecuted": true },
            call("c2"),
            { "type": "tool-approval-request", "approvalId": "a2", "toolCallId": "c2" },
        ] },
        { "role": "tool", "content": [
            { "type": "tool-approval-response", "approvalId": "a2", "approved": false },
        ] },
        { "role": "user", "content": "next" },
    ]));
}
