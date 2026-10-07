//! Small end-to-end subset of the pinned Mistral upstream tests.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::FinishReasonUnified;
use aimux_providers::mistral::{MistralProvider, MistralProviderSettings, create_mistral};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> MistralProvider {
    create_mistral(MistralProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn expected_request(stream: bool) -> Value {
    let mut body = json!({
        "model": "mistral-small-latest",
        "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}]
    });
    if stream {
        body["stream"] = json!(true);
    }
    body
}

fn assert_chat_request(fetch: &MockFetch, stream: bool) {
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].url,
        "https://api.mistral.ai/v1/chat/completions"
    );
    assert_eq!(requests[0].headers["authorization"], "Bearer test-api-key");
    assert_eq!(requests[0].json_body(), expected_request(stream));
}
fn usage_details_response() -> Value {
    serde_json::from_str(r###"{"id":"usage-details","created":1787866434,"model":"mistral-small-latest","usage":{"prompt_tokens":20,"completion_tokens":2,"total_tokens":22,"prompt_audio_seconds":1,"request_count":1,"service_tier":"standard","num_cached_tokens":0,"prompt_tokens_details":{"cached_tokens":0,"audio_tokens":1,"messages":[{"role":"user","total_tokens":20,"settings_tokens":null,"truncated":false,"usage_count":1}],"additional_prompt_detail":{"value":true}},"prompt_token_details":{"cached_tokens":0,"audio_tokens":1,"additional_prompt_token_detail":["value"]},"completion_tokens_details":{"reasoning_tokens":7,"additional_completion_detail":"value"},"additional_usage_field":{"nested":true}},"object":"chat.completion","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","tool_calls":null,"content":"OK"}}]}"###).unwrap()
}

fn tool_call_response() -> Value {
    serde_json::from_str(r###"{"id":"b3999b8c93e04e11bcbff7bcab829667","created":1769088854,"model":"mistral-small-latest","usage":{"prompt_tokens":124,"total_tokens":146,"completion_tokens":22},"object":"chat.completion","choices":[{"index":0,"finish_reason":"tool_calls","message":{"role":"assistant","tool_calls":[{"id":"gSIMJiOkT","function":{"name":"weather","arguments":"{\"location\": \"San Francisco\"}"}}]}}]}"###).unwrap()
}

fn text_chunks() -> Value {
    serde_json::from_str(r###"[{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":", "},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":"world!"},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":" This"},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":" is a test"},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":" response."},"finish_reason":null,"logprobs":null}]},{"id":"5319bd0299614c679a0068a4f2c8ffd0","object":"chat.completion.chunk","created":1769088720,"model":"mistral-small-latest","choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":13,"total_tokens":21,"completion_tokens":8}}]"###).unwrap()
}

/// TS: "should preserve the complete raw usage object without changing normalized usage" (mistral/src/mistral-chat-language-model.test.ts)
#[tokio::test]
async fn generate_preserves_complete_usage() {
    let body = usage_details_response();
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
    let result = provider(&fetch)
        .chat("mistral-small-latest")
        .do_generate(&options())
        .await
        .unwrap();
    assert_chat_request(&fetch, false);
    assert_eq!(result.request.unwrap().body, Some(expected_request(false)));
    assert_eq!(
        result.content,
        vec![GenerateContent::Text {
            text: "OK".into(),
            provider_metadata: None
        }]
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("stop"));
    assert_eq!(
        serde_json::to_value(result.usage).unwrap(),
        json!({
            "inputTokens": {"total": 20, "noCache": 20},
            "outputTokens": {"total": 2, "text": 2},
            "raw": body["usage"]
        })
    );
    assert!(result.warnings.is_empty());
    assert!(result.provider_metadata.is_none());
    let response = result.response.unwrap();
    assert_eq!(response.id.as_deref(), Some("usage-details"));
    assert_eq!(response.model_id.as_deref(), Some("mistral-small-latest"));
    assert_eq!(response.body, Some(body));
}

/// TS: "should stream text" (mistral/src/mistral-chat-language-model.test.ts)
#[tokio::test]
async fn stream_text_and_usage() {
    let chunks = text_chunks();
    let mut body: String = chunks
        .as_array()
        .unwrap()
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    body.push_str("data: [DONE]\n\n");
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }]);
    let result = provider(&fetch)
        .chat("mistral-small-latest")
        .do_stream(&options())
        .await
        .unwrap();
    assert_chat_request(&fetch, true);
    assert_eq!(result.request.unwrap().body, Some(expected_request(true)));
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 11);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    match &parts[1] {
        StreamPart::ResponseMetadata(metadata) => {
            assert_eq!(
                metadata.id.as_deref(),
                Some("5319bd0299614c679a0068a4f2c8ffd0")
            );
            assert_eq!(metadata.model_id.as_deref(), Some("mistral-small-latest"));
            assert_eq!(
                metadata.timestamp.as_deref(),
                Some("2026-01-22T13:32:00+00:00")
            );
        }
        part => panic!("unexpected metadata: {part:?}"),
    }
    assert!(
        matches!(&parts[2], StreamPart::TextStart { id, provider_metadata } if id == "0" && provider_metadata.is_none())
    );
    for (part, expected) in
        parts[3..9]
            .iter()
            .zip(["Hello", ", ", "world!", " This", " is a test", " response."])
    {
        assert!(
            matches!(part, StreamPart::TextDelta { id, delta, provider_metadata } if id == "0" && delta == expected && provider_metadata.is_none())
        );
    }
    assert!(
        matches!(&parts[9], StreamPart::TextEnd { id, provider_metadata } if id == "0" && provider_metadata.is_none())
    );
    match &parts[10] {
        StreamPart::Finish {
            finish_reason,
            usage,
            provider_metadata,
        } => {
            assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
            assert_eq!(finish_reason.raw.as_deref(), Some("stop"));
            assert_eq!(
                serde_json::to_value(usage).unwrap(),
                json!({
                    "inputTokens": {"total": 13, "noCache": 13},
                    "outputTokens": {"total": 8, "text": 8},
                    "raw": {"prompt_tokens": 13, "completion_tokens": 8, "total_tokens": 21}
                })
            );
            assert!(provider_metadata.is_none());
        }
        part => panic!("unexpected finish: {part:?}"),
    }
}

