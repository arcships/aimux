//! Assistant-role `ContentPart::ToolResult` → Anthropic provider-executed
//! result blocks.
//!
//! Anthropic only accepts a bare `tool_result` block inside a **user** message.
//! On an **assistant** message the converter has to emit the typed
//! provider-executed block (`web_search_tool_result`, `mcp_tool_result`, …)
//! instead, or the follow-up request is rejected with HTTP 400.
//!
//! `generate_text` does not route provider-executed results into
//! `response_messages` yet — that needs `provider_executed` on the variant to
//! express upstream's `!part.providerExecuted` predicate, and ships with the
//! type change. The guard is in place first so the path is protected before it
//! opens; these tests construct the assistant prompt directly rather than
//! going through a generate call.
//!
//! Mirrors the assistant `case 'tool-result'` branch of
//! `reference/vercel-ai/anthropic/src/convert-to-anthropic-prompt.ts` (:871-1285).

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::tool::{ProviderTool, Tool};
use aimux_providers::anthropic::AnthropicConfig;
use aimux_providers::anthropic::convert::build_request_body_with_warnings;
use aimux_providers::anthropic::model::AnthropicModel;

// ── helpers ─────────────────────────────────────────────────────────────────

fn provider_tool(id: &str, name: &str) -> Tool {
    Tool::Provider(ProviderTool {
        id: id.to_string(),
        name: name.to_string(),
        args: json!({}),
    })
}

fn tool_result(tool_call_id: &str, tool_name: &str, result: Value) -> ContentPart {
    ContentPart::ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: Some(tool_name.to_string()),
        result,
        is_error: None,
        preliminary: None,
        dynamic: None,
        provider_options: None,
    }
}

fn msg(role: Role, content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content,
        provider_options: None,
    }
}

/// Build the request body for `prompt` + `tools` and return
/// `(assistant content blocks, warning messages)`.
fn convert(prompt: LanguageModelPrompt, tools: Vec<Tool>) -> (Vec<Value>, Vec<String>) {
    let mut options = CallOptions::new(prompt);
    if !tools.is_empty() {
        options.tools = Some(tools);
    }
    let req = build_request_body_with_warnings("claude-sonnet-4-5", &options, false).unwrap();
    let messages = req.body["messages"].as_array().cloned().unwrap_or_default();
    let blocks = messages
        .iter()
        .filter(|m| m["role"] == "assistant")
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .collect();
    let warnings = req
        .warnings
        .iter()
        .map(|w| serde_json::to_string(w).unwrap())
        .collect();
    (blocks, warnings)
}

// ── provider-executed result blocks ─────────────────────────────────────────

#[test]
fn assistant_web_search_result_becomes_web_search_tool_result() {
    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "web_search",
                json!([{
                    "url": "https://example.com",
                    "title": "Example",
                    "pageAge": "3 days",
                    "encryptedContent": "enc-1",
                    "type": "web_search_result",
                }]),
            )],
        )],
        vec![provider_tool("anthropic.web_search_20250305", "web_search")],
    );

    assert_eq!(
        blocks,
        vec![json!({
            "type": "web_search_tool_result",
            "tool_use_id": "srvtoolu_1",
            "content": [{
                "url": "https://example.com",
                "title": "Example",
                "page_age": "3 days",
                "encrypted_content": "enc-1",
                "type": "web_search_result",
            }],
        })]
    );
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

/// A renamed provider tool must still resolve to the provider block type.
#[test]
fn renamed_web_search_tool_still_resolves_to_provider_block() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result("srvtoolu_1", "mySearch", json!([]))],
        )],
        vec![provider_tool("anthropic.web_search_20250305", "mySearch")],
    );

    assert_eq!(blocks[0]["type"], "web_search_tool_result");
}

#[test]
fn assistant_web_search_error_becomes_error_content() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "web_search",
                json!({
                    "type": "web_search_tool_result_error",
                    "errorCode": "max_uses_exceeded",
                }),
            )],
        )],
        vec![provider_tool("anthropic.web_search_20250305", "web_search")],
    );

    assert_eq!(
        blocks[0]["content"],
        json!({
            "type": "web_search_tool_result_error",
            "error_code": "max_uses_exceeded",
        })
    );
}

