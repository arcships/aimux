//! Remaining Mistral tests translated from TS that are NOT already covered by
//! `mistral_model_test.rs`. Covers:
//!
//! - `mistral-prepare-tools.test.ts` — all 8 cases (prepare_tools unit tests)
//! - `convert-to-mistral-chat-messages.test.ts` — 12 cases (message conversion
//!   unit tests): system, user text/image/file, assistant text/tool-call,
//!   prefix continuation, tool results
//! - `convert-mistral-usage.test.ts` — 4 cases (usage conversion via doGenerate):
//!   alternate cached-token spellings, zero cached, missing tokens
//! - `mistral-chat-language-model.test.ts` — uncovered scenarios:
//!   parse_finish_reason unit tests (6), build_request_body unit tests (7):
//!   stream flag, provider tool silently dropped, tool with description/strict,
//!   json_schema with name+description

use base64::Engine;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::tool::{FunctionTool, ProviderTool};
use aimux_core::types::FinishReasonUnified;

use aimux_providers::mistral::convert::{
    build_request_body, convert_prompt_to_mistral_messages, parse_finish_reason, prepare_tools,
};
use aimux_providers::{MistralConfig, MistralProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn basic_function_tool() -> FunctionTool {
    FunctionTool::new(
        "testFunction",
        json!({ "type": "object", "properties": {} }),
    )
    .with_description("test description")
}

async fn mock_json_response(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn ok_mistral_body() -> Value {
    json!({
        "id": "test-id",
        "model": "mistral-small-latest",
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 },
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": { "role": "assistant", "content": "ok" }
        }]
    })
}

// ════════════════════════════════════════════════════════════════════════════
// mistral-prepare-tools.test.ts
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should return undefined tools when no tools are provided"
#[test]
fn prepare_tools_empty_returns_none() {
    let result = prepare_tools(&Some(vec![]), None);
    assert!(result.tools.is_none());
    assert!(result.tool_choice.is_none());
    assert!(result.tool_warnings.is_empty());
}

/// TS: "should process function tools correctly"
#[test]
fn prepare_tools_function_tool() {
    let result = prepare_tools(&Some(vec![basic_function_tool()]), None);
    assert_eq!(
        result.tools,
        Some(vec![json!({
            "type": "function",
            "function": {
                "name": "testFunction",
                "description": "test description",
                "parameters": { "type": "object", "properties": {} }
            }
        })])
    );
    assert!(result.tool_choice.is_none());
    assert!(result.tool_warnings.is_empty());
}

/// TS: "should include strict field when set"
#[test]
fn prepare_tools_function_tool_with_strict() {
    let tool = FunctionTool::new("strictTool", json!({ "type": "object", "properties": {} }))
        .with_strict(true);
    let result = prepare_tools(&Some(vec![tool]), None);
    let tools = result.tools.expect("tools should be Some");
    assert_eq!(tools[0]["function"]["strict"], json!(true));
}

/// TS: "should handle auto tool choice"
#[test]
fn prepare_tools_auto_choice() {
    let result = prepare_tools(&Some(vec![basic_function_tool()]), Some(&ToolChoice::Auto));
    assert_eq!(result.tool_choice, Some(json!("auto")));
    assert!(result.tools.is_some());
}

/// TS: "should handle none tool choice"
#[test]
fn prepare_tools_none_choice() {
    let result = prepare_tools(&Some(vec![basic_function_tool()]), Some(&ToolChoice::None));
    assert_eq!(result.tool_choice, Some(json!("none")));
    assert!(result.tools.is_some());
}

/// TS: "should handle required tool choice" — Mistral maps Required → "any".
#[test]
fn prepare_tools_required_choice() {
    let result = prepare_tools(
        &Some(vec![basic_function_tool()]),
        Some(&ToolChoice::Required),
    );
    assert_eq!(result.tool_choice, Some(json!("any")));
    assert!(result.tools.is_some());
}

