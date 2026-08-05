//! OpenAI compatible output format — round-trip and cross-protocol tests.
//!
//! These tests verify RFC-0026: that `to_chat_completion` /
//! `to_chat_completion_stream` produce correct OpenAI Chat Completions output
//! from any provider's `GenerateResult` / `StreamPart` stream.
//!
//! ## Test categories
//!
//! 1. **Round-trip (OpenAI → aimux → OpenAI)**: mock a known OpenAI response,
//!    replay through `generate_text`, convert back to `ChatCompletion`, assert
//!    key fields match the original. Verifies lossless conversion.
//!
//! 2. **Cross-protocol (Anthropic → aimux → OpenAI)**: replay Anthropic
//!    cassettes, convert to `ChatCompletion`, assert valid OpenAI format.
//!
//! 3. **Streaming round-trip**: mock a known OpenAI SSE stream, replay through
//!    `stream_text`, convert to `ChatCompletionChunk` stream, assert structure.
//!
//! 4. **End-to-end**: `generate_text_as_openai` / `stream_text_as_openai`.

mod common;

use common::replay;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text, stream_text_as_openai,
};
use aimux_core::openai_output::{
    ChatCompletionChunk, OpenAiStreamOptions, encode_chunk_sse, to_chat_completion,
    to_chat_completion_stream,
};
use aimux_providers::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIProvider};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Build an OpenAIProvider pointed at the replay server.
fn openai_provider(uri: &str) -> aimux_providers::openai::OpenAIModel {
    let provider = OpenAIProvider::new(
        OpenAIConfig::new("test-key")
            .with_base_url(format!("{}/v1", uri))
            .with_profile(OpenAICompatProfile::full()),
    );
    provider.model("gpt-4o")
}

/// Mount a single non-streaming OpenAI response on the mock server.
async fn mock_openai_completion(server: &MockServer, response: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(serde_json::to_string(&response).unwrap()),
        )
        .mount(server)
        .await;
}

/// Mount a single streaming OpenAI SSE response on the mock server.
async fn mock_openai_stream(server: &MockServer, sse_body: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body.to_string()),
        )
        .mount(server)
        .await;
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Round-trip: OpenAI → aimux → OpenAI (non-streaming)
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn round_trip_text_only() {
    let original = json!({
        "id": "chatcmpl-test-001",
        "object": "chat.completion",
        "created": 1711115037,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Rust is a systems programming language."
            },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 8,
            "total_tokens": 18
        }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original.clone()).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(&model, "What is Rust?", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    // ── Exact field round-trip ──
    assert_eq!(completion.id, "chatcmpl-test-001");
    assert_eq!(completion.object, "chat.completion");
    assert_eq!(completion.model, "gpt-4o");
    assert_eq!(completion.choices.len(), 1);
    assert_eq!(completion.choices[0].index, 0);
    assert_eq!(completion.choices[0].message.role, "assistant");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("Rust is a systems programming language.")
    );
    assert_eq!(completion.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(completion.usage.prompt_tokens, 10);
    assert_eq!(completion.usage.completion_tokens, 8);
    assert_eq!(completion.usage.total_tokens, 18);
}

#[tokio::test]
async fn round_trip_with_tool_calls() {
    let original = json!({
        "id": "chatcmpl-test-002",
        "object": "chat.completion",
        "created": 1711115038,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_abc123",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"city\":\"Tokyo\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 20,
            "completion_tokens": 15,
            "total_tokens": 35
        }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original.clone()).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(&model, "Weather in Tokyo?", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    // content should be null (tool_calls only).
    assert_eq!(completion.choices[0].message.content, None);

    let tool_calls = completion.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls should be present");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_abc123");
    assert_eq!(tool_calls[0].tool_type, "function");
    assert_eq!(tool_calls[0].function.name, "get_weather");
    // arguments should round-trip as a JSON string.
    assert_eq!(tool_calls[0].function.arguments, r#"{"city":"Tokyo"}"#);

    assert_eq!(
        completion.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[tokio::test]
async fn round_trip_with_reasoning() {
    let original = json!({
        "id": "chatcmpl-test-003",
        "object": "chat.completion",
        "created": 1711115039,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "The answer is 42.",
                "reasoning_content": "Let me think about this..."
            },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 10,
            "total_tokens": 15,
            "completion_tokens_details": {
                "reasoning_tokens": 7
            }
        }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original.clone()).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(
        &model,
        "What is the answer?",
        GenerateTextOptions::default(),
    )
    .await
    .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("The answer is 42.")
    );
    assert_eq!(
        completion.choices[0].message.reasoning_content.as_deref(),
        Some("Let me think about this...")
    );

    // reasoning_tokens should round-trip.
    let details = completion
        .usage
        .completion_tokens_details
        .as_ref()
        .expect("completion_tokens_details should be present");
    assert_eq!(details.reasoning_tokens, Some(7));
}

#[tokio::test]
async fn round_trip_with_cached_tokens() {
    let original = json!({
        "id": "chatcmpl-test-004",
        "object": "chat.completion",
        "created": 1711115040,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Cached response."
            },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 5,
            "total_tokens": 105,
            "prompt_tokens_details": {
                "cached_tokens": 80
            }
        }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original.clone()).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    // KV cache token should round-trip — this is the critical assertion for
    // cache-hit preservation across the conversion.
    let details = completion
        .usage
        .prompt_tokens_details
        .as_ref()
        .expect("prompt_tokens_details should be present");
    assert_eq!(details.cached_tokens, 80);
    assert_eq!(completion.usage.prompt_tokens, 100);
}

