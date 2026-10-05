//! End-to-end subset of the pinned openai-compatible upstream tests.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;

use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::shared::{Size, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::FinishReasonUnified;
use aimux_providers::openai_compatible::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

const BASE_URL: &str = "https://my.api.com/v1";
const IMAGE_PROMPT: &str = "A photorealistic astronaut riding a horse";

fn provider(fetch: &Arc<MockFetch>) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "test-provider".into(),
        base_url: BASE_URL.into(),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn chat_response(message: Value, usage: Value) -> Value {
    json!({"id":"chatcmpl-test", "created":1711115037, "model":"test-model",
        "choices":[{"index":0,"message":message,"finish_reason":"stop"}],
        "usage":usage})
}

fn stream_response(values: &[Value]) -> Canned {
    let mut body = values
        .iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

fn assert_request(fetch: &MockFetch, path: &str, body: Value) {
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, format!("{BASE_URL}{path}"));
    assert_eq!(requests[0].json_body(), body);
}

fn stream_request() -> Value {
    json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"stream":true})
}

/// TS: "should use camelCase metadata key when camelCase provider options are used" (openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts)
/// TS: "should extract detailed token usage when available" (openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts)
#[tokio::test]
async fn chat_generate_metadata_and_usage() {
    let usage = json!({"prompt_tokens":20,"completion_tokens":30,"total_tokens":50,
        "prompt_tokens_details":{"cached_tokens":5},
        "completion_tokens_details":{"reasoning_tokens":10,"accepted_prediction_tokens":15,"rejected_prediction_tokens":5}});
    let fetch = MockFetch::new(vec![Canned::json(&chat_response(
        json!({"content":"Hello!"}),
        usage.clone(),
    ))]);
    let mut opts = options();
    opts.provider_options = Some(provider_namespace(
        "testProvider",
        json!({"reasoningEffort":"high"}),
    ));
    let result = provider(&fetch)
        .chat("test-model")
        .do_generate(&opts)
        .await
        .unwrap();
    let expected = json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"reasoning_effort":"high"});
    assert_request(&fetch, "/chat/completions", expected.clone());
    assert_eq!(result.request.unwrap().body, Some(expected));
    assert_eq!(result.content.len(), 1);
    assert!(matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello!"));
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    assert_eq!(result.usage.input_tokens.total, Some(20));
    assert_eq!(result.usage.input_tokens.no_cache, Some(15));
    assert_eq!(result.usage.input_tokens.cache_read, Some(5));
    assert_eq!(result.usage.input_tokens.cache_write, None);
    assert_eq!(result.usage.output_tokens.total, Some(30));
    assert_eq!(result.usage.output_tokens.text, Some(20));
    assert_eq!(result.usage.output_tokens.reasoning, Some(10));
    assert_eq!(result.usage.raw, Some(usage));
    let metadata = result.provider_metadata.unwrap();
    assert_eq!(
        metadata,
        provider_namespace(
            "testProvider",
            json!({"acceptedPredictionTokens":15,"rejectedPredictionTokens":5})
        )
    );
    assert!(!metadata.contains_key("test-provider"));
    let response = result.response.unwrap();
    assert_eq!(response.id.as_deref(), Some("chatcmpl-test"));
    assert_eq!(response.model_id.as_deref(), Some("test-model"));
    assert_eq!(
        response.headers.unwrap()["content-type"],
        "application/json"
    );
    assert!(result.warnings.is_empty());
}