/// TS: "should handle tool type tool choice by filtering tools" — uses "any".
#[test]
fn prepare_tools_tool_choice_filters() {
    let result = prepare_tools(
        &Some(vec![basic_function_tool()]),
        Some(&ToolChoice::Tool {
            tool_name: "testFunction".to_string(),
        }),
    );
    assert_eq!(result.tool_choice, Some(json!("any")));
    let tools = result.tools.expect("tools should be Some");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["function"]["name"], json!("testFunction"));
}

/// TS: "should return no tools when tool_choice references a non-existent tool"
#[test]
fn prepare_tools_tool_choice_filters_to_empty() {
    let result = prepare_tools(
        &Some(vec![basic_function_tool()]),
        Some(&ToolChoice::Tool {
            tool_name: "nonExistent".to_string(),
        }),
    );
    assert_eq!(result.tool_choice, Some(json!("any")));
    let tools = result.tools.expect("tools should be Some");
    assert!(tools.is_empty());
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-mistral-chat-messages.test.ts — text messages
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should convert a system message to a string content"
#[test]
fn convert_system_message() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::System,
        content: vec![ContentPart::text("You are a helpful assistant.")],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    assert_eq!(
        messages,
        vec![json!({ "role": "system", "content": "You are a helpful assistant." })]
    );
}

/// TS: "should convert a user message to an array content"
#[test]
fn convert_user_text_message() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    assert_eq!(
        messages,
        vec![json!({
            "role": "user",
            "content": [{ "type": "text", "text": "Hello" }]
        })]
    );
}

/// TS: "should convert an assistant message to a string content"
#[test]
fn convert_assistant_message() {
    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hi")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("Hello!")],
            ..Default::default()
        },
    ];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    // The assistant message is last → prefix: true.
    let last = messages.last().expect("last message");
    assert_eq!(last["role"], json!("assistant"));
    assert_eq!(last["content"], json!("Hello!"));
    assert_eq!(last["prefix"], json!(true));
}

/// TS: "should not set prefix on a non-last assistant message"
#[test]
fn convert_assistant_message_no_prefix_when_not_last() {
    let prompt: LanguageModelPrompt = vec![
        LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::text("I am thinking")],
            ..Default::default()
        },
        LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Continue")],
            ..Default::default()
        },
    ];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    let assistant_msg = &messages[0];
    assert_eq!(assistant_msg["role"], json!("assistant"));
    assert_eq!(assistant_msg["content"], json!("I am thinking"));
    assert!(
        assistant_msg.get("prefix").is_none(),
        "prefix should be absent on non-last assistant"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-mistral-chat-messages.test.ts — image / file processing
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should convert an image part to an image_url data URI"
#[test]
fn convert_user_image_message() {
    let data = vec![0, 1, 2, 3];
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![
            ContentPart::text("What is this?"),
            ContentPart::Image {
                image: data.clone(),
                media_type: "image/png".to_string(),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    let expected_url = format!("data:image/png;base64,{}", b64(&data));
    assert_eq!(
        messages,
        vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "What is this?" },
                { "type": "image_url", "image_url": expected_url }
            ]
        })]
    );
}

/// TS: "should convert an image file to an image_url data URI"
#[test]
fn convert_user_image_file_message() {
    let data = vec![0, 1, 2, 3];
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::File {
            data: data.clone(),
            media_type: "image/png".to_string(),
            filename: None,
            provider_options: None,
        }],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    let expected_url = format!("data:image/png;base64,{}", b64(&data));
    assert_eq!(
        messages,
        vec![json!({
            "role": "user",
            "content": [{ "type": "image_url", "image_url": expected_url }]
        })]
    );
}

/// TS: "should convert a non-image file to a document_url data URI"
#[test]
fn convert_user_document_file_message() {
    let data = b"Document content".to_vec();
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::File {
            data: data.clone(),
            media_type: "text/plain".to_string(),
            filename: Some("doc.txt".to_string()),
            provider_options: None,
        }],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    let expected_url = format!("data:text/plain;base64,{}", b64(&data));
    assert_eq!(
        messages,
        vec![json!({
            "role": "user",
            "content": [{ "type": "document_url", "document_url": expected_url }]
        })]
    );
}

