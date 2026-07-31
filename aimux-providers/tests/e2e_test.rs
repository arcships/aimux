//! End-to-end tests: verify generate_text / stream_text user-facing API
//! works with both OpenAI and Anthropic providers via wiremock.

use aimux_core::content::ContentPart;
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::message::{MessageContent, ModelMessage, Role};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool, ToolChoice};
use aimux_core::types::FinishReasonUnified;
use aimux_providers::{AnthropicConfig, AnthropicProvider, OpenAIConfig, OpenAIProvider};
use futures::StreamExt;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

// ─────────────────────────────────────────────────────────────────
// OpenAI end-to-end
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_generate_text() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-test",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "Rust is a systems programming language."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let result = generate_text(&model, "What is Rust?", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    assert_eq!(result.text, "Rust is a systems programming language.");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(10));
    assert_eq!(result.usage.output_tokens.total, Some(8));
    assert!(result.tool_calls.is_empty());
}

#[tokio::test]
async fn e2e_openai_stream_text() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let result = stream_text(&model, "Say hello", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let text = result.text().await.expect("should collect text");
    assert_eq!(text, "Hello world");
}

#[tokio::test]
async fn e2e_openai_generate_with_tools() {
    let server = MockServer::start().await;

    // First call: model requests a tool call
    // Second call: model returns final text after tool result
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-2",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "The weather in Tokyo is sunny."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 30, "completion_tokens": 8, "total_tokens": 38}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let result = generate_text(
        &model,
        vec![ModelMessage::user("What's the weather in Tokyo?")],
        GenerateTextOptions {
            tools: Some(vec![Tool::Function(FunctionTool {
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }),
                strict: None,
                provider_options: None,
                input_examples: None,
            })]),
            ..Default::default()
        },
    )
    .await
    .expect("generate_text should succeed");

    // The first call should return a tool call
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "get_weather");
    assert_eq!(result.tool_calls[0].input["location"], "Tokyo");
}

#[tokio::test]
async fn e2e_openai_error_401() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "Invalid API key", "type": "invalid_request_error"}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let result = generate_text(&model, "test", GenerateTextOptions::default()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, aimux_core::AiMuxError::Auth(_)),
        "expected Auth error, got {:?}",
        err
    );
}

// ─────────────────────────────────────────────────────────────────
// Anthropic end-to-end
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_anthropic_generate_text() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_test",
            "model": "claude-3-5-sonnet-20241022",
            "content": [{"type": "text", "text": "Rust is memory-safe."}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 12, "output_tokens": 5}
        })))
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("claude-3-5-sonnet-20241022");

    let result = generate_text(
        &model,
        vec![ModelMessage::user("What is Rust?")],
        GenerateTextOptions::default(),
    )
    .await
    .expect("generate_text should succeed");

    assert_eq!(result.text, "Rust is memory-safe.");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(12));
    assert_eq!(result.usage.output_tokens.total, Some(5));
}

#[tokio::test]
async fn e2e_anthropic_stream_text() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-3\",\"usage\":{\"input_tokens\":5}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"!\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("claude-3-5-sonnet-20241022");

    let result = stream_text(
        &model,
        vec![ModelMessage::user("Say hello")],
        GenerateTextOptions::default(),
    )
    .await
    .expect("stream_text should succeed");

    let text = result.text().await.expect("should collect text");
    assert_eq!(text, "Hello!");
}

#[tokio::test]
async fn e2e_anthropic_error_429() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"message": "Rate limit exceeded", "type": "rate_limit_error"}
        })))
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("claude-3-5-sonnet-20241022");

    let result = generate_text(&model, "test", GenerateTextOptions::default()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, aimux_core::AiMuxError::RateLimited { .. }),
        "expected RateLimited error, got {:?}",
        err
    );
}

// ─────────────────────────────────────────────────────────────────
// Stream part sequence verification
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_stream_part_sequence() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let result = stream_text(&model, "test", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(part) = stream.next().await {
        parts.push(part.expect("stream part should be ok"));
    }

    // Expected sequence: StreamStart, TextStart, TextDelta, TextEnd, Finish
    // (ResponseMetadata may or may not be present depending on implementation)
    assert!(
        parts.len() >= 5,
        "expected at least 5 parts, got {}",
        parts.len()
    );

    // First part must be StreamStart
    assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

    // Last part must be Finish
    let last = parts.last().unwrap();
    assert!(matches!(last, StreamPart::Finish { .. }));

    // Must contain at least one TextDelta
    let has_text_delta = parts
        .iter()
        .any(|p| matches!(p, StreamPart::TextDelta { delta, .. } if delta == "Hi"));
    assert!(has_text_delta, "expected a TextDelta with 'Hi'");
}

