//! Golden request-body and warning tests for chat and messages conversion.

use serde_json::json;

use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::{FunctionTool, Tool};

use aimux_providers::anthropic::convert::build_request_body_with_warnings as anthropic_build;
use aimux_providers::openai::convert::build_request_body_with_warnings as openai_build;

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn weather_tool() -> FunctionTool {
    FunctionTool {
        name: "weather".to_string(),
        description: Some("current weather".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } },
            "required": ["location"],
            "additionalProperties": false
        }),
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

fn json_schema_format() -> ResponseFormat {
    ResponseFormat::Json {
        schema: Some(json!({
            "type": "object",
            "properties": { "answer": { "type": "string" } },
            "required": ["answer"]
        })),
        name: Some("qa".to_string()),
        description: Some("answer extraction".to_string()),
    }
}

// ── OpenAI Chat Completions ─────────────────────────────────────────────────

#[test]
fn openai_chat_golden() {
    let options = CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(2048),
        temperature: Some(0.7),
        reasoning: Some(aimux_core::types::ReasoningEffort::High),
        tools: Some(vec![Tool::Function(weather_tool())]),
        tool_choice: ToolChoice::Auto,
        response_format: Some(json_schema_format()),
        ..CallOptions::default()
    };

    let result = openai_build("o3-mini", &options, false).expect("openai chat build");

    assert_eq!(
        result.body,
        json!({
            "model": "o3-mini",
            "messages": [ { "role": "user", "content": "Hello" } ],
            "max_completion_tokens": 2048,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "schema": {
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"]
                    },
                    "name": "qa",
                    "description": "answer extraction",
                    "strict": true
                }
            },
            "reasoning_effort": "high",
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "weather",
                        "description": "current weather",
                        "parameters": {
                            "type": "object",
                            "properties": { "location": { "type": "string" } },
                            "required": ["location"],
                            "additionalProperties": false
                        }
                    }
                }
            ],
            "tool_choice": "auto"
        }),
        "openai chat body diverged: {}",
        result.body
    );
    // `temperature` is stripped for reasoning models with a warning.
    assert_eq!(result.body.get("temperature"), None);
    assert_eq!(
        serde_json::to_value(&result.warnings).unwrap(),
        json!([{ "Unsupported": { "feature": "temperature",
                 "details": "temperature is not supported for reasoning models" } }])
    );
}

// ── Anthropic ───────────────────────────────────────────────────────────────

#[test]
fn anthropic_golden() {
    let provider = aimux_core::shared::provider_namespace(
        "anthropic",
        json!({
            "thinking": { "type": "enabled", "budgetTokens": 4096 }
        }),
    );
    let options = CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(1000),
        temperature: Some(0.7),
        top_k: Some(0.5),
        stop_sequences: Some(vec!["END".to_string()]),
        provider_options: Some(provider),
        ..CallOptions::default()
    };

    let result = anthropic_build("claude-sonnet-4-5", &options, false).expect("anthropic build");

    assert_eq!(
        result.body,
        json!({
            "model": "claude-sonnet-4-5",
            "messages": [ { "role": "user", "content": [ { "type": "text", "text": "Hello" } ] } ],
            "max_tokens": 1000 + 4096,
            "thinking": { "type": "enabled", "budget_tokens": 4096 },
            "stop_sequences": ["END"]
        }),
        "anthropic body diverged: {}",
        result.body
    );

    // Thinking enabled strips temperature/topK with warnings.
    assert_eq!(result.body.get("temperature"), None);
    assert_eq!(result.body.get("top_k"), None);
    assert_eq!(
        serde_json::to_value(&result.warnings).unwrap(),
        json!([
            { "Unsupported": { "feature": "temperature",
              "details": "temperature is not supported when thinking is enabled" } },
            { "Unsupported": { "feature": "topK",
              "details": "topK is not supported when thinking is enabled" } }
        ])
    );
}
