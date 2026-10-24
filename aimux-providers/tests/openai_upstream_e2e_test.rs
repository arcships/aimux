//! Small end-to-end subset of the pinned upstream tests, using an in-process transport.
#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::shared::{Size, provider_namespace};
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_core::types::FinishReasonUnified;
use aimux_providers::openai::tools::{FileSearchArgs, file_search};
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        base_url: Some("https://example.test/v1".into()),
        api_key: Some("test-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}
fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}
fn chat_response() -> Value {
    json!({"id":"chatcmpl-test","model":"test-model","created":1711115037,"choices":[{"index":0,"message":{"role":"assistant","content":"Hello, World!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"total_tokens":34,"completion_tokens":30}})
}
fn responses_response() -> Value {
    json!({"id":"resp_test","created_at":1741257730,"output":[{"id":"msg_test","type":"message","content":[{"type":"output_text","text":"answer text","annotations":[]}]}],"usage":{"input_tokens":345,"input_tokens_details":{"cached_tokens":234,"cache_write_tokens":45,"future_input_detail":{"tokens":7}},"output_tokens":538,"output_tokens_details":{"reasoning_tokens":123,"future_output_detail":["preserved"]},"total_tokens":572,"future_usage_field":{"value":true}}})
}
fn sse(events: Vec<Value>) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}
fn assert_request(fetch: &MockFetch, endpoint: &str, expected: Value) {
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("https://example.test/v1{endpoint}"));
    assert_eq!(seen[0].headers["authorization"], "Bearer test-key");
    assert_eq!(seen[0].json_body(), expected);
}

/// TS: "should extract text response" (src/chat/openai-chat-language-model.test.ts)
/// TS: "should send request body" (src/chat/openai-chat-language-model.test.ts)
/// TS: "should extract usage" (src/chat/openai-chat-language-model.test.ts)
#[tokio::test]
async fn chat_generate() {
    let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    let body = json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}]});
    assert_request(&fetch, "/chat/completions", body.clone());
    assert_eq!(result.request.unwrap().body, Some(body));
    assert!(
        matches!(&result.content[..],[GenerateContent::Text{text,..}] if text=="Hello, World!")
    );
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.input_tokens.no_cache, Some(4));
    assert_eq!(result.usage.output_tokens.text, Some(30));
    assert_eq!(
        result.usage.raw,
        chat_response()["usage"].as_object().cloned()
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(
        result.response.unwrap().id.as_deref(),
        Some("chatcmpl-test")
    );
}

/// TS: "should stream text deltas" (src/chat/openai-chat-language-model.test.ts)
#[tokio::test]
async fn chat_stream() {
    let chunk = |delta: Value, finish: Value| json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
    let fetch = MockFetch::new(vec![sse(vec![
        chunk(json!({"role":"assistant","content":""}), Value::Null),
        chunk(json!({"content":"Hello"}), Value::Null),
        chunk(json!({"content":", "}), Value::Null),
        chunk(json!({"content":"World!"}), Value::Null),
        chunk(json!({}), json!("stop")),
        json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[],"usage":{"prompt_tokens":17,"total_tokens":244,"completion_tokens":227}}),
    ])]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_stream(&options())
        .await
        .unwrap();
    let parts = result
        .stream
        .map(|part| part.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_request(
        &fetch,
        "/chat/completions",
        json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"stream":true,"stream_options":{"include_usage":true}}),
    );
    let text = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "Hello, World!");
    assert_eq!(
        parts
            .iter()
            .filter(|part| matches!(part, StreamPart::TextStart { .. }))
            .count(),
        1
    );
    assert_eq!(
        parts
            .iter()
            .filter(|part| matches!(part, StreamPart::TextEnd { .. }))
            .count(),
        1
    );
    assert!(
        matches!(parts.last(),Some(StreamPart::Finish{finish_reason,usage,..}) if finish_reason.unified==FinishReasonUnified::Stop && usage.input_tokens.total==Some(17) && usage.output_tokens.total==Some(227))
    );
}