/// TS: "should accept top-level 'image' media type and still route to image_url"
#[test]
fn convert_user_image_file_top_level_mediatype() {
    let data = vec![0x89, 0x50, 0x4e, 0x47];
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::File {
            data: data.clone(),
            media_type: "image".to_string(),
            filename: None,
            provider_options: None,
        }],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    let content = messages[0]["content"].as_array().expect("content array");
    assert_eq!(content[0]["type"], json!("image_url"));
    let url = content[0]["image_url"].as_str().unwrap();
    assert!(
        url.starts_with("data:image;base64,"),
        "url should start with data:image: {url}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-mistral-chat-messages.test.ts — tool messages
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should convert an assistant tool call message"
#[test]
fn convert_assistant_tool_call_message() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Assistant,
        content: vec![
            ContentPart::text("Calling a tool"),
            ContentPart::ToolCall {
                tool_call_id: "tool-call-1".to_string(),
                tool_name: "tool-1".to_string(),
                input: json!({ "test": "This is a tool message" }),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    // The assistant message is last → prefix: true.
    assert_eq!(
        messages,
        vec![json!({
            "role": "assistant",
            "content": "Calling a tool",
            "tool_calls": [{
                "id": "tool-call-1",
                "type": "function",
                "function": {
                    "name": "tool-1",
                    "arguments": "{\"test\":\"This is a tool message\"}"
                }
            }],
            "prefix": true
        })]
    );
}

/// TS: "should convert a tool result message"
#[test]
fn convert_tool_result_message() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Tool,
        content: vec![ContentPart::ToolResult {
            tool_call_id: "tool-call-1".to_string(),
            output: json!({ "test": "This is a tool message" }),
            provider_options: None,
        }],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    assert_eq!(
        messages,
        vec![json!({
            "role": "tool",
            "tool_call_id": "tool-call-1",
            "content": "{\"test\":\"This is a tool message\"}"
        })]
    );
}

/// TS: "should convert a tool result with a string output"
#[test]
fn convert_tool_result_with_string_output() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Tool,
        content: vec![ContentPart::ToolResult {
            tool_call_id: "tool-call-1".to_string(),
            output: json!("plain string result"),
            provider_options: None,
        }],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    assert_eq!(
        messages,
        vec![json!({
            "role": "tool",
            "tool_call_id": "tool-call-1",
            "content": "plain string result"
        })]
    );
}

/// TS: "should convert multiple tool results into separate tool messages"
#[test]
fn convert_multiple_tool_results() {
    let prompt: LanguageModelPrompt = vec![LanguageModelPromptMessage {
        role: Role::Tool,
        content: vec![
            ContentPart::ToolResult {
                tool_call_id: "tool-call-1".to_string(),
                output: json!({ "test": "result 1" }),
                provider_options: None,
            },
            ContentPart::ToolResult {
                tool_call_id: "tool-call-2".to_string(),
                output: json!({ "test": "result 2" }),
                provider_options: None,
            },
        ],
        ..Default::default()
    }];
    let messages = convert_prompt_to_mistral_messages(&prompt);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], json!("tool"));
    assert_eq!(messages[0]["tool_call_id"], json!("tool-call-1"));
    assert_eq!(messages[0]["content"], json!("{\"test\":\"result 1\"}"));
    assert_eq!(messages[1]["role"], json!("tool"));
    assert_eq!(messages[1]["tool_call_id"], json!("tool-call-2"));
    assert_eq!(messages[1]["content"], json!("{\"test\":\"result 2\"}"));
}

// ════════════════════════════════════════════════════════════════════════════
// convert-mistral-usage.test.ts — usage conversion via doGenerate
//
// `convert_usage` is private in model.rs, so these tests exercise it through
// `do_generate` and assert on `result.usage`.
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should extract usage with prompt_tokens_details.cached_tokens"
#[tokio::test]
async fn usage_with_prompt_tokens_details_cached() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 100,
                "total_tokens": 200,
                "completion_tokens": 100,
                "prompt_tokens_details": { "cached_tokens": 25 }
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(100));
    assert_eq!(result.usage.input_tokens.no_cache, Some(75));
    assert_eq!(result.usage.input_tokens.cache_read, Some(25));
    assert_eq!(result.usage.output_tokens.total, Some(100));
}

