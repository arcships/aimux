//! The shared helpers of the older ports (previously `tests/bedrock_remaining_test.rs`),
//! used by their parts across `src/bedrock/tests/`.

use crate::bedrock::api_types::BedrockUsage;
use crate::bedrock::chat_language_model::build_request_body_checked;
use crate::bedrock::{AmazonBedrockProviderSettings, BedrockModel, create_amazon_bedrock};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::language_model_message::{
    AssistantPart, TextPart, ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_provider_utils::Resolvable;
use futures::StreamExt;
use serde_json::{Value, json};

pub(super) fn test_tool() -> aimux_core::tool::FunctionTool {
    aimux_core::tool::FunctionTool {
        name: "getWeather".into(),
        description: None,
        input_schema: json!({"type":"object"}),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

pub(super) fn model_normalizes_tool_id(id: &str) -> bool {
    let mut opts = default_options(vec![assistant_msg(vec![tool_call(
        "tooluse_bpe71yCfRu2b5i-nKGDr5g".into(),
        "getWeather".into(),
        json!({}),
    )])]);
    opts.tools = Some(vec![test_tool().into()]);
    let (body, _, _, _) = build_request_body_checked(id, &opts, None).unwrap();
    body["messages"][0]["content"][0]["toolUse"]["toolUseId"] == "8eHypBDcw"
}

pub(super) fn text(value: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: value.into(),
        provider_options: None,
    })
}

pub(super) fn tool_call(id: String, name: String, input: Value) -> AssistantPart {
    AssistantPart::ToolCall(ToolCallPart {
        tool_call_id: id,
        tool_name: name,
        input,
        provider_executed: None,
        provider_options: None,
    })
}

pub(super) fn tool_result(id: String, output: Value) -> ToolPart {
    ToolPart::ToolResult(ToolResultPart {
        tool_call_id: id,
        tool_name: "weather".into(),
        output: serde_json::from_value::<ToolResultOutput>(output).unwrap(),
        provider_options: None,
    })
}

// ── Shared helpers ───────────────────────────────────────────────────────────

pub(super) fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::User {
        content: vec![text("Hello")],
        provider_options: None,
    }]
}

pub(super) fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

pub(super) fn make_model(server: &std::sync::Arc<MockFetch>) -> BedrockModel {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        fetch: Some(server.transport()),
        base_url: Some("https://bedrock.test".into()),
        api_key: Some(Resolvable::Value("test-token".into())),
        ..Default::default()
    })
    .unwrap()
    .chat("anthropic.claude-3-5-sonnet-20240620-v1:0")
}

pub(super) fn mock_converse_json(status: u16, body: Value) -> std::sync::Arc<MockFetch> {
    let mut canned = Canned::json(&body);
    canned.status = status;
    MockFetch::new(vec![canned])
}

pub(super) fn ok_converse_body() -> Value {
    json!({
        "output": {
            "message": {
                "role": "assistant",
                "content": [{ "text": "ok" }]
            }
        },
        "stopReason": "end_turn",
        "usage": { "inputTokens": 4, "outputTokens": 7, "totalTokens": 11 }
    })
}

pub(super) fn as_text(item: &GenerateContent) -> &str {
    match item {
        GenerateContent::Text { text, .. } => text,
        _ => panic!("expected Text content, got {item:?}"),
    }
}

pub(super) fn as_tool_call(item: &GenerateContent) -> (&str, &str, &str) {
    match item {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id,
            tool_name,
            input,
            ..
        }) => (tool_call_id, tool_name, input),
        _ => panic!("expected ToolCall content, got {item:?}"),
    }
}

pub(super) async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {e:?}"),
        }
    }
    parts
}

/// Clear the authentication environment between serial tests.
pub(super) fn clear_bedrock_env() {
    unsafe {
        std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK");
        std::env::remove_var("AWS_ACCESS_KEY_ID");
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("AWS_REGION");
        std::env::remove_var("AWS_SESSION_TOKEN");
    }
}

pub(super) fn bedrock_usage_with_total(
    input: u32,
    output: u32,
    total: Option<u32>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> BedrockUsage {
    BedrockUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: total,
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        ..Default::default()
    }
}

pub(super) fn tool_msg(content: Vec<ToolPart>) -> LanguageModelMessage {
    LanguageModelMessage::Tool {
        content,
        provider_options: None,
    }
}

pub(super) fn assistant_msg(content: Vec<AssistantPart>) -> LanguageModelMessage {
    LanguageModelMessage::Assistant {
        content,
        provider_options: None,
    }
}