#[test]
fn assistant_mcp_result_becomes_mcp_tool_result() {
    let mcp_call = ContentPart::ToolCall {
        tool_call_id: "mcptoolu_1".to_string(),
        tool_name: "echo".to_string(),
        input: json!({ "text": "hi" }),
        thought_signature: None,
        provider_options: Some(json!({
            "anthropic": { "type": "mcp-tool-use", "serverName": "my-server" }
        })),
    };

    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![
                mcp_call,
                tool_result(
                    "mcptoolu_1",
                    "echo",
                    json!([{ "type": "text", "text": "hi" }]),
                ),
            ],
        )],
        vec![],
    );

    assert_eq!(blocks[0]["type"], "tool_use");
    assert_eq!(
        blocks[1],
        json!({
            "type": "mcp_tool_result",
            "tool_use_id": "mcptoolu_1",
            "is_error": false,
            "content": [{ "type": "text", "text": "hi" }],
        })
    );
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn assistant_code_execution_result_becomes_code_execution_tool_result() {
    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "code_execution",
                json!({
                    "type": "code_execution_result",
                    "stdout": "42\n",
                    "stderr": "",
                    "return_code": 0,
                    "content": [],
                }),
            )],
        )],
        vec![provider_tool(
            "anthropic.code_execution_20250522",
            "code_execution",
        )],
    );

    assert_eq!(
        blocks,
        vec![json!({
            "type": "code_execution_tool_result",
            "tool_use_id": "srvtoolu_1",
            "content": {
                "type": "code_execution_result",
                "stdout": "42\n",
                "stderr": "",
                "return_code": 0,
                "content": [],
            },
        })]
    );
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

/// The bash subtool reports through the `code_execution` tool name but needs
/// its own block type.
#[test]
fn assistant_bash_code_execution_result_uses_bash_block() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "code_execution",
                json!({
                    "type": "bash_code_execution_result",
                    "stdout": "ok",
                    "stderr": "",
                    "return_code": 0,
                    "content": [],
                }),
            )],
        )],
        vec![provider_tool(
            "anthropic.code_execution_20250825",
            "code_execution",
        )],
    );

    assert_eq!(blocks[0]["type"], "bash_code_execution_tool_result");
    assert_eq!(blocks[0]["content"]["type"], "bash_code_execution_result");
}

#[test]
fn assistant_advisor_result_becomes_advisor_tool_result() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "advisor",
                json!({
                    "type": "advisor_result",
                    "text": "consider X",
                    "stopReason": "end_turn",
                }),
            )],
        )],
        vec![provider_tool("anthropic.advisor_20260301", "advisor")],
    );

    assert_eq!(
        blocks,
        vec![json!({
            "type": "advisor_tool_result",
            "tool_use_id": "srvtoolu_1",
            "content": {
                "type": "advisor_result",
                "text": "consider X",
                "stop_reason": "end_turn",
            },
        })]
    );
}

#[test]
fn assistant_tool_search_result_becomes_tool_search_tool_result() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "tool_search_tool_regex",
                json!([{ "type": "tool_reference", "toolName": "get_weather" }]),
            )],
        )],
        vec![provider_tool(
            "anthropic.tool_search_regex_20251119",
            "tool_search_tool_regex",
        )],
    );

    assert_eq!(
        blocks,
        vec![json!({
            "type": "tool_search_tool_result",
            "tool_use_id": "srvtoolu_1",
            "content": {
                "type": "tool_search_tool_search_result",
                "tool_references": [{ "type": "tool_reference", "tool_name": "get_weather" }],
            },
        })]
    );
}

#[test]
fn assistant_web_fetch_result_becomes_web_fetch_tool_result() {
    let (blocks, _) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "web_fetch",
                json!({
                    "type": "web_fetch_result",
                    "url": "https://example.com",
                    "retrievedAt": "2025-01-01T00:00:00Z",
                    "content": {
                        "type": "document",
                        "title": "Example",
                        "citations": { "enabled": true },
                        "source": {
                            "type": "text",
                            "mediaType": "text/plain",
                            "data": "hello",
                        },
                    },
                }),
            )],
        )],
        vec![provider_tool("anthropic.web_fetch_20250910", "web_fetch")],
    );

    assert_eq!(
        blocks[0]["content"],
        json!({
            "type": "web_fetch_result",
            "url": "https://example.com",
            "retrieved_at": "2025-01-01T00:00:00Z",
            "content": {
                "type": "document",
                "title": "Example",
                "citations": { "enabled": true },
                "source": { "type": "text", "media_type": "text/plain", "data": "hello" },
            },
        })
    );
}

// ── the fix: unknown tools must not emit a bare `tool_result` ───────────────

/// The core of the fix. A `tool_result` block inside an assistant message is a
/// hard HTTP 400 from Anthropic, so an unrecognised provider-executed result
/// must be dropped with a warning rather than emitted.
#[test]
fn unrecognized_assistant_tool_result_is_dropped_with_warning() {
    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![
                ContentPart::text("done"),
                tool_result("call_1", "get_weather", json!({ "temp": 20 })),
            ],
        )],
        vec![],
    );

    assert_eq!(blocks, vec![json!({ "type": "text", "text": "done" })]);
    assert!(
        blocks.iter().all(|b| b["type"] != "tool_result"),
        "assistant messages must never carry a bare tool_result: {blocks:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w
                .contains("provider executed tool result for tool get_weather is not supported")),
        "expected an unsupported-tool warning, got {warnings:?}"
    );
}

