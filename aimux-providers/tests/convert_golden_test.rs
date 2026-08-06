//! Golden equivalence tests for the three request-body converters that were
//! split during M11 (#76).
//!
//! The splits were pure reorganization; these tests pin each converter's FULL
//! request body + warning list for a representative option matrix so any
//! future refactor (or a regression like the Anthropic default-budget drop)
//! is caught by an exact comparison, not by a partial assertion.

use serde_json::json;

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::{FunctionTool, Tool};

use aimux_providers::anthropic::convert::build_request_body_with_warnings as anthropic_build;
use aimux_providers::openai::convert::build_request_body_with_warnings as openai_build;
use aimux_providers::openai::responses::convert::build_responses_request_body;

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
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
        body_overrides: Some(json!({ "user": "override-me" })),
        ..CallOptions::default()
    };

    let result = openai_build(
        "o3-mini",
        &options,
        false,
        "openai",
        &aimux_providers::openai::OpenAICompatProfile::full(),
    )
    .expect("openai chat build");

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
            "tool_choice": "auto",
            "user": "override-me"
        }),
        "openai chat body diverged: {}",
        result.body
    );
    // `temperature` is stripped for reasoning models with a warning; the
    // body override `user` lands after built-in fields.
    assert_eq!(result.body.get("temperature"), None);
    assert_eq!(
        serde_json::to_value(&result.warnings).unwrap(),
        json!([{ "Unsupported": { "feature": "temperature",
                 "details": "temperature is not supported for reasoning models" } }])
    );
}

// ── OpenAI Responses API ────────────────────────────────────────────────────

#[test]
fn responses_golden() {
    let mut provider = std::collections::HashMap::new();
    provider.insert(
        "openai".to_string(),
        json!({
            "reasoningEffort": "high",
            "textVerbosity": "low",
            "metadata": { "k": "v" }
        }),
    );
    let options = CallOptions {
        prompt: user_prompt(),
        seed: Some(42),
        top_k: Some(0.2),
        response_format: Some(json_schema_format()),
        provider_options: Some(provider),
        ..CallOptions::default()
    };

    let result = build_responses_request_body("o3-mini", &options, false);

    assert_eq!(
        result.body,
        json!({
            "model": "o3-mini",
            "input": [
                { "role": "user", "content": [ { "type": "input_text", "text": "Hello" } ] }
            ],
            "text": {
                "format": {
                    "type": "json_schema",
                    "strict": true,
                    "name": "qa",
                    "description": "answer extraction",
                    "schema": {
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"]
                    }
                },
                "verbosity": "low"
            },
            "metadata": { "k": "v" },
            "reasoning": { "effort": "high", "summary": "detailed" }
        }),
        "responses body diverged: {}",
        result.body
    );

    // top_k / seed are unsupported in the Responses API → compatibility
    // warnings, and the model is a reasoning model so it exposes `reasoning`.
    assert_eq!(
        serde_json::to_value(&result.warnings).unwrap(),
        json!([
            { "Unsupported": { "feature": "topK", "details": null } },
            { "Unsupported": { "feature": "seed", "details": null } }
        ])
    );
}

// ── Anthropic ───────────────────────────────────────────────────────────────

#[test]
fn anthropic_golden() {
    let mut provider = std::collections::HashMap::new();
    provider.insert(
        "anthropic".to_string(),
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
            "stream": false,
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