/// TS: "should extract usage with prompt_token_details.cached_tokens" — the
/// alternate (singular "token") spelling used by Mistral at different times.
#[tokio::test]
async fn usage_with_prompt_token_details_cached() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 100,
                "total_tokens": 200,
                "completion_tokens": 100,
                "prompt_token_details": { "cached_tokens": 40 }
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(100));
    assert_eq!(result.usage.input_tokens.no_cache, Some(60));
    assert_eq!(result.usage.input_tokens.cache_read, Some(40));
}

/// TS: "should not set cache_read when cached_tokens is 0" — the impl omits
/// `cache_read` when the value is 0 (uses `if cache_read > 0`).
#[tokio::test]
async fn usage_with_zero_cached_tokens() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": {
                "prompt_tokens": 50,
                "total_tokens": 100,
                "completion_tokens": 50,
                "num_cached_tokens": 0
            },
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(50));
    assert_eq!(result.usage.input_tokens.no_cache, Some(50));
    assert_eq!(result.usage.input_tokens.cache_read, None);
}

/// TS: "should default missing token counts to 0"
#[tokio::test]
async fn usage_with_missing_tokens_defaults_to_zero() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        json!({
            "id": "test-id",
            "model": "mistral-small-latest",
            "usage": {},
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" }
            }]
        }),
    )
    .await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    assert_eq!(result.usage.input_tokens.total, Some(0));
    assert_eq!(result.usage.input_tokens.no_cache, Some(0));
    assert_eq!(result.usage.input_tokens.cache_read, None);
    assert_eq!(result.usage.output_tokens.total, Some(0));
}

// ════════════════════════════════════════════════════════════════════════════
// mistral-chat-language-model.test.ts — parse_finish_reason unit tests
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_finish_reason_stop() {
    let fr = parse_finish_reason("stop");
    assert_eq!(fr.unified, FinishReasonUnified::Stop);
    assert_eq!(fr.raw.as_deref(), Some("stop"));
}

#[test]
fn parse_finish_reason_length() {
    let fr = parse_finish_reason("length");
    assert_eq!(fr.unified, FinishReasonUnified::Length);
    assert_eq!(fr.raw.as_deref(), Some("length"));
}

/// TS: "model_length" is a Mistral-specific finish reason also mapped to Length.
#[test]
fn parse_finish_reason_model_length() {
    let fr = parse_finish_reason("model_length");
    assert_eq!(fr.unified, FinishReasonUnified::Length);
    assert_eq!(fr.raw.as_deref(), Some("model_length"));
}

#[test]
fn parse_finish_reason_tool_calls() {
    let fr = parse_finish_reason("tool_calls");
    assert_eq!(fr.unified, FinishReasonUnified::ToolCalls);
}

#[test]
fn parse_finish_reason_content_filter() {
    let fr = parse_finish_reason("content_filter");
    assert_eq!(fr.unified, FinishReasonUnified::ContentFilter);
}

#[test]
fn parse_finish_reason_unknown() {
    let fr = parse_finish_reason("something_unexpected");
    assert_eq!(fr.unified, FinishReasonUnified::Other);
    assert_eq!(fr.raw.as_deref(), Some("something_unexpected"));
}

// ════════════════════════════════════════════════════════════════════════════
// build_request_body unit tests — stream flag, provider tools, tool fields,
// response format with name + description.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn build_request_body_stream_flag() {
    let options = default_options(test_prompt());
    let body = build_request_body("mistral-small-latest", &options, true);
    assert_eq!(body["stream"], json!(true));
}

#[test]
fn build_request_body_no_stream_flag() {
    let options = default_options(test_prompt());
    let body = build_request_body("mistral-small-latest", &options, false);
    assert!(body.get("stream").is_none());
}