/// TS: "should parse tool results" (src/chat/openai-chat-language-model.test.ts)
#[tokio::test]
async fn chat_tool_call() {
    let mut response = chat_response();
    response["choices"][0]["message"]["content"] = json!("");
    response["choices"][0]["message"]["tool_calls"] = json!([{"id":"call_test","type":"function","function":{"name":"test-tool","arguments":"{\"value\":\"Spark\"}"}}]);
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let schema = json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false});
    let mut opts = options();
    opts.tools = Some(vec![Tool::Function(FunctionTool::new(
        "test-tool",
        schema.clone(),
    ))]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_generate(&opts)
        .await
        .unwrap();
    let body = fetch.seen()[0].json_body();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"], json!([{"role":"user","content":"Hello"}]));
    assert_eq!(body["tools"][0]["function"]["name"], "test-tool");
    assert_eq!(body["tools"][0]["function"]["parameters"], schema);
    assert!(
        matches!(&result.content[..],[GenerateContent::ToolCall(call)] if call.tool_call_id=="call_test" && call.tool_name=="test-tool" && call.input==r#"{"value":"Spark"}"#)
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/chat/openai-chat-language-model.ts:539-542
/// creates the stream's tool-call tracker with `typeValidation: 'if-present'`:
/// a tool-call delta without `type` starts a call, one whose `type` is
/// present and not `function` is invalid response data.
#[tokio::test]
async fn chat_stream_tool_call_type_is_validated_if_present() {
    let tool_stream = |delta: Value| {
        sse(vec![
            json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[delta]},"finish_reason":null}]}),
            json!({"id":"chatcmpl-test","model":"test-model","created":1702657020,"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ])
    };
    let fetch = MockFetch::new(vec![
        tool_stream(
            json!({"index":0,"id":"call_1","function":{"name":"test-tool","arguments":"{}"}}),
        ),
        tool_stream(
            json!({"index":0,"id":"call_2","type":"custom","function":{"name":"test-tool","arguments":"{}"}}),
        ),
    ]);
    let model = provider(&fetch).chat("test-model");

    let untyped = model
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .collect::<Vec<_>>()
        .await;
    assert!(
        untyped.iter().any(
            |part| matches!(part, Ok(StreamPart::ToolCall(call)) if call.tool_call_id == "call_1")
        ),
        "{untyped:?}"
    );
    assert!(untyped.iter().all(Result::is_ok), "{untyped:?}");

    let custom = model
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .collect::<Vec<_>>()
        .await;
    assert!(
        custom.iter().any(|part| matches!(part, Err(error) if error.to_string().contains("Expected 'function' type."))),
        "{custom:?}"
    );
    assert!(
        !custom
            .iter()
            .any(|part| matches!(part, Ok(StreamPart::ToolCall(_))))
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/openai-stream-error.ts:18-37,
/// :235-267 and :303-315: an error frame before any output fails the call
/// with the status `createOpenAIProviderStreamError` gives it — an HTTP
/// status in `code`, else one inferred from `code`/`type` — and
/// `insufficient_quota` is never retryable.
#[tokio::test]
async fn chat_stream_error_frame_status_is_inferred_as_upstream() {
    let cases = [
        (
            json!({"message":"quota","type":"insufficient_quota","code":"insufficient_quota","param":null}),
            429,
            false,
        ),
        (
            json!({"message":"slow down","type":"requests","code":"rate_limit_exceeded"}),
            429,
            true,
        ),
        (
            json!({"message":"busy","type":"server_error","code":"503"}),
            503,
            true,
        ),
        (
            json!({"message":"bad","type":"invalid_request_error","code":null}),
            400,
            false,
        ),
        (
            json!({"message":"oops","type":"server_error","code":null}),
            500,
            true,
        ),
    ];
    let fetch = MockFetch::new(
        cases
            .iter()
            .map(|(error, ..)| sse(vec![json!({ "error": error })]))
            .collect(),
    );
    let model = provider(&fetch).chat("test-model");
    for (frame, status, retryable) in cases {
        let Err(AiMuxError::ApiCall(error)) = model.do_stream(&options()).await.map(|_| ()) else {
            panic!("expected an API error for {frame}");
        };
        assert_eq!(error.message, frame["message"], "{frame}");
        assert_eq!(error.status_code, Some(status), "{frame}");
        assert_eq!(error.is_retryable, retryable, "{frame}");
    }
}

/// Upstream: @ai-sdk/openai 4.0.83 src/openai-stream-error.ts:207-217 types a
/// `response.failed` frame as `response.failed`, so its status comes from
/// the inner error's `code` alone, never from an inner `type`.
#[tokio::test]
async fn responses_failed_frame_status_ignores_the_inner_type() {
    let fetch = MockFetch::new(vec![sse(vec![json!({
        "type":"response.failed","sequence_number":1,
        "response":{"error":{"code":"server_error","message":"boom","type":"invalid_request_error"}}
    })])]);
    let Err(AiMuxError::ApiCall(error)) = provider(&fetch)
        .responses("test-model")
        .do_stream(&options())
        .await
        .map(|_| ())
    else {
        panic!("expected an API error");
    };
    assert_eq!(error.message, "boom");
    assert_eq!(error.status_code, Some(500));
    assert!(error.is_retryable);
}

/// Upstream: @ai-sdk/openai 4.0.83 src/chat/map-openai-finish-reason.ts:13-15
/// maps the legacy `function_call` finish reason to `tool-calls`; the raw
/// value stays `function_call`.
#[tokio::test]
async fn chat_function_call_finish_reason_is_tool_calls() {
    let mut response = chat_response();
    response["choices"][0]["finish_reason"] = json!("function_call");
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("function_call"));
}

/// Upstream: @ai-sdk/openai 4.0.83 src/chat/convert-openai-chat-usage.ts:28
/// reads cache reads only from the nested `prompt_tokens_details.cached_tokens`;
/// a top-level `cached_tokens` (Moonshot's wire format) is not a cache read and
/// only survives in `usage.raw`.
#[tokio::test]
async fn chat_cache_read_is_only_the_nested_cached_tokens() {
    let usage = |usage: Value| {
        let mut response = chat_response();
        response["usage"] = usage;
        response
    };
    let fetch = MockFetch::new(vec![
        Canned::json(&usage(
            json!({"prompt_tokens":100,"completion_tokens":10,"cached_tokens":7,"prompt_tokens_details":{"cached_tokens":25}}),
        )),
        Canned::json(&usage(
            json!({"prompt_tokens":100,"completion_tokens":10,"cached_tokens":7}),
        )),
    ]);
    let model = provider(&fetch).chat("test-model");

    let nested = model.do_generate(&options()).await.unwrap().usage;
    assert_eq!(nested.input_tokens.cache_read, Some(25));
    assert_eq!(nested.input_tokens.no_cache, Some(75));
    assert_eq!(nested.raw.unwrap()["cached_tokens"], json!(7));

    let top_level_only = model.do_generate(&options()).await.unwrap().usage;
    assert_eq!(top_level_only.input_tokens.cache_read, Some(0));
    assert_eq!(top_level_only.input_tokens.no_cache, Some(100));
    assert_eq!(top_level_only.raw.unwrap()["cached_tokens"], json!(7));
}

/// openai-chat-api.ts:71-87 (response) and :159-175 (chunk) parse a logprobs
/// entry as `{token, logprob, top_logprobs: [{token, logprob}]}`, so `bytes`
/// never reaches `providerMetadata.openai.logprobs`
/// (openai-chat-language-model.ts:459-461 generate, :613-615 stream), and a
/// `null` `content` sets none.
#[tokio::test]
async fn chat_logprobs_metadata_has_no_bytes() {
    let wire = json!({"content":[{"token":"Hello","logprob":-0.5,"bytes":[72,101,108,108,111],
        "top_logprobs":[{"token":"Hello","logprob":-0.5,"bytes":[72,101,108,108,111]},
            {"token":"Hi","logprob":-1.5,"bytes":[72,105]}]}],"refusal":null});
    let expected = json!([{"token":"Hello","logprob":-0.5,
        "top_logprobs":[{"token":"Hello","logprob":-0.5},{"token":"Hi","logprob":-1.5}]}]);
    let generated_metadata = |logprobs: Value| async move {
        let mut response = chat_response();
        response["choices"][0]["logprobs"] = logprobs;
        let fetch = MockFetch::new(vec![Canned::json(&response)]);
        let result = provider(&fetch)
            .chat("test-model")
            .do_generate(&options())
            .await
            .unwrap();
        serde_json::to_value(result.provider_metadata).unwrap()
    };
    assert_eq!(
        generated_metadata(wire.clone()).await["openai"]["logprobs"],
        expected
    );
    let metadata = generated_metadata(json!({"content": null})).await;
    assert_eq!(metadata["openai"].get("logprobs"), None, "{metadata}");

    let fetch = MockFetch::new(vec![sse(vec![json!({
        "id":"chatcmpl-test","model":"test-model","created":1702657020,
        "choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},
            "logprobs":wire,"finish_reason":"stop"}]})])]);
    let parts = provider(&fetch)
        .chat("test-model")
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .map(|part| part.unwrap())
        .collect::<Vec<_>>()
        .await;
    let Some(StreamPart::Finish {
        provider_metadata, ..
    }) = parts.last()
    else {
        panic!("no finish part: {parts:?}");
    };
    assert_eq!(
        serde_json::to_value(provider_metadata).unwrap()["openai"]["logprobs"],
        expected
    );
}

/// TS: "should parse OpenRouter resource exhausted error" (src/openai-error.test.ts)
#[tokio::test]
async fn chat_error_response() {
    let message = "{\n  \"error\": {\n    \"code\": 429,\n    \"message\": \"Resource has been exhausted (e.g. check quota).\",\n    \"status\": \"RESOURCE_EXHAUSTED\"\n  }\n}\n";
    let response = json!({"error":{"message":message,"code":429}});
    let mut canned = Canned::json(&response);
    canned.status = 429;
    let fetch = MockFetch::new(vec![canned]);
    let error = provider(&fetch)
        .chat("test-model")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert_request(
        &fetch,
        "/chat/completions",
        json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}]}),
    );
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected API error")
    };
    assert_eq!(error.message, message);
    assert_eq!(error.status_code, Some(429));
    assert_eq!(error.data, Some(response));
}

