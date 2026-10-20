//! The wiremock harness shared by the `@ai-sdk/groq` chat ports: every test
//! builds the model through the Groq package (`create_groq(..).chat(model)`).

pub(super) use aimux_core::tool::RawToolCall;
pub(super) use futures::StreamExt;
pub(super) use serde_json::{Value, json};
pub(super) use wiremock::matchers::{method, path};
pub(super) use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) use serial_test::serial;

pub(super) use aimux_core::AiMuxError;
pub(super) use aimux_core::language_model::LanguageModel;
pub(super) use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultOutput, ToolResultPart, UserPart,
};
pub(super) use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, Tool};
pub(super) use aimux_core::provider::Provider;
pub(super) use aimux_core::result::{GenerateContent, ReasoningOutput};
pub(super) use aimux_core::shared::{FileBytes, FileData};
pub(super) use aimux_core::stream_part::StreamPart;
pub(super) use aimux_core::tool::FunctionTool;
pub(super) use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

pub(super) use aimux_providers::groq::{
    GroqChatLanguageModel, GroqProviderSettings, create_groq, groq,
};

// ── helpers ─────────────────────────────────────────────────────────────────

/// The TS `TEST_PROMPT`: a single user text message "Hello".
pub(super) fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `CallOptions` with only `prompt` set.
pub(super) fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// A Groq chat model pointed at the mock server.
pub(super) fn model_at(server: &MockServer, model_id: &str) -> GroqChatLanguageModel {
    create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .expect("groq provider should build")
    .chat(model_id)
}

pub(super) fn browser_search_model_ids() -> Vec<&'static str> {
    include_str!("../../../src/groq/browser_search_models.rs")
        .split('"')
        .filter(|value| value.starts_with("openai/"))
        .collect()
}

/// Build the default test model pointed at the mock server.
pub(super) fn make_provider(server: &MockServer) -> GroqChatLanguageModel {
    model_at(server, "gemma2-9b-it")
}

/// A standard non-streaming chat-completion JSON body returning "Hello, World!".
pub(super) fn text_completion_body() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gemma2-9b-it",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Hello, World!" },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 4, "total_tokens": 34, "completion_tokens": 30 }
    })
}

/// The groq-text fixture (inline).
pub(super) fn groq_text_body() -> Value {
    json!({
        "id": "chatcmpl-09d64d2a-ed1c-4473-829f-78db43f45d13",
        "object": "chat.completion",
        "created": 1770770798,
        "model": "llama-3.3-70b-versatile",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "Luminaria holiday response text." },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 45,
            "completion_tokens": 607,
            "total_tokens": 652
        }
    })
}

/// The groq-reasoning fixture (inline).
pub(super) fn groq_reasoning_body() -> Value {
    json!({
        "id": "chatcmpl-73cf8a54-d54e-400c-88b8-603d1a346d96",
        "object": "chat.completion",
        "created": 1770770833,
        "model": "qwen/qwen3-32b",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "The word \"strawberry\" contains 3 R's.",
                "reasoning": "Okay, so the user is asking how many times the letter r appears in the word strawberry."
            },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 17,
            "completion_tokens": 649,
            "total_tokens": 666,
            "completion_tokens_details": {
                "reasoning_tokens": 570
            }
        }
    })
}

/// The groq-tool-call fixture (inline).
pub(super) fn groq_tool_call_body() -> Value {
    json!({
        "id": "chatcmpl-1fd017fc-60b8-44eb-a736-375b8e1bc3e7",
        "object": "chat.completion",
        "created": 1770770815,
        "model": "llama-3.3-70b-versatile",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "ax9fskhev",
                    "type": "function",
                    "function": { "name": "weather", "arguments": "{}" }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 218,
            "completion_tokens": 15,
            "total_tokens": 233
        }
    })
}

/// Build a single SSE `data: <json>\n\n` event string.
pub(super) fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
pub(super) fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
pub(super) async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(_) => break,
        }
    }
    parts
}

/// Mount a JSON mock response on the server.
pub(super) async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE mock response on the server.
pub(super) async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Get the first received request body as JSON.
pub(super) async fn first_request_body(server: &MockServer) -> Value {
    let requests = server
        .received_requests()
        .await
        .expect("no requests received");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("invalid JSON body");
    body
}

pub(super) async fn failed_call(status: u16, body: Value) -> aimux_core::error::ApiCallError {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    match make_provider(&server)
        .do_generate(&default_options(test_prompt()))
        .await
    {
        Err(aimux_core::AiMuxError::ApiCall(error)) => *error,
        other => panic!("expected an API call error, got {other:?}"),
    }
}

pub(super) fn groq_at(server: &MockServer) -> aimux_providers::groq::GroqProvider {
    create_groq(GroqProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some("test-api-key".to_string()),
        ..Default::default()
    })
    .unwrap()
}

pub(super) fn options_with(provider_options: Value) -> CallOptions {
    let mut options = default_options(test_prompt());
    options.provider_options = Some(serde_json::from_value(provider_options).unwrap());
    options
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-groq-chat-messages.test.ts
// ════════════════════════════════════════════════════════════════════════════