/// TS: provider-defined tools are silently dropped (Mistral filters them out
/// before calling prepare_tools — no warning is emitted).
#[test]
fn build_request_body_provider_tool_dropped() {
    let provider_tool = Tool::Provider(ProviderTool {
        id: "mistral.web_search".to_string(),
        name: "web_search".to_string(),
        args: json!({}),
    });
    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![provider_tool]),
        ..default_options(Vec::new())
    };
    let body = build_request_body("mistral-small-latest", &options, false);
    assert!(
        body.get("tools").is_none(),
        "provider-only tools should produce no tools field"
    );
}

/// TS: a function tool with a description includes it in the request body.
#[test]
fn build_request_body_tool_with_description() {
    let tool = FunctionTool::new("weather", json!({ "type": "object", "properties": {} }))
        .with_description("Get the weather");
    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![Tool::Function(tool)]),
        ..default_options(Vec::new())
    };
    let body = build_request_body("mistral-small-latest", &options, false);
    assert_eq!(body["tools"][0]["function"]["name"], json!("weather"));
    assert_eq!(
        body["tools"][0]["function"]["description"],
        json!("Get the weather")
    );
}

/// TS: a function tool with strict=true includes it in the request body.
#[test]
fn build_request_body_tool_with_strict() {
    let tool = FunctionTool::new("weather", json!({ "type": "object", "properties": {} }))
        .with_strict(true);
    let options = CallOptions {
        prompt: test_prompt(),
        tools: Some(vec![Tool::Function(tool)]),
        ..default_options(Vec::new())
    };
    let body = build_request_body("mistral-small-latest", &options, false);
    assert_eq!(body["tools"][0]["function"]["strict"], json!(true));
}

/// TS: json_schema response format with name + description + strict=false.
#[test]
fn build_request_body_json_schema_with_name_and_description() {
    let schema = json!({ "type": "object", "properties": { "name": { "type": "string" } } });
    let options = CallOptions {
        prompt: test_prompt(),
        response_format: Some(ResponseFormat::Json {
            schema: Some(schema.clone()),
            name: Some("person".to_string()),
            description: Some("A person object".to_string()),
        }),
        ..default_options(Vec::new())
    };
    let body = build_request_body("mistral-small-latest", &options, false);
    assert_eq!(body["response_format"]["type"], json!("json_schema"));
    assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        json!("person")
    );
    assert_eq!(
        body["response_format"]["json_schema"]["description"],
        json!("A person object")
    );
    assert_eq!(
        body["response_format"]["json_schema"]["strict"],
        json!(false)
    );
}

/// TS: json_object response format when schema is None.
#[test]
fn build_request_body_json_object_when_no_schema() {
    let options = CallOptions {
        prompt: test_prompt(),
        response_format: Some(ResponseFormat::Json {
            schema: None,
            name: Some("ignored".to_string()),
            description: None,
        }),
        ..default_options(Vec::new())
    };
    let body = build_request_body("mistral-small-latest", &options, false);
    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate — request body via the model (verifies build_request_body wiring)
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should send correct request body" — the model wires build_request_body
/// into the HTTP request and exposes it via `result.request_body`.
#[tokio::test]
async fn should_expose_request_body_from_model() {
    let server = MockServer::start().await;
    mock_json_response(&server, ok_mistral_body()).await;

    let config = MistralConfig::new("test-api-key").with_base_url(server.uri());
    let provider = MistralProvider::new(config);
    let model = provider.model("mistral-small-latest");

    let result = model
        .do_generate(&default_options(test_prompt()))
        .await
        .expect("should succeed");

    let body = result.request_body.expect("request body");
    assert_eq!(body["model"], "mistral-small-latest");
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": [{ "type": "text", "text": "Hello" }] }])
    );
    // No optional fields set → absent.
    assert!(body.get("max_tokens").is_none());
    assert!(body.get("tools").is_none());
    assert!(body.get("stream").is_none());
}