#[tokio::test]
async fn e2e_anthropic_stream_part_sequence() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-3\",\"usage\":{\"input_tokens\":5}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(AnthropicConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("claude-3-5-sonnet-20241022");

    let result = stream_text(&model, "test", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(part) = stream.next().await {
        parts.push(part.expect("stream part should be ok"));
    }

    // First part must be StreamStart
    assert!(matches!(parts[0], StreamPart::StreamStart { .. }));

    // Last part must be Finish
    let last = parts.last().unwrap();
    assert!(matches!(last, StreamPart::Finish { .. }));

    // Must contain at least one TextDelta
    let has_text_delta = parts
        .iter()
        .any(|p| matches!(p, StreamPart::TextDelta { delta, .. } if delta == "Hi"));
    assert!(has_text_delta, "expected a TextDelta with 'Hi'");
}

// ─────────────────────────────────────────────────────────────────
// Provider interchangeability: same user API, different providers
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_provider_interchangeability() {
    // Verify that the same generate_text call works with different providers
    // through the LanguageModel trait (the core goal of this SDK).

    // OpenAI
    let openai_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "model": "gpt-4o",
            "choices": [{"message": {"role": "assistant", "content": "from-openai"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })))
        .mount(&openai_server)
        .await;

    let openai = OpenAIProvider::new(OpenAIConfig::new("key").with_base_url(openai_server.uri()))
        .model("gpt-4o");

    let openai_result = generate_text(&openai, "test", GenerateTextOptions::default())
        .await
        .expect("openai generate should succeed");
    assert_eq!(openai_result.text, "from-openai");

    // Anthropic
    let anthropic_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "1", "model": "claude-3",
            "content": [{"type": "text", "text": "from-anthropic"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .mount(&anthropic_server)
        .await;

    let anthropic =
        AnthropicProvider::new(AnthropicConfig::new("key").with_base_url(anthropic_server.uri()))
            .model("claude-3");

    let anthropic_result = generate_text(&anthropic, "test", GenerateTextOptions::default())
        .await
        .expect("anthropic generate should succeed");
    assert_eq!(anthropic_result.text, "from-anthropic");

    // Both use the same user-facing API (generate_text) and return the same result type
    assert_eq!(
        openai_result.finish_reason.unified,
        FinishReasonUnified::Stop
    );
    assert_eq!(
        anthropic_result.finish_reason.unified,
        FinishReasonUnified::Stop
    );
}

// ─────────────────────────────────────────────────────────────────
// Tool-call full round-trip: ToolCall → user executes → ToolResult → final text
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_tool_call_round_trip() {
    let server = MockServer::start().await;

    // First call: model requests a tool call.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Second call: model returns final text after receiving the tool result.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-2",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "The weather in Tokyo is sunny."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 30, "completion_tokens": 8, "total_tokens": 38}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let weather_tool = || {
        Tool::Function(
            FunctionTool::new(
                "get_weather",
                json!({
                    "type": "object",
                    "properties": {"location": {"type": "string"}},
                    "required": ["location"]
                }),
            )
            .with_description("Get weather for a location"),
        )
    };

    // Step 1: first call — model requests a tool call.
    let result1 = generate_text(
        &model,
        vec![ModelMessage::user("What's the weather in Tokyo?")],
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
    )
    .await
    .expect("first generate_text should succeed");

    assert_eq!(
        result1.finish_reason.unified,
        FinishReasonUnified::ToolCalls
    );
    assert_eq!(result1.tool_calls.len(), 1);
    assert_eq!(result1.tool_calls[0].tool_name, "get_weather");
    assert_eq!(result1.tool_calls[0].input["location"], "Tokyo");
    assert_eq!(result1.tool_calls[0].tool_call_id, "call_abc");

    // Step 2: user executes the tool, then builds the full conversation:
    //   user → assistant(tool_call) → tool(result)
    let messages = vec![
        ModelMessage::user("What's the weather in Tokyo?"),
        ModelMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::tool_call(
                "call_abc",
                "get_weather",
                json!({"location": "Tokyo"}),
            )]),
        },
        ModelMessage {
            role: Role::Tool,
            content: MessageContent::Parts(vec![ContentPart::tool_result(
                "call_abc",
                json!({"temperature": 22, "condition": "sunny"}),
            )]),
        },
    ];

    // Step 3: second call — model returns final text after the tool result.
    let result2 = generate_text(
        &model,
        messages,
        GenerateTextOptions {
            tools: Some(vec![weather_tool()]),
            ..Default::default()
        },
    )
    .await
    .expect("second generate_text should succeed");

    assert_eq!(result2.text, "The weather in Tokyo is sunny.");
    assert_eq!(result2.finish_reason.unified, FinishReasonUnified::Stop);
    assert!(result2.tool_calls.is_empty());

    // Step 4: verify the second request carried the full tool round-trip.
    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    assert_eq!(requests.len(), 2, "expected exactly two requests");
    let body: serde_json::Value =
        serde_json::from_slice(&requests[1].body).expect("second request body is JSON");

    let msgs = body["messages"].as_array().expect("messages is an array");
    // user, assistant(tool_calls), tool(result)
    assert_eq!(msgs.len(), 3, "expected 3 messages in second request");
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[1]["role"], "assistant");
    assert!(
        msgs[1]["tool_calls"].is_array(),
        "assistant message carries tool_calls"
    );
    assert_eq!(msgs[1]["tool_calls"][0]["function"]["name"], "get_weather");
    assert_eq!(msgs[1]["tool_calls"][0]["id"], "call_abc");
    assert_eq!(msgs[2]["role"], "tool");
    assert_eq!(msgs[2]["tool_call_id"], "call_abc");
}