/// TS: "should generate text" (src/responses/openai-responses-language-model.test.ts)
/// TS: "should extract usage" (src/responses/openai-responses-language-model.test.ts)
#[tokio::test]
async fn responses_generate() {
    let response = responses_response();
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/responses",
        json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}]}),
    );
    assert_eq!(
        result.content,
        vec![GenerateContent::Text {
            text: "answer text".into(),
            provider_metadata: Some(
                provider_namespace("openai", json!({"itemId":"msg_test"}))
                    .expect("provider metadata must be an object")
            )
        }]
    );
    assert_eq!(result.usage.input_tokens.total, Some(345));
    assert_eq!(result.usage.input_tokens.cache_read, Some(234));
    assert_eq!(result.usage.input_tokens.cache_write, Some(45));
    assert_eq!(result.usage.input_tokens.no_cache, Some(66));
    assert_eq!(result.usage.output_tokens.total, Some(538));
    assert_eq!(result.usage.output_tokens.reasoning, Some(123));
    assert_eq!(result.usage.output_tokens.text, Some(415));
    assert_eq!(result.usage.raw, response["usage"].as_object().cloned());
    assert_eq!(result.response.unwrap().id.as_deref(), Some("resp_test"));
}

/// TS: "should stream text deltas" (src/responses/openai-responses-language-model.test.ts)
#[tokio::test]
async fn responses_stream() {
    let events=json!([{"type": "response.created", "response": {"id": "resp_67c9a81b6a048190a9ee441c5755a4e8", "object": "response", "created_at": 1741269019, "status": "in_progress", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": null, "user": null, "metadata": {}}}, {"type": "response.in_progress", "response": {"id": "resp_67c9a81b6a048190a9ee441c5755a4e8", "object": "response", "created_at": 1741269019, "status": "in_progress", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": null, "user": null, "metadata": {}}}, {"type": "response.output_item.added", "output_index": 0, "item": {"id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "type": "message", "status": "in_progress", "role": "assistant", "content": []}}, {"type": "response.content_part.added", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": [], "logprobs": []}}, {"type": "response.output_text.delta", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "delta": "Hello,", "logprobs": []}, {"type": "response.output_text.delta", "item_id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "output_index": 0, "content_index": 0, "delta": " World!", "logprobs": []}, {"type": "response.output_text.done", "item_id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "output_index": 0, "content_index": 0, "text": "Hello, World!"}, {"type": "response.content_part.done", "item_id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": "Hello, World!", "annotations": [], "logprobs": []}}, {"type": "response.output_item.done", "output_index": 0, "item": {"id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Hello, World!", "annotations": [], "logprobs": []}]}}, {"type": "response.completed", "response": {"id": "resp_67c9a878139c8190aa2e3105411b408b", "object": "response", "created_at": 1741269112, "status": "completed", "error": null, "incomplete_details": null, "input": [], "instructions": null, "max_output_tokens": null, "model": "test-model", "output": [{"id": "msg_67c9a8787f4c8190b49c858d4c1cf20c", "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": "Hello, World!", "annotations": []}]}], "parallel_tool_calls": true, "previous_response_id": null, "reasoning": {"effort": null, "summary": null}, "store": true, "temperature": 0.3, "text": {"format": {"type": "text"}}, "tool_choice": "auto", "tools": [], "top_p": 1, "truncation": "disabled", "usage": {"input_tokens": 543, "input_tokens_details": {"cached_tokens": 234}, "output_tokens": 478, "output_tokens_details": {"reasoning_tokens": 123}, "total_tokens": 512}, "user": null, "metadata": {}}}]).as_array().unwrap().clone();
    let fetch = MockFetch::new(vec![sse(events)]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_stream(&options())
        .await
        .unwrap();
    let parts = result
        .stream
        .map(|part| part.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_request(
        &fetch,
        "/responses",
        json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}],"stream":true}),
    );
    assert_eq!(
        serde_json::to_value(parts).unwrap(),
        json!([
            {"type": "stream-start", "warnings": []},
            {
                "type": "response-metadata",
                "id": "resp_67c9a81b6a048190a9ee441c5755a4e8",
                "modelId": "test-model",
                "timestamp": "2025-03-06T13:50:19.000Z"
            },
            {
                "type": "text-start",
                "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                "providerMetadata": {"openai": {"itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e"}}
            },
            {"type": "text-delta", "id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "delta": "Hello,"},
            {"type": "text-delta", "id": "msg_67c9a81dea8c8190b79651a2b3adf91e", "delta": " World!"},
            {
                "type": "text-end",
                "id": "msg_67c9a81dea8c8190b79651a2b3adf91e",
                "providerMetadata": {"openai": {"itemId": "msg_67c9a81dea8c8190b79651a2b3adf91e"}}
            },
            {
                "type": "finish",
                "finishReason": {"unified": "stop"},
                "providerMetadata": {"openai": {"responseId": "resp_67c9a81b6a048190a9ee441c5755a4e8"}},
                "usage": {
                    "inputTokens": {"total": 543, "noCache": 309, "cacheRead": 234},
                    "outputTokens": {"total": 478, "reasoning": 123, "text": 355},
                    "raw": {
                        "input_tokens": 543,
                        "input_tokens_details": {"cached_tokens": 234},
                        "output_tokens": 478,
                        "output_tokens_details": {"reasoning_tokens": 123},
                        "total_tokens": 512
                    }
                }
            }
        ])
    );
}