/// TS: "should extract tool call content" (mistral/src/mistral-chat-language-model.test.ts)
#[tokio::test]
async fn generate_tool_call_keeps_argument_text() {
    let body = tool_call_response();
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
    let result = provider(&fetch)
        .chat("mistral-small-latest")
        .do_generate(&options())
        .await
        .unwrap();
    assert_chat_request(&fetch, false);
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall(call) => {
            assert_eq!(call.tool_call_id, "gSIMJiOkT");
            assert_eq!(call.tool_name, "weather");
            assert_eq!(call.input, "{\"location\": \"San Francisco\"}");
            assert!(call.provider_executed.is_none());
            assert!(call.dynamic.is_none());
            assert!(call.provider_metadata.is_none());
        }
        part => panic!("unexpected tool content: {part:?}"),
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("tool_calls"));
    assert_eq!(result.usage.input_tokens.total, Some(124));
    assert_eq!(result.usage.output_tokens.total, Some(22));
    assert_eq!(result.usage.raw, body["usage"].as_object().cloned());
    assert!(result.warnings.is_empty());
    assert!(result.provider_metadata.is_none());
}

/// TS: "should extract embedding" (mistral/src/mistral-embedding-model.test.ts)
#[tokio::test]
async fn embed_request_vectors_and_usage() {
    let body = json!({
        "id": "b322cfc2b9d34e2f8e14fc99874faee5", "object": "list",
        "data": [
            {"object": "embedding", "embedding": [0.1, 0.2, 0.3, 0.4, 0.5], "index": 0},
            {"object": "embedding", "embedding": [0.6, 0.7, 0.8, 0.9, 1], "index": 1}
        ],
        "model": "mistral-embed", "usage": {"prompt_tokens": 8, "total_tokens": 8}
    });
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
    let mut options = EmbeddingCallOptions::new("sunny day at the beach");
    options.values.push("rainy day in the city".into());
    let result = provider(&fetch)
        .embedding("mistral-embed")
        .do_embed(&options)
        .await
        .unwrap();
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, "https://api.mistral.ai/v1/embeddings");
    assert_eq!(requests[0].headers["authorization"], "Bearer test-api-key");
    assert_eq!(
        requests[0].json_body(),
        json!({"model": "mistral-embed", "input": options.values, "encoding_format": "float"})
    );
    assert_eq!(
        result.embeddings,
        vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
    );
    assert_eq!(result.usage.unwrap().tokens, 8);
    assert_eq!(result.response.unwrap().body, Some(body));
    assert!(result.provider_metadata.is_none());
    assert!(result.warnings.is_empty());
}