#[tokio::test]
async fn round_trip_multiple_tool_calls() {
    let original = json!({
        "id": "chatcmpl-test-005",
        "object": "chat.completion",
        "created": 1711115041,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call_001",
                        "type": "function",
                        "function": { "name": "search", "arguments": "{\"q\":\"rust\"}" }
                    },
                    {
                        "id": "call_002",
                        "type": "function",
                        "function": { "name": "fetch", "arguments": "{\"url\":\"https://example.com\"}" }
                    }
                ]
            },
            "finish_reason": "tool_calls",
            "logprobs": null
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30 }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original.clone()).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(&model, "Search and fetch", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    let tool_calls = completion.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls should be present");
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].id, "call_001");
    assert_eq!(tool_calls[0].function.name, "search");
    assert_eq!(tool_calls[0].function.arguments, r#"{"q":"rust"}"#);
    assert_eq!(tool_calls[1].id, "call_002");
    assert_eq!(tool_calls[1].function.name, "fetch");
    assert_eq!(
        tool_calls[1].function.arguments,
        r#"{"url":"https://example.com"}"#
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Cross-protocol: Anthropic → aimux → OpenAI
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cross_protocol_anthropic_to_openai_non_streaming() {
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};

    let server = MockServer::start().await;
    replay::mount_cassettes(&server, "tests/cassettes/anthropic").await;

    let provider = AnthropicProvider::new(
        AnthropicConfig::new("test-key").with_base_url(format!("{}/v1", server.uri())),
    );
    let model = provider.model("claude-sonnet-4-6");

    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("anthropic generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "claude-sonnet-4-6");

    // ── Validate OpenAI structure ──
    assert_eq!(completion.object, "chat.completion");
    assert_eq!(completion.choices.len(), 1);
    assert_eq!(completion.choices[0].message.role, "assistant");
    assert_eq!(completion.choices[0].index, 0);

    let content = completion.choices[0]
        .message
        .content
        .as_deref()
        .unwrap_or("");
    assert!(
        !content.is_empty(),
        "Anthropic → OpenAI: content should not be empty"
    );

    assert!(
        completion.usage.prompt_tokens > 0,
        "Anthropic → OpenAI: prompt_tokens should be > 0"
    );
    assert!(
        completion.usage.completion_tokens > 0,
        "Anthropic → OpenAI: completion_tokens should be > 0"
    );
    assert_eq!(
        completion.usage.total_tokens,
        completion.usage.prompt_tokens + completion.usage.completion_tokens
    );

    let fr = completion.choices[0]
        .finish_reason
        .as_deref()
        .unwrap_or("stop");
    assert!(
        matches!(fr, "stop" | "length" | "tool_calls" | "content_filter"),
        "finish_reason should be valid OpenAI value, got {}",
        fr
    );
}

#[tokio::test]
async fn cross_protocol_anthropic_to_openai_streaming() {
    use aimux_providers::anthropic::{AnthropicConfig, AnthropicProvider};

    let server = MockServer::start().await;
    replay::mount_cassettes(&server, "tests/cassettes/anthropic").await;

    let provider = AnthropicProvider::new(
        AnthropicConfig::new("test-key").with_base_url(format!("{}/v1", server.uri())),
    );
    let model = provider.model("claude-sonnet-4-6");

    let result = stream_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("anthropic stream_text should succeed");

    let stream_result = to_chat_completion_stream(
        result.stream,
        "claude-sonnet-4-6",
        OpenAiStreamOptions::default(),
    );

    let mut chunks = Vec::new();
    let mut stream = stream_result.stream;
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chunk should be ok"));
    }

    assert!(!chunks.is_empty(), "should produce at least one chunk");

    // First chunk should have role:assistant.
    assert!(
        chunks[0]
            .choices
            .first()
            .and_then(|c| c.delta.role.as_deref())
            == Some("assistant"),
        "first chunk should have role=assistant"
    );

    // At least one chunk should have content.
    let has_content = chunks.iter().any(|c| {
        c.choices
            .first()
            .and_then(|ch| ch.delta.content.as_deref())
            .is_some_and(|s| !s.is_empty())
    });
    assert!(has_content, "at least one chunk should have content");

    // Last chunk should have finish_reason.
    let last = chunks.last().unwrap();
    let fr = last
        .choices
        .first()
        .and_then(|c| c.finish_reason.as_deref())
        .unwrap_or("stop");
    assert!(
        matches!(fr, "stop" | "length" | "tool_calls" | "content_filter"),
        "last chunk finish_reason should be valid, got {}",
        fr
    );

    for chunk in &chunks {
        assert_eq!(chunk.object, "chat.completion.chunk");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Streaming round-trip: OpenAI → aimux → OpenAI
// ═════════════════════════════════════════════════════════════════════════════

/// A simple OpenAI SSE stream: role frame + 2 content deltas + finish.
const TEXT_SSE: &str = "\
data: {\"id\":\"chatcmpl-s1\",\"object\":\"chat.completion.chunk\",\"created\":1711115100,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-s1\",\"object\":\"chat.completion.chunk\",\"created\":1711115100,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello \"},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-s1\",\"object\":\"chat.completion.chunk\",\"created\":1711115100,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world\"},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-s1\",\"object\":\"chat.completion.chunk\",\"created\":1711115100,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\",\"logprobs\":null}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n\
data: [DONE]\n\n";

#[tokio::test]
async fn round_trip_streaming_text() {
    let server = MockServer::start().await;
    mock_openai_stream(&server, TEXT_SSE).await;

    let model = openai_provider(&server.uri());
    let result = stream_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let stream_result =
        to_chat_completion_stream(result.stream, "gpt-4o", OpenAiStreamOptions::default());

    let mut chunks: Vec<ChatCompletionChunk> = Vec::new();
    let mut stream = stream_result.stream;
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chunk should be ok"));
    }

    assert!(!chunks.is_empty(), "should produce chunks");

    // First chunk: role frame.
    assert_eq!(chunks[0].object, "chat.completion.chunk");
    assert!(
        chunks[0]
            .choices
            .first()
            .and_then(|c| c.delta.role.as_deref())
            == Some("assistant"),
        "first chunk should set role=assistant"
    );

    // Collect content deltas — should reconstruct "Hello world".
    let full_content: String = chunks
        .iter()
        .filter_map(|c| c.choices.first().and_then(|ch| ch.delta.content.as_deref()))
        .collect();
    assert_eq!(full_content, "Hello world");

    // Last chunk: finish_reason + usage.
    let last = chunks.last().unwrap();
    assert_eq!(
        last.choices
            .first()
            .and_then(|c| c.finish_reason.as_deref()),
        Some("stop")
    );
    let usage = last.usage.as_ref().expect("last chunk should have usage");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
}

#[tokio::test]
async fn round_trip_streaming_no_usage() {
    let server = MockServer::start().await;
    mock_openai_stream(&server, TEXT_SSE).await;

    let model = openai_provider(&server.uri());
    let result = stream_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let stream_result = to_chat_completion_stream(
        result.stream,
        "gpt-4o",
        OpenAiStreamOptions {
            include_usage: false,
            include_reasoning: true,
        },
    );

    let mut chunks = Vec::new();
    let mut stream = stream_result.stream;
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chunk should be ok"));
    }

    let last = chunks.last().unwrap();
    assert!(
        last.usage.is_none(),
        "last chunk should not have usage when include_usage=false"
    );
    assert!(
        last.choices
            .first()
            .and_then(|c| c.finish_reason.as_deref())
            .is_some(),
        "last chunk should still have finish_reason"
    );
}

/// OpenAI SSE stream with tool calls.
const TOOL_CALL_SSE: &str = "\
data: {\"id\":\"chatcmpl-t1\",\"object\":\"chat.completion.chunk\",\"created\":1711115200,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-t1\",\"object\":\"chat.completion.chunk\",\"created\":1711115200,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-t1\",\"object\":\"chat.completion.chunk\",\"created\":1711115200,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\\\"Tokyo\\\"}\"}}]},\"finish_reason\":null,\"logprobs\":null}],\"usage\":null}\n\n\
data: {\"id\":\"chatcmpl-t1\",\"object\":\"chat.completion.chunk\",\"created\":1711115200,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\",\"logprobs\":null}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n\
data: [DONE]\n\n";

#[tokio::test]
async fn round_trip_streaming_tool_calls() {
    let server = MockServer::start().await;
    mock_openai_stream(&server, TOOL_CALL_SSE).await;

    let model = openai_provider(&server.uri());
    let result = stream_text(&model, "Weather?", GenerateTextOptions::default())
        .await
        .expect("stream_text should succeed");

    let stream_result =
        to_chat_completion_stream(result.stream, "gpt-4o", OpenAiStreamOptions::default());

    let mut chunks: Vec<ChatCompletionChunk> = Vec::new();
    let mut stream = stream_result.stream;
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk.expect("chunk should be ok"));
    }

    // Find the tool_call opening chunk (with id + name).
    let open_chunk = chunks
        .iter()
        .find(|c| {
            c.choices
                .first()
                .and_then(|ch| ch.delta.tool_calls.as_ref())
                .and_then(|tcs| tcs.first())
                .and_then(|tc| tc.id.as_deref())
                == Some("call_1")
        })
        .expect("tool call opening chunk not found");

    let tc = open_chunk.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .unwrap()
        .first()
        .unwrap();
    assert_eq!(tc.index, 0);
    assert_eq!(tc.id.as_deref(), Some("call_1"));
    assert_eq!(tc.tool_type.as_deref(), Some("function"));
    assert_eq!(tc.function.name.as_deref(), Some("get_weather"));

    // Find argument delta.
    let arg_chunk = chunks
        .iter()
        .find(|c| {
            c.choices
                .first()
                .and_then(|ch| ch.delta.tool_calls.as_ref())
                .and_then(|tcs| tcs.first())
                .is_some_and(|tc| tc.id.is_none() && tc.function.arguments.is_some())
        })
        .expect("argument delta chunk not found");
    let arg_tc = arg_chunk.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .unwrap()
        .first()
        .unwrap();
    assert_eq!(
        arg_tc.function.arguments.as_deref(),
        Some(r#"{"city":"Tokyo"}"#)
    );

    // Last chunk: finish_reason=tool_calls.
    let last = chunks.last().unwrap();
    assert_eq!(
        last.choices
            .first()
            .and_then(|c| c.finish_reason.as_deref()),
        Some("tool_calls")
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. End-to-end: generate_text_as_openai / stream_text_as_openai
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_generate_text_as_openai() {
    let original = json!({
        "id": "chatcmpl-e2e-001",
        "object": "chat.completion",
        "created": 1711115300,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "E2E test response." },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 4, "total_tokens": 9 }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original).await;

    let model = openai_provider(&server.uri());
    let completion = generate_text_as_openai(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("generate_text_as_openai should succeed");

    assert_eq!(completion.object, "chat.completion");
    assert_eq!(completion.id, "chatcmpl-e2e-001");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("E2E test response.")
    );
    assert_eq!(completion.usage.total_tokens, 9);
}

#[tokio::test]
async fn e2e_stream_text_as_openai() {
    let server = MockServer::start().await;
    mock_openai_stream(&server, TEXT_SSE).await;

    let model = openai_provider(&server.uri());
    let result = stream_text_as_openai(
        &model,
        "Hello",
        GenerateTextOptions::default(),
        OpenAiStreamOptions::default(),
    )
    .await
    .expect("stream_text_as_openai should succeed");

    let mut stream = result.stream;
    let mut chunk_count = 0;
    let mut has_role = false;
    let mut has_content = false;
    let mut has_finish = false;
    let mut full_content = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("chunk should be ok");
        chunk_count += 1;
        assert_eq!(chunk.object, "chat.completion.chunk");

        if let Some(choice) = chunk.choices.first() {
            if choice.delta.role.as_deref() == Some("assistant") {
                has_role = true;
            }
            if let Some(content) = choice.delta.content.as_deref()
                && !content.is_empty()
            {
                has_content = true;
                full_content.push_str(content);
            }
            if choice.finish_reason.is_some() {
                has_finish = true;
            }
        }
    }

    assert!(chunk_count > 0, "should produce chunks");
    assert!(has_role, "should have a role frame");
    assert!(has_content, "should have content deltas");
    assert_eq!(full_content, "Hello world");
    assert!(has_finish, "should have a finish frame");
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. SSE encoding
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sse_encoding_produces_valid_frames() {
    let server = MockServer::start().await;
    mock_openai_stream(&server, TEXT_SSE).await;

    let model = openai_provider(&server.uri());
    let result = stream_text_as_openai(
        &model,
        "Hello",
        GenerateTextOptions::default(),
        OpenAiStreamOptions::default(),
    )
    .await
    .expect("stream_text_as_openai should succeed");

    let mut stream = result.stream;
    let mut sse_count = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("chunk should be ok");
        let sse = encode_chunk_sse(&chunk);
        assert!(
            sse.starts_with("data: "),
            "SSE frame must start with 'data: '"
        );
        assert!(sse.ends_with("\n\n"), "SSE frame must end with '\\n\\n'");
        let json_str = &sse["data: ".len()..sse.len() - 2];
        let parsed: Value =
            serde_json::from_str(json_str).expect("SSE payload should be valid JSON");
        assert_eq!(parsed["object"], "chat.completion.chunk");
        sse_count += 1;
    }

    assert!(sse_count > 0, "should produce SSE frames");
}