// ─────────────────────────────────────────────────────────────────
// Multi-turn dialog: system + user + assistant + user message sequence
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_multi_turn_dialog() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-m",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "Tokyo has about 14 million people."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 40, "completion_tokens": 12, "total_tokens": 52}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    // Full multi-turn conversation:
    //   system → user(q1) → assistant(a1) → user(q2 follow-up)
    let messages = vec![
        ModelMessage::system("You are a helpful geography assistant."),
        ModelMessage::user("What is the capital of Japan?"),
        ModelMessage::assistant("The capital of Japan is Tokyo."),
        ModelMessage::user("And what is its population?"),
    ];

    let result = generate_text(&model, messages, GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    assert_eq!(result.text, "Tokyo has about 14 million people.");
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);

    // Verify the full multi-turn message sequence was sent.
    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("request body is JSON");

    let msgs = body["messages"].as_array().expect("messages is an array");
    assert_eq!(
        msgs.len(),
        4,
        "expected 4 messages (system + user + assistant + user)"
    );
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], "You are a helpful geography assistant.");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[2]["content"], "The capital of Japan is Tokyo.");
    assert_eq!(msgs[3]["role"], "user");
    assert_eq!(msgs[3]["content"], "And what is its population?");
}

// ─────────────────────────────────────────────────────────────────
// ToolChoice end-to-end: verify tool_choice reaches the provider request
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_tool_choice_required() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-tc",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
        })))
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let weather_tool = Tool::Function(
        FunctionTool::new(
            "get_weather",
            json!({
                "type": "object",
                "properties": {"location": {"type": "string"}},
                "required": ["location"]
            }),
        )
        .with_description("Get weather for a location"),
    );

    let result = generate_text(
        &model,
        "What's the weather in Tokyo?",
        GenerateTextOptions {
            tools: Some(vec![weather_tool]),
            tool_choice: Some(ToolChoice::Required),
            ..Default::default()
        },
    )
    .await
    .expect("generate_text should succeed");

    // Model returned a tool call
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "get_weather");

    // Verify the request body carries tool_choice: "required"
    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("request body is JSON");
    assert_eq!(body["tool_choice"], "required");
    assert!(body["tools"].is_array(), "tools array present in request");
}

// ─────────────────────────────────────────────────────────────────
// Streaming tool calls end-to-end: stream_text with tools → ToolCall/ToolInputDelta parts
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_openai_stream_tool_calls() {
    let server = MockServer::start().await;

    let sse_body = concat!(
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_xyz\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]}}]}\n\n",
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"location\\\":\\\"Tokyo\\\"}\"}}]}}]}\n\n",
        "data: {\"id\":\"1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
        "data: [DONE]\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(OpenAIConfig::new("test-key").with_base_url(server.uri()));
    let model = provider.model("gpt-4o");

    let weather_tool = Tool::Function(
        FunctionTool::new(
            "get_weather",
            json!({
                "type": "object",
                "properties": {"location": {"type": "string"}},
                "required": ["location"]
            }),
        )
        .with_description("Get weather for a location"),
    );

    let result = stream_text(
        &model,
        "What's the weather in Tokyo?",
        GenerateTextOptions {
            tools: Some(vec![weather_tool]),
            ..Default::default()
        },
    )
    .await
    .expect("stream_text should succeed");

    let mut stream = result.stream;
    let mut parts = Vec::new();
    while let Some(part) = stream.next().await {
        parts.push(part.expect("stream part should be ok"));
    }

    assert!(!parts.is_empty(), "stream should produce parts");

    // Must contain tool-related parts (not just TextDelta/Finish)
    let has_tool_part = parts.iter().any(|p| {
        matches!(
            p,
            StreamPart::ToolCall { .. }
                | StreamPart::ToolInputStart { .. }
                | StreamPart::ToolInputDelta { .. }
                | StreamPart::ToolInputEnd { .. }
        )
    });
    assert!(
        has_tool_part,
        "expected a tool-related StreamPart; got: {:?}",
        parts
    );

    // Must contain a Finish part
    let has_finish = parts.iter().any(|p| matches!(p, StreamPart::Finish { .. }));
    assert!(has_finish, "expected a Finish StreamPart");

    // If a complete ToolCall part exists, verify its fields
    let tool_call = parts.iter().find_map(|p| match p {
        StreamPart::ToolCall {
            tool_name, input, ..
        } => Some((tool_name, input)),
        _ => None,
    });
    if let Some((tool_name, input)) = tool_call {
        assert_eq!(tool_name, "get_weather");
        assert_eq!(input["location"], "Tokyo");
    }
}