/// TS: "should throw a descriptive error when the response has no output" (src/responses/openai-responses-language-model.test.ts)
#[tokio::test]
async fn responses_error_response() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"id":"resp_no_output","status":"incomplete","incomplete_details":{"reason":"content_filter"}}),
    )]);
    let error = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert_request(
        &fetch,
        "/responses",
        json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}]}),
    );
    assert!(
        error
            .to_string()
            .contains("Responses API returned no output (content_filter)"),
        "{error}"
    );
}

fn embedding_response() -> Value {
    serde_json::from_str(
        r#"{
  "object": "list",
  "data": [
    {
      "object": "embedding",
      "index": 0,
      "embedding": [
        0.0057293195, -0.012727811, 0.020042092, -0.013437585, 0.022833068
      ]
    },
    {
      "object": "embedding",
      "index": 1,
      "embedding": [
        -0.037104916, -0.05178114, -0.008340587, 0.001164541, -0.0035253682
      ]
    }
  ],
  "model": "test-embedding",
  "usage": {
    "prompt_tokens": 12,
    "total_tokens": 12
  }
}
"#,
    )
    .unwrap()
}

fn image_response() -> Value {
    serde_json::from_str(r#"{
  "created": 1770935200,
  "data": [
    {
      "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj",
      "revised_prompt": "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed."
    },
    {
      "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
    }
  ]
}
"#).unwrap()
}