/// A `ToolResult` with no `tool_name` cannot be resolved either.
#[test]
fn assistant_tool_result_without_tool_name_is_dropped_with_warning() {
    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![ContentPart::tool_result("call_1", json!({ "ok": true }))],
        )],
        vec![],
    );

    assert!(blocks.is_empty(), "expected no blocks, got {blocks:?}");
    assert!(
        warnings.iter().any(|w| w.contains("is not supported")),
        "expected a warning, got {warnings:?}"
    );
}

/// A `code_execution` result whose payload has no recognisable `type` is also
/// dropped (upstream pushes the "not a valid code execution result" warning).
#[test]
fn invalid_code_execution_payload_is_dropped_with_warning() {
    let (blocks, warnings) = convert(
        vec![msg(
            Role::Assistant,
            vec![tool_result(
                "srvtoolu_1",
                "code_execution",
                json!({ "stdout": "no type field" }),
            )],
        )],
        vec![provider_tool(
            "anthropic.code_execution_20250522",
            "code_execution",
        )],
    );

    assert!(blocks.is_empty(), "expected no blocks, got {blocks:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not a valid code execution result")),
        "expected a code-execution warning, got {warnings:?}"
    );
}

// ── regression: user/tool-role results keep the bare `tool_result` ──────────

#[test]
fn user_role_tool_result_still_emits_bare_tool_result() {
    let mut options = CallOptions::new(vec![msg(
        Role::User,
        vec![tool_result("call_1", "get_weather", json!("sunny"))],
    )]);
    options.tools = Some(vec![provider_tool(
        "anthropic.web_search_20250305",
        "web_search",
    )]);
    let req = build_request_body_with_warnings("claude-sonnet-4-5", &options, false).unwrap();

    assert_eq!(
        req.body["messages"],
        json!([{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "call_1",
                "content": "sunny",
            }],
        }])
    );
}

#[test]
fn tool_role_tool_result_still_emits_bare_tool_result() {
    let options = CallOptions::new(vec![msg(
        Role::Tool,
        vec![tool_result(
            "call_1",
            "web_search",
            json!([{ "url": "https://example.com" }]),
        )],
    )]);
    let req = build_request_body_with_warnings("claude-sonnet-4-5", &options, false).unwrap();

    assert_eq!(req.body["messages"][0]["role"], "user");
    assert_eq!(
        req.body["messages"][0]["content"][0]["type"], "tool_result",
        "tool-role results must keep the bare block: {}",
        req.body["messages"][0]
    );
}

// ── round-trip: response block → ToolResult → the same wire block ───────────

/// The response side (`parse_anthropic_content`) camel-cases the payload; the
/// prompt side has to snake-case it back. Feeding a parsed `ToolResult`
/// straight back into the converter must reproduce the original wire block.
#[tokio::test]
async fn web_search_result_round_trips_through_a_generate_call() {
    let wire_block = json!({
        "type": "web_search_tool_result",
        "tool_use_id": "srvtoolu_rt",
        "content": [{
            "url": "https://example.com",
            "title": "Example",
            "page_age": "3 days",
            "encrypted_content": "enc-1",
            "type": "web_search_result",
        }],
    });

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_rt",
            "type": "message",
            "role": "assistant",
            "content": [wire_block],
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        })))
        .mount(&server)
        .await;

    let mut options = CallOptions::new(vec![msg(Role::User, vec![ContentPart::text("Hello")])]);
    options.tools = Some(vec![provider_tool(
        "anthropic.web_search_20250305",
        "mySearch",
    )]);
    let model = AnthropicModel::new(
        "claude-sonnet-4-5".to_string(),
        AnthropicConfig::new("test-api-key").with_base_url(server.uri()),
    );
    let result = model.do_generate(&options).await.unwrap();

    // Replay the parsed tool result on an assistant message.
    let replayed: Vec<ContentPart> = result
        .content
        .iter()
        .filter_map(|c| match c {
            GenerateContent::ToolResult {
                tool_call_id,
                tool_name,
                result,
                ..
            } => Some(tool_result(tool_call_id, tool_name, result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(replayed.len(), 1, "expected one parsed tool result");

    let (blocks, warnings) = convert(
        vec![msg(Role::Assistant, replayed)],
        vec![provider_tool("anthropic.web_search_20250305", "mySearch")],
    );

    assert_eq!(blocks, vec![wire_block]);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}