// ═════════════════════════════════════════════════════════════════════════════
// 6. DeepSeek (OpenAI-compatible with reasoning_content)
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cross_protocol_deepseek_with_reasoning() {
    // DeepSeek returns reasoning_content in non-streaming responses.
    let original = json!({
        "id": "chatcmpl-ds-001",
        "object": "chat.completion",
        "created": 1711115400,
        "model": "deepseek-chat",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "The answer is 42.",
                "reasoning_content": "I need to calculate the meaning of life."
            },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 8,
            "completion_tokens": 12,
            "total_tokens": 20,
            "completion_tokens_details": { "reasoning_tokens": 8 }
        }
    });

    let server = MockServer::start().await;
    // DeepSeek's endpoint is /chat/completions (no /v1 prefix).
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(serde_json::to_string(&original).unwrap()),
        )
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(
        OpenAIConfig::new("test-key")
            .with_base_url(server.uri())
            .with_profile(OpenAICompatProfile::deepseek()),
    );
    let model = provider.model("deepseek-chat");

    let result = generate_text(
        &model,
        "What is the answer?",
        GenerateTextOptions::default(),
    )
    .await
    .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "deepseek-chat");

    assert_eq!(completion.object, "chat.completion");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("The answer is 42.")
    );
    assert_eq!(
        completion.choices[0].message.reasoning_content.as_deref(),
        Some("I need to calculate the meaning of life.")
    );

    let details = completion
        .usage
        .completion_tokens_details
        .as_ref()
        .expect("completion_tokens_details should be present");
    assert_eq!(details.reasoning_tokens, Some(8));
}

// ═════════════════════════════════════════════════════════════════════════════
// 7. KV cache token preservation (critical for multi-turn cache hits)
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn kv_cache_tokens_preserved_with_cache_write() {
    let original = json!({
        "id": "chatcmpl-cache-001",
        "object": "chat.completion",
        "created": 1711115500,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "ok" },
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 1,
            "total_tokens": 101,
            "prompt_tokens_details": {
                "cached_tokens": 60,
                "cache_write_tokens": 30
            }
        }
    });

    let server = MockServer::start().await;
    mock_openai_completion(&server, original).await;

    let model = openai_provider(&server.uri());
    let result = generate_text(&model, "Hello", GenerateTextOptions::default())
        .await
        .expect("generate_text should succeed");

    let completion = to_chat_completion(&result.raw, "gpt-4o");

    let details = completion
        .usage
        .prompt_tokens_details
        .as_ref()
        .expect("prompt_tokens_details should be present");

    // Both cache_read (cached_tokens) and cache_write must survive the round-trip.
    assert_eq!(details.cached_tokens, 60);
    assert_eq!(details.cache_write_tokens, Some(30));
    assert_eq!(completion.usage.prompt_tokens, 100);
}