fn transcription_response() -> Value {
    serde_json::from_str(r#"{
  "task": "transcribe",
  "language": "english",
  "duration": 36.709999084472656,
  "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies.",
  "words": [
    {
      "word": "Galileo",
      "start": 0,
      "end": 0.6600000262260437
    },
    {
      "word": "was",
      "start": 0.6600000262260437,
      "end": 0.8999999761581421
    },
    {
      "word": "an",
      "start": 0.8999999761581421,
      "end": 1.1399999856948853
    },
    {
      "word": "American",
      "start": 1.1399999856948853,
      "end": 1.5
    },
    {
      "word": "robotic",
      "start": 1.5,
      "end": 2.0199999809265137
    }
  ],
  "usage": {
    "type": "duration",
    "seconds": 37
  }
}
"#).unwrap()
}

/// TS: "should extract embedding" (src/embedding/openai-embedding-model.test.ts)
/// TS: "should pass the model and the values" (src/embedding/openai-embedding-model.test.ts)
#[tokio::test]
async fn embedding_generate() {
    let response = embedding_response();
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
    opts.values.push("rainy day in the city".into());
    let result = provider(&fetch)
        .embedding("test-embedding")
        .do_embed(&opts)
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/embeddings",
        json!({"model":"test-embedding","input":opts.values,"encoding_format":"float"}),
    );
    assert_eq!(
        result.embeddings,
        vec![
            vec![
                0.0057293195,
                -0.012727811,
                0.020042092,
                -0.013437585,
                0.022833068
            ],
            vec![
                -0.037104916,
                -0.05178114,
                -0.008340587,
                0.001164541,
                -0.0035253682
            ]
        ]
    );
    assert_eq!(result.usage.unwrap().tokens, 12);
    assert_eq!(result.response.unwrap().body, Some(response));
}

/// TS: "should pass the model and the settings" (src/image/openai-image-model.test.ts)
/// TS: "should extract the generated images" (src/image/openai-image-model.test.ts)
#[tokio::test]
async fn image_generate() {
    let response = image_response();
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let mut opts = ImageCallOptions::new("A cute baby sea otter");
    opts.size = Some(Size::new(1024, 1024));
    opts.provider_options = provider_namespace("openai", json!({"style":"vivid"}))
        .expect("provider metadata must be an object");
    let result = provider(&fetch)
        .image("test-image")
        .do_generate(&opts)
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/images/generations",
        json!({"model":"test-image","prompt":"A cute baby sea otter","n":1,"size":"1024x1024","style":"vivid","response_format":"b64_json"}),
    );
    let expected = response["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|image| image["b64_json"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(matches!(result.images,ImageOutputs::Base64(images) if images==expected));
}

/// openai-image-model.ts:166 (files) and :357 (mask) fetch a URL file with
/// `downloadBlob(file.url, {abortSignal})`, which goes through
/// `fetchUntrustedUrl` (provider-utils download-blob.ts:111-114): a
/// non-public target is refused before any request, and the edit request is
/// never sent.
#[tokio::test]
async fn image_edit_url_files_are_downloaded_through_the_guard() {
    let url = || ImageFile::Url {
        url: "http://169.254.169.254/latest/meta-data/image.png".into(),
    };
    let mut files = ImageCallOptions::new("Edit");
    files.files = Some(vec![url()]);
    let mut mask = ImageCallOptions::new("Edit");
    mask.files = Some(vec![ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Binary(vec![1, 2, 3]),
    }]);
    mask.mask = Some(url());
    for options in [files, mask] {
        let fetch = MockFetch::new(vec![Canned::json(&image_response())]);
        let error = provider(&fetch)
            .image("gpt-image-1")
            .do_generate(&options)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AiMuxError::InvalidArgument { ref message, .. } if message.contains("non-public")),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());
    }
}

/// TS: "should pass the model and text" (src/speech/openai-speech-model.test.ts)
/// TS: "should return audio data with correct content type" (src/speech/openai-speech-model.test.ts)
#[tokio::test]
async fn speech_generate() {
    let audio = vec![1, 2, 3, 4];
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "audio/mpeg".into())],
        body: audio.clone(),
    }]);
    let result = provider(&fetch)
        .speech("test-speech")
        .do_generate(&SpeechCallOptions::new("Hello from the AI SDK!"))
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/audio/speech",
        json!({"model":"test-speech","input":"Hello from the AI SDK!","voice":"alloy","response_format":"mp3"}),
    );
    assert!(matches!(result.audio,AudioData::Binary(bytes) if bytes==audio));
    assert_eq!(
        result.response.headers.unwrap()["content-type"],
        "audio/mpeg"
    );
}