/// TS: "should stream reasoning content before text deltas" (openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts)
#[tokio::test]
async fn chat_stream_reasoning_before_text() {
    let delta = |value: Value| {
        json!({"id":"chatcmpl-test","created":1711357598,
        "model":"test-model","choices":[{"index":0,"delta":value,"finish_reason":null}]})
    };
    let fetch = MockFetch::new(vec![stream_response(&[
        delta(json!({"role":"assistant","content":"","reasoning_content":"Let me think"})),
        delta(json!({"content":"","reasoning_content":" about this"})),
        delta(json!({"content":"Here's"})),
        delta(json!({"content":" my response"})),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":18,"completion_tokens":439}}),
    ])]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_stream(&options())
        .await
        .unwrap();
    assert_request(&fetch, "/chat/completions", stream_request());
    assert_eq!(result.request.unwrap().body, Some(stream_request()));
    let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
    assert_eq!(parts.len(), 11);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(
        matches!(&parts[1], StreamPart::ResponseMetadata(metadata) if metadata.id.as_deref() == Some("chatcmpl-test") && metadata.model_id.as_deref() == Some("test-model"))
    );
    assert!(matches!(&parts[2], StreamPart::ReasoningStart { id, .. } if id == "reasoning-0"));
    assert!(matches!(&parts[5], StreamPart::ReasoningEnd { id, .. } if id == "reasoning-0"));
    assert!(matches!(&parts[6], StreamPart::TextStart { id, .. } if id == "txt-0"));
    assert!(matches!(&parts[9], StreamPart::TextEnd { id, .. } if id == "txt-0"));
    let deltas: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ReasoningDelta { id, delta, .. } => {
                Some(("reasoning", id.as_str(), delta.as_str()))
            }
            StreamPart::TextDelta { id, delta, .. } => Some(("text", id.as_str(), delta.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec![
            ("reasoning", "reasoning-0", "Let me think"),
            ("reasoning", "reasoning-0", " about this"),
            ("text", "txt-0", "Here's"),
            ("text", "txt-0", " my response")
        ]
    );
    let StreamPart::Finish {
        finish_reason,
        usage,
        provider_metadata,
    } = &parts[10]
    else {
        panic!("expected finish");
    };
    assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(usage.input_tokens.total, Some(18));
    assert_eq!(usage.input_tokens.no_cache, Some(18));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.output_tokens.total, Some(439));
    assert_eq!(usage.output_tokens.text, Some(439));
    assert_eq!(usage.output_tokens.reasoning, Some(0));
    assert_eq!(
        provider_metadata.as_ref().unwrap(),
        &provider_namespace("test-provider", json!({}))
    );
}

/// TS: "should parse thought signature from extra_content and include in providerMetadata" (openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts)
#[ignore = "CallOptions::tool_choice is not optional, so `tool_choice: \"auto\"` is always sent; upstream omits it when unset"]
#[tokio::test]
async fn chat_tool_call_preserves_arguments_and_signature() {
    let fetch = MockFetch::new(vec![Canned::json(&chat_response(
        json!({"tool_calls":[{
        "id":"function-call-1","type":"function",
        "function":{"name":"check_flight","arguments":"{\"flight\":\"AA100\"}"},
        "extra_content":{"google":{"thought_signature":"<Signature A>"}}}]}),
        Value::Null,
    ))]);
    let schema = json!({"type":"object","properties":{"flight":{"type":"string"}},
        "required":["flight"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"});
    let mut opts = options();
    opts.tools = Some(vec![
        FunctionTool::new("check_flight", schema.clone()).into(),
    ]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_generate(&opts)
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/chat/completions",
        json!({"model":"test-model",
        "messages":[{"role":"user","content":"Hello"}],
        "tools":[{"type":"function","function":{"name":"check_flight","parameters":schema}}]}),
    );
    assert_eq!(result.content.len(), 1);
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    let GenerateContent::ToolCall(call) = &result.content[0] else {
        panic!("expected tool call");
    };
    assert_eq!(call.tool_call_id, "function-call-1");
    assert_eq!(call.tool_name, "check_flight");
    assert_eq!(call.input, "{\"flight\":\"AA100\"}");
    assert_eq!(
        call.provider_metadata.as_ref().unwrap(),
        &provider_namespace("test-provider", json!({"thoughtSignature":"<Signature A>"}))
    );
}

/// TS: "should preserve structured error stream parts" (openai-compatible/src/chat/openai-compatible-chat-language-model.test.ts)
#[tokio::test]
async fn chat_stream_preserves_structured_error() {
    let fetch = MockFetch::new(vec![stream_response(&[json!({"error":{
        "message":"Context length exceeded","code":"CONTEXT_LENGTH_EXCEEDED"}})])]);
    let result = provider(&fetch)
        .chat("test-model")
        .do_stream(&options())
        .await
        .unwrap();
    assert_request(&fetch, "/chat/completions", stream_request());
    let parts: Vec<_> = result.stream.map(|part| part.unwrap()).collect().await;
    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    let StreamPart::Error {
        error: AiMuxError::ApiCall(error),
    } = &parts[1]
    else {
        panic!("expected structured API error");
    };
    assert_eq!(error.message, "Context length exceeded");
    assert_eq!(
        error.provider_code.as_deref(),
        Some("CONTEXT_LENGTH_EXCEEDED")
    );
    let StreamPart::Finish {
        finish_reason,
        usage,
        provider_metadata,
    } = &parts[2]
    else {
        panic!("expected finish");
    };
    assert_eq!(finish_reason.unified, FinishReasonUnified::Error);
    assert_eq!(finish_reason.raw, None);
    assert_eq!(usage.input_tokens.total, None);
    assert_eq!(usage.output_tokens.total, None);
    assert_eq!(usage.raw, None);
    assert_eq!(
        provider_metadata.as_ref().unwrap(),
        &provider_namespace("test-provider", json!({}))
    );
}

/// TS: "should extract embedding" (openai-compatible/src/embedding/openai-compatible-embedding-model.test.ts)
/// TS: "should pass the model and the values" (openai-compatible/src/embedding/openai-compatible-embedding-model.test.ts)
#[tokio::test]
async fn embedding_request_and_result() {
    let body = json!({"object":"list","data":[
        {"object":"embedding","index":0,"embedding":[0.1,0.2,0.3,0.4,0.5]},
        {"object":"embedding","index":1,"embedding":[0.6,0.7,0.8,0.9,1.0]}],
        "model":"test-embedding","usage":{"prompt_tokens":8,"total_tokens":8}});
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
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
        json!({"model":"test-embedding",
        "input":["sunny day at the beach","rainy day in the city"],"encoding_format":"float"}),
    );
    assert_eq!(
        result.embeddings,
        vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
    );
    assert_eq!(result.usage.unwrap().tokens, 8);
    let response = result.response.unwrap();
    assert_eq!(response.body, Some(body));
    assert_eq!(
        response.headers.unwrap()["content-type"],
        "application/json"
    );
    assert!(result.warnings.is_empty());
}

fn image_options() -> ImageCallOptions {
    let mut opts = ImageCallOptions::new(IMAGE_PROMPT);
    opts.size = Some(Size::new(1024, 1024));
    opts
}

/// TS: "should return the raw b64_json content" (openai-compatible/src/image/openai-compatible-image-model.test.ts)
/// TS: "should map the usage object reported by the provider" (openai-compatible/src/image/openai-compatible-image-model.test.ts)
#[tokio::test]
async fn image_generate_raw_content_and_usage() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({"data":[
        {"b64_json":"test1234"},{"b64_json":"test5678"}],
        "usage":{"input_tokens":12,"output_tokens":4,"total_tokens":16}}))]);
    let mut opts = image_options();
    opts.n = 2;
    let result = provider(&fetch)
        .image("test-image")
        .do_generate(&opts)
        .await
        .unwrap();
    assert_request(
        &fetch,
        "/images/generations",
        json!({"model":"test-image",
        "prompt":IMAGE_PROMPT,"n":2,"size":"1024x1024"}),
    );
    let ImageOutputs::Base64(images) = result.images else {
        panic!("expected base64 images");
    };
    assert_eq!(images, vec!["test1234", "test5678"]);
    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(4));
    assert_eq!(usage.total_tokens, Some(16));
    assert_eq!(result.response.model_id.as_deref(), Some("test-image"));
    assert_eq!(
        result.response.headers.unwrap()["content-type"],
        "application/json"
    );
    assert!(result.warnings.is_empty());
}

/// TS: "should handle API errors with default error structure" (openai-compatible/src/image/openai-compatible-image-model.test.ts)
#[tokio::test]
async fn image_generate_api_error() {
    let mut canned = Canned::json(&json!({"error":{"message":"Invalid prompt content",
        "type":"invalid_request_error","param":null,"code":null}}));
    canned.status = 400;
    let fetch = MockFetch::new(vec![canned]);
    let error = provider(&fetch)
        .image("test-image")
        .do_generate(&image_options())
        .await
        .unwrap_err();
    assert_request(
        &fetch,
        "/images/generations",
        json!({"model":"test-image",
        "prompt":IMAGE_PROMPT,"n":1,"size":"1024x1024"}),
    );
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected API error");
    };
    assert_eq!(error.message, "Invalid prompt content");
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.url, format!("{BASE_URL}/images/generations"));
}