/// TS: "should extract the transcription text" (src/transcription/openai-transcription-model.test.ts)
#[tokio::test]
async fn transcription_generate() {
    let response = transcription_response();
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let opts = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
    let result = provider(&fetch)
        .transcription("test-transcription")
        .do_generate(&opts)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://example.test/v1/audio/transcriptions");
    assert_eq!(seen[0].method, "POST");
    assert!(seen[0].headers["content-type"].starts_with("multipart/form-data; boundary="));
    let body = String::from_utf8_lossy(&seen[0].body);
    assert!(body.contains("name=\"model\"\r\n\r\ntest-transcription\r\n"));
    assert!(body.contains("name=\"file\"; filename=\"audio.wav\""));
    assert!(seen[0].body.windows(3).any(|window| window == [1, 2, 3]));
    assert_eq!(result.text, response["text"].as_str().unwrap());
    assert_eq!(result.language.as_deref(), Some("en"));
    assert_eq!(result.duration_in_seconds, Some(36.709999084472656));
    assert_eq!(result.segments.len(), 5);
}

/// TS: "should handle streaming web search without action" (src/responses/openai-responses-language-model.test.ts)
#[tokio::test]
async fn responses_web_search_stream() {
    let fetch = MockFetch::new(vec![sse(vec![
        json!({"type":"response.created","response":{"id":"resp_missing_action","created_at":1741630255,"model":"test-model"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws_missing_action","status":"in_progress"}}),
        json!({"type":"response.web_search_call.in_progress","output_index":0,"item_id":"ws_missing_action"}),
        json!({"type":"response.web_search_call.completed","output_index":0,"item_id":"ws_missing_action"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"web_search_call","id":"ws_missing_action","status":"completed"}}),
        json!({"type":"response.completed","response":{"id":"resp_missing_action","usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":12}}}),
    ])]);
    let mut call = options();
    call.tools = Some(vec![Tool::Provider(aimux_core::tool::ProviderTool {
        id: "openai.web_search".parse().unwrap(),
        name: "webSearch".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_stream(&call)
        .await
        .unwrap();
    let parts = result
        .stream
        .map(|part| part.unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_request(
        &fetch,
        "/responses",
        json!({"model":"test-model","input":[{"role":"user","content":[{"type":"input_text","text":"Hello"}]}],"stream":true,"tools":[{"type":"web_search"}],"include":["web_search_call.action.sources"]}),
    );
    assert_eq!(
        serde_json::to_value(parts).unwrap(),
        json!([
            {"type":"stream-start","warnings":[]},
            {"type":"response-metadata","id":"resp_missing_action","modelId":"test-model","timestamp":"2025-03-10T18:10:55.000Z"},
            {"type":"tool-input-start","id":"ws_missing_action","toolName":"webSearch","providerExecuted":true},
            {"type":"tool-input-end","id":"ws_missing_action"},
            {"type":"tool-call","toolCallId":"ws_missing_action","toolName":"webSearch","input":"{}","providerExecuted":true},
            {"type":"tool-result","toolCallId":"ws_missing_action","toolName":"webSearch","result":{}},
            {"type":"finish","finishReason":{"unified":"stop"},"usage":{"inputTokens":{"total":10,"noCache":10,"cacheRead":0},"outputTokens":{"total":2,"reasoning":0,"text":2},"raw":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":12}},"providerMetadata":{"openai":{"responseId":"resp_missing_action"}}}
        ])
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/openai-provider.ts:387 puts
/// `openaiTools` on the provider, so `openai.tools.webSearch(…)` makes the
/// provider tool; it is the module's factory and reaches the request.
#[tokio::test]
async fn provider_tools_member_makes_the_provider_tools() {
    use aimux_providers::openai::tools::{SearchContextSize, WebSearchArgs};
    let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
    let openai = provider(&fetch);
    let args = || WebSearchArgs {
        search_context_size: Some(SearchContextSize::High),
        ..Default::default()
    };
    let tool = openai.tools.web_search(args()).unwrap();
    let factory = aimux_providers::openai::tools::web_search(args()).unwrap();
    assert_eq!(
        (&tool.id, &tool.name, &tool.args),
        (&factory.id, &factory.name, &factory.args)
    );
    assert_eq!(openai.tools.apply_patch().id, "openai.apply_patch");
    // Of the OpenAI tools only programmatic tool calling supports deferred
    // results (src/tool/programmatic-tool-calling.ts:55).
    assert_eq!(
        openai
            .tools
            .programmatic_tool_calling()
            .supports_deferred_results,
        Some(true)
    );
    assert_eq!(openai.tools.apply_patch().supports_deferred_results, None);
    let mut call = options();
    call.tools = Some(vec![Tool::Provider(tool)]);
    openai
        .responses("test-model")
        .do_generate(&call)
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].json_body()["tools"],
        json!([{"type":"web_search","search_context_size":"high"}])
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-language-model.ts:1660-1663
/// enqueues `{ type: 'raw', rawValue }` for every chunk, before the parts
/// derived from it, when `includeRawChunks` is set, and nothing otherwise.
#[tokio::test]
async fn responses_stream_emits_raw_chunks_when_asked() {
    let events = vec![
        json!({"type":"response.created","response":{"id":"resp_raw","created_at":1741630255,"model":"test-model"}}),
        json!({"type":"response.completed","response":{"id":"resp_raw","usage":{"input_tokens":1,"output_tokens":1}}}),
    ];
    let fetch = MockFetch::new(vec![sse(events.clone()), sse(events.clone())]);
    let model = provider(&fetch).responses("test-model");
    let mut call = options();
    call.include_raw_chunks = Some(true);
    let with_raw: Vec<_> = model
        .do_stream(&call)
        .await
        .unwrap()
        .stream
        .map(|part| part.unwrap())
        .collect()
        .await;
    assert!(matches!(&with_raw[0], StreamPart::StreamStart { .. }));
    assert!(matches!(&with_raw[1], StreamPart::Raw { raw_value } if *raw_value == events[0]));
    assert!(matches!(&with_raw[2], StreamPart::ResponseMetadata(_)));
    assert!(matches!(&with_raw[3], StreamPart::Raw { raw_value } if *raw_value == events[1]));
    assert!(matches!(with_raw.last(), Some(StreamPart::Finish { .. })));

    let without_raw: Vec<_> = model
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .map(|part| part.unwrap())
        .collect()
        .await;
    assert!(
        !without_raw
            .iter()
            .any(|part| matches!(part, StreamPart::Raw { .. }))
    );
}

/// Upstream: @ai-sdk/openai 4.0.83
/// src/responses/openai-responses-language-model.ts:1587 starts `responseId` at
/// `null` and only `response.created` sets it, so a stream without that event
/// finishes with `responseId: null`, not an empty string.
#[tokio::test]
async fn a_stream_without_response_created_finishes_with_a_null_response_id() {
    let fetch = MockFetch::new(vec![sse(vec![json!({
        "type":"response.completed",
        "response":{"id":"resp_1","created_at":1,"model":"test-model","status":"completed","output":[],
            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}
    })])]);
    let parts: Vec<_> = provider(&fetch)
        .responses("test-model")
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .map(|part| part.unwrap())
        .collect()
        .await;
    let Some(StreamPart::Finish {
        provider_metadata: Some(metadata),
        ..
    }) = parts.last()
    else {
        panic!("expected a finish part with provider metadata: {parts:?}");
    };
    assert_eq!(metadata["openai"]["responseId"], Value::Null);
}

/// Upstream: @ai-sdk/openai 4.0.83
/// src/responses/openai-responses-language-model.ts:2735-2749 gives every
/// `reasoning-delta` the reasoning item's id as `providerMetadata.<key>.itemId`,
/// as it does for `reasoning-start` and `reasoning-end`.
#[tokio::test]
async fn reasoning_deltas_carry_the_item_id() {
    let fetch = MockFetch::new(vec![sse(vec![
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_1","type":"reasoning","summary":[]}}),
        json!({"type":"response.reasoning_summary_part.added","item_id":"rs_1","output_index":0,
            "summary_index":0,"part":{"type":"summary_text","text":""}}),
        json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,
            "summary_index":0,"delta":"Thinking"}),
    ])]);
    let parts: Vec<_> = provider(&fetch)
        .responses("test-model")
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .map(|part| part.unwrap())
        .collect()
        .await;
    let delta = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::ReasoningDelta {
                provider_metadata, ..
            } => Some(provider_metadata),
            _ => None,
        })
        .expect("a reasoning delta");
    assert_eq!(
        serde_json::to_value(delta).unwrap(),
        json!({ "openai": { "itemId": "rs_1" } })
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/tool/file-search.ts:29 takes
/// `maxNumResults` as a JS number and
/// src/responses/openai-responses-prepare-tools.ts:219 sends it as is, so
/// `maxNumResults: 5` reaches the API's integer field as `5`, not `5.0`.
#[tokio::test]
async fn file_search_sends_max_num_results_as_an_integer() {
    let fetch = MockFetch::new(vec![Canned::json(&responses_response())]);
    let args: FileSearchArgs =
        serde_json::from_value(json!({ "vectorStoreIds": ["vs_1"], "maxNumResults": 5 })).unwrap();
    let mut options = options();
    options.tools = Some(vec![Tool::Provider(file_search(args).unwrap())]);
    provider(&fetch)
        .responses("test-model")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].json_body()["tools"][0]["max_num_results"],
        json!(5)
    );
}

/// Upstream: @ai-sdk/openai 4.0.83
/// src/responses/openai-responses-language-model.ts:128 (`mapComputerAction`)
/// builds each computer action from the fields its type has and adds `keys`
/// only when it is not null, so a `double_click` with `keys: null` reaches the
/// tool input without `keys`.
#[tokio::test]
async fn computer_actions_leave_out_null_keys() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_computer",
        "created_at": 1,
        "model": "test-model",
        "output": [{
            "type": "computer_call",
            "id": "computer_item",
            "call_id": "computer_call",
            "status": "completed",
            "actions": [
                {"type": "double_click", "x": 1, "y": 2, "keys": null},
                {"type": "scroll", "x": 3, "y": 4, "scroll_x": 0, "scroll_y": 5, "keys": null}
            ],
            "pending_safety_checks": []
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
        panic!("expected a tool call: {:?}", result.content);
    };
    assert_eq!(
        call.input,
        r#"{"actions":[{"type":"double_click","x":1,"y":2},{"type":"scroll","x":3,"y":4,"scrollX":0,"scrollY":5}],"pendingSafetyChecks":[],"status":"completed"}"#
    );
}

/// Upstream: @ai-sdk/openai 4.0.83
/// src/responses/openai-responses-language-model.ts:378 maps the names of
/// twelve provider tools back to the caller's tool names
/// (`providerToolNames`); `computer_use` is not one of them, so a computer
/// call without a `call_id` (line 1358) keeps the name `computer_use` even
/// when the request declares an `openai.computer_use` tool with another name.
#[tokio::test]
async fn computer_use_results_keep_the_canonical_tool_name() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_computer_use",
        "created_at": 1,
        "model": "test-model",
        "output": [{"type": "computer_call", "id": "computer_item", "status": "completed"}],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let mut options = options();
    options.tools = Some(vec![Tool::Provider(aimux_core::tool::ProviderTool {
        id: "openai.computer_use".parse().unwrap(),
        name: "computerUse".into(),
        args: serde_json::Map::new(),
        supports_deferred_results: None,
    })]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options)
        .await
        .unwrap();
    let names: Vec<&str> = result
        .content
        .iter()
        .filter_map(|part| match part {
            GenerateContent::ToolCall(call) => Some(call.tool_name.as_str()),
            GenerateContent::ToolResult(result) => Some(result.tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["computer_use", "computer_use"]);
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1520
/// parses a `container_file_citation` annotation with only `type`,
/// `container_id`, `file_id`, `filename`, `start_index` and `end_index`, and
/// src/responses/openai-responses-language-model.ts:1072 puts the parsed
/// annotations in the text part's provider metadata, so an extra `index` is
/// dropped.
#[tokio::test]
async fn text_annotations_keep_only_the_fields_of_their_type() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_annotations",
        "created_at": 1,
        "model": "test-model",
        "output": [{
            "type": "message",
            "id": "msg_container",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": "Generated with container file.",
                "annotations": [{
                    "type": "container_file_citation",
                    "container_id": "cntr_test",
                    "file_id": "file-container",
                    "filename": "data.csv",
                    "start_index": 0,
                    "end_index": 10,
                    "index": 2
                }]
            }]
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    let Some(GenerateContent::Text {
        provider_metadata: Some(metadata),
        ..
    }) = result.content.first()
    else {
        panic!("expected a text part: {:?}", result.content);
    };
    assert_eq!(
        metadata["openai"]["annotations"],
        json!([{
            "type": "container_file_citation",
            "container_id": "cntr_test",
            "file_id": "file-container",
            "filename": "data.csv",
            "start_index": 0,
            "end_index": 10
        }])
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1698
/// parses an apply patch operation as `{type, path, diff}`, and
/// src/responses/openai-responses-language-model.ts:1443 stringifies that
/// parsed operation into the tool input, so its keys follow the schema order
/// whatever order the wire sent them in.
#[tokio::test]
async fn apply_patch_input_lists_operation_fields_in_schema_order() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_apply_patch",
        "created_at": 1,
        "model": "test-model",
        "output": [{
            "type": "apply_patch_call",
            "id": "apc_1",
            "call_id": "call_1",
            "status": "completed",
            "operation": {"diff": "+hello\n", "type": "create_file", "path": "a.md"}
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
        panic!("expected a tool call: {:?}", result.content);
    };
    assert_eq!(
        call.input,
        r#"{"callId":"call_1","operation":{"type":"create_file","path":"a.md","diff":"+hello\n"}}"#
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:147
/// parses a local shell action as `{type, command, timeout_ms, user,
/// working_directory, env}`, and
/// src/responses/openai-responses-language-model.ts:998 stringifies the
/// parsed action into the tool input, so its keys follow that order.
#[tokio::test]
async fn local_shell_input_lists_action_fields_in_schema_order() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_local_shell",
        "created_at": 1,
        "model": "test-model",
        "output": [{
            "type": "local_shell_call",
            "id": "lsh_1",
            "call_id": "call_1",
            "status": "completed",
            "action": {"type": "exec", "command": ["ls"], "env": {}, "working_directory": "/root"}
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let result = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap();
    let Some(GenerateContent::ToolCall(call)) = result.content.first() else {
        panic!("expected a tool call: {:?}", result.content);
    };
    assert_eq!(
        call.input,
        r#"{"action":{"type":"exec","command":["ls"],"working_directory":"/root","env":{}}}"#
    );
}

/// Upstream: @ai-sdk/openai 4.0.83 src/responses/openai-responses-api.ts:1486
/// lets a message's `content` hold only `output_text` parts, so a `refusal`
/// part fails the response schema and the JSON response handler rejects the
/// call with an `APICallError` ("Invalid JSON response").
#[tokio::test]
async fn a_refusal_message_part_rejects_the_response() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "resp_refusal",
        "created_at": 1,
        "model": "test-model",
        "output": [{
            "type": "message",
            "id": "msg_refusal",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "refusal", "refusal": "Cannot help with this."}]
        }],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let error = provider(&fetch)
        .responses("test-model")
        .do_generate(&options())
        .await
        .unwrap_err();
    let AiMuxError::ApiCall(detail) = &error else {
        panic!("expected an API call error: {error:?}");
    };
    assert!(
        detail.message.starts_with("Invalid JSON response"),
        "{error}"
    );
    assert!(
        detail
            .response_body
            .as_deref()
            .is_some_and(|body| body.contains("Cannot help with this.")),
        "{error:?}"
    );
}
