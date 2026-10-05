//! Small end-to-end subset of the pinned Azure provider tests.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel};
use aimux_core::stream_part::StreamPart;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::Resolvable;
use aimux_providers::azure::{AzureOpenAIProvider, AzureOpenAIProviderSettings, create_azure};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};
use std::sync::Arc;

fn provider(fetch: &Arc<MockFetch>) -> AzureOpenAIProvider {
    create_azure(AzureOpenAIProviderSettings {
        resource_name: Some("test-resource".into()),
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn assert_request(fetch: &MockFetch, endpoint: &str) -> Value {
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        format!("https://test-resource.openai.azure.com/openai/v1/{endpoint}?api-version=v1")
    );
    assert_eq!(seen[0].headers["api-key"], "test-api-key");
    seen[0].json_body()
}
fn chat_response() -> Value {
    json!({"id":"chatcmpl-1","object":"chat.completion","created":1711115037,"model":"test-deployment","choices":[{"index":0,"message":{"role":"assistant","content":"Hello World!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":30,"total_tokens":34}})
}

fn text_fixture() -> Value {
    json!({"id":"resp_0d6bb044bb6ff37200698c51948054819385e24e2ad931ae6e","object":"response","created_at":1770803604,"status":"completed","model":"test-deployment","output":[{"id":"msg_0d6bb044bb6ff37200698c51952e288193beb9044db4d8c810","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Word","annotations":[],"logprobs":[]}]}],"usage":{"input_tokens":11,"input_tokens_details":{"cached_tokens":0},"output_tokens":11,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":22}})
}

fn image_response() -> Value {
    json!({"created":1733837122,"data":[{"revised_prompt":"A charming visual illustration of a baby sea otter swimming joyously.","b64_json":"base64-image-1"},{"revised_prompt":"A charming visual illustration of a baby sea otter swimming joyously.","b64_json":"base64-image-2"}]})
}

fn text_chunks() -> &'static str {
    r###"data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"in_progress","background":false,"completed_at":null,"content_filters":null,"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"auto","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":null,"user":null,"metadata":{}}}

data: {"type":"response.in_progress","sequence_number":1,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"in_progress","background":false,"completed_at":null,"content_filters":null,"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"auto","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":null,"user":null,"metadata":{}}}

data: {"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"in_progress","content":[],"role":"assistant"}}

data: {"type":"response.content_part.added","sequence_number":3,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}

data: {"type":"response.output_text.delta","sequence_number":4,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"delta":"Hello","logprobs":[],"obfuscation":"VLMdSoETMf5"}

data: {"type":"response.output_text.done","sequence_number":5,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"text":"Hello","logprobs":[]}

data: {"type":"response.content_part.done","sequence_number":6,"item_id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}}

data: {"type":"response.output_item.done","sequence_number":7,"output_index":0,"item":{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}],"role":"assistant"}}

data: {"type":"response.completed","sequence_number":8,"response":{"id":"resp_02ce8deeb6197db200698c5196e9588197a572bbea62d38cd1","object":"response","created_at":1770803606,"status":"completed","background":false,"completed_at":1770803608,"content_filters":[{"blocked":false,"source_type":"prompt","content_filter_raw":null,"content_filter_results":{"self_harm":{"filtered":false,"severity":"safe"},"sexual":{"filtered":false,"severity":"safe"},"violence":{"filtered":false,"severity":"safe"},"jailbreak":{"filtered":false,"detected":false},"hate":{"filtered":false,"severity":"safe"}},"content_filter_offsets":{"start_offset":1429,"end_offset":1447,"check_offset":0}},{"blocked":false,"source_type":"completion","content_filter_raw":null,"content_filter_results":{"hate":{"filtered":false,"severity":"safe"},"sexual":{"filtered":false,"severity":"safe"},"self_harm":{"filtered":false,"severity":"safe"},"violence":{"filtered":false,"severity":"safe"},"protected_material_text":{"filtered":false,"detected":false},"protected_material_code":{"filtered":false,"detected":false}},"content_filter_offsets":{"start_offset":0,"end_offset":5,"check_offset":0}}],"error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"test-deployment","output":[{"id":"msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"Hello"}],"role":"assistant"}],"parallel_tool_calls":true,"previous_response_id":null,"prompt_cache_key":null,"prompt_cache_retention":null,"reasoning":{"effort":"none","summary":null},"safety_identifier":null,"service_tier":"default","store":true,"temperature":1,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tools":[],"top_logprobs":0,"top_p":1,"truncation":"disabled","usage":{"input_tokens":11,"input_tokens_details":{"cached_tokens":0},"output_tokens":11,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":22},"user":null,"metadata":{}}}

data: [DONE]

"###
}

/// TS: "should use the baseURL correctly" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn chat_generate() {
    let fetch = MockFetch::new(vec![Canned::json(&chat_response())]);
    let result = provider(&fetch)
        .chat("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "chat/completions");
    assert_eq!(body["model"], "test-deployment");
    assert_eq!(body["messages"], json!([{"role":"user","content":"Hello"}]));
    assert_eq!(result.content.len(), 1);
    assert!(
        matches!(&result.content[0], GenerateContent::Text { text, .. } if text == "Hello World!")
    );
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
    assert_eq!(
        result.response.as_ref().unwrap().id.as_deref(),
        Some("chatcmpl-1")
    );
}

/// TS: "should extract text content" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
/// TS: "should extract usage" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn responses_generate() {
    let fixture = text_fixture();
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let result = provider(&fetch)
        .responses("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "responses");
    assert_eq!(body["model"], "test-deployment");
    assert_eq!(
        body["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::Text {
            text,
            provider_metadata,
        } => {
            assert_eq!(text, "Word");
            assert_eq!(
                provider_metadata.as_ref().unwrap()["azure"]["itemId"],
                "msg_0d6bb044bb6ff37200698c51952e288193beb9044db4d8c810"
            );
        }
        _ => panic!("expected text"),
    }
    assert_eq!(
        result.provider_metadata.as_ref().unwrap()["azure"]["responseId"],
        fixture["id"]
    );
    assert_eq!(result.usage.input_tokens.total, Some(11));
    assert_eq!(result.usage.input_tokens.cache_read, Some(0));
    assert_eq!(result.usage.input_tokens.no_cache, Some(11));
    assert_eq!(result.usage.output_tokens.total, Some(11));
    assert_eq!(result.usage.output_tokens.reasoning, Some(0));
    assert_eq!(result.usage.output_tokens.text, Some(11));
    assert_eq!(result.usage.raw, Some(fixture["usage"].clone()));
    assert_eq!(
        result.response.as_ref().unwrap().model_id.as_deref(),
        Some("test-deployment")
    );
}

/// TS: "should extract tool call content" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn responses_tool_call() {
    let mut fixture = text_fixture();
    fixture["output"] = json!([{"type":"function_call","id":"fc_0a2fa1b539ba14ba00698c519ebab0819494302fc0b5c31440","call_id":"call_YunNGbIwdVJ2i0y0Mybva4Pw","name":"weather","arguments":"{\"location\":\"San Francisco\"}","status":"completed"}]);
    let fetch = MockFetch::new(vec![Canned::json(&fixture)]);
    let result = provider(&fetch)
        .responses("test-deployment")
        .do_generate(&prompt())
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "responses")["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        GenerateContent::ToolCall(call) => {
            assert_eq!(call.tool_call_id, "call_YunNGbIwdVJ2i0y0Mybva4Pw");
            assert_eq!(call.tool_name, "weather");
            assert_eq!(call.input, "{\"location\":\"San Francisco\"}");
        }
        _ => panic!("expected tool call"),
    }
}

/// TS: "should stream text content" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn responses_stream() {
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: text_chunks().as_bytes().to_vec(),
    }]);
    let result = provider(&fetch)
        .responses("test-deployment")
        .do_stream(&prompt())
        .await
        .unwrap();
    let body = assert_request(&fetch, "responses");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["input"],
        json!([{"role":"user","content":[{"type":"input_text","text":"Hello"}]}])
    );
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 6);
    assert!(matches!(parts[0], StreamPart::StreamStart { .. }));
    assert!(matches!(parts[1], StreamPart::ResponseMetadata(_)));
    assert!(matches!(parts[2], StreamPart::TextStart { .. }));
    assert!(matches!(&parts[3], StreamPart::TextDelta { delta, .. } if delta == "Hello"));
    match &parts[4] {
        StreamPart::TextEnd {
            provider_metadata, ..
        } => assert_eq!(
            provider_metadata.as_ref().unwrap()["azure"]["itemId"],
            "msg_02ce8deeb6197db200698c5198ca0c81979bedbe6c98a8ab93"
        ),
        _ => panic!("expected text end"),
    }
    match &parts[5] {
        StreamPart::Finish { usage, .. } => {
            assert_eq!(usage.input_tokens.total, Some(11));
            assert_eq!(usage.output_tokens.total, Some(11));
        }
        _ => panic!("expected finish"),
    }
}

/// TS: "should set the correct api version" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn embedding_generate() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3,0.4,0.5]},{"object":"embedding","index":1,"embedding":[0.6,0.7,0.8,0.9,1.0]}],"model":"my-embedding","usage":{"prompt_tokens":8,"total_tokens":8}}),
    )]);
    let mut options = EmbeddingCallOptions::new("sunny day at the beach");
    options.values.push("rainy day in the city".into());
    let result = provider(&fetch)
        .embedding("my-embedding")
        .do_embed(&options)
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "embeddings"),
        json!({"model":"my-embedding","input":options.values,"encoding_format":"float"})
    );
    assert_eq!(
        result.embeddings,
        vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
    );
    assert_eq!(result.usage.unwrap().tokens, 8);
}

/// TS: "should send the correct request body" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
/// TS: "should extract the generated images" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn image_generate() {
    let fetch = MockFetch::new(vec![Canned::json(&image_response())]);
    let mut options = ImageCallOptions::new("A cute baby sea otter");
    options.n = 2;
    options.size = Some("1024x1024".parse().unwrap());
    options.provider_options =
        serde_json::from_value(json!({"openai":{"style":"natural"}})).unwrap();
    let result = provider(&fetch)
        .image("test-deployment")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "images/generations"),
        json!({"model":"test-deployment","prompt":"A cute baby sea otter","n":2,"size":"1024x1024","response_format":"b64_json","style":"natural"})
    );
    match result.images {
        ImageOutputs::Base64(images) => assert_eq!(images, ["base64-image-1", "base64-image-2"]),
        _ => panic!("expected base64 images"),
    }
}

/// TS: "should use correct URL format" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn transcription_generate() {
    let fetch = MockFetch::new(vec![Canned::json(
        &json!({"text":"Hello, world!","segments":[],"language":"en","duration":5.0}),
    )]);
    let result = provider(&fetch)
        .transcription("test-deployment")
        .do_generate(&TranscriptionCallOptions::new(
            AudioInput::Binary(vec![]),
            "audio/wav",
        ))
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://test-resource.openai.azure.com/openai/v1/audio/transcriptions?api-version=v1"
    );
    assert_eq!(seen[0].headers["api-key"], "test-api-key");
    assert!(seen[0].headers["content-type"].starts_with("multipart/form-data; boundary="));
    let body = String::from_utf8(seen[0].body.clone()).unwrap();
    assert!(body.contains("name=\"model\"\r\n\r\ntest-deployment"));
    assert!(body.contains("filename=\"audio.wav\""));
    assert!(body.contains("Content-Type: audio/wav"));
    assert_eq!(result.text, "Hello, world!");
    assert!(result.segments.is_empty());
    assert_eq!(result.duration_in_seconds, Some(5.0));
}

/// TS: "should use correct URL format" (@ai-sdk/azure/src/azure-openai-provider.test.ts)
#[tokio::test]
async fn speech_generate() {
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "audio/mpeg".into())],
        body: vec![1, 2, 3],
    }]);
    let result = provider(&fetch)
        .speech("test-deployment")
        .do_generate(&SpeechCallOptions::new("Hello, world!"))
        .await
        .unwrap();
    assert_eq!(
        assert_request(&fetch, "audio/speech"),
        json!({"model":"test-deployment","input":"Hello, world!","voice":"alloy","response_format":"mp3"})
    );
    assert!(matches!(result.audio, AudioData::Binary(audio) if audio == vec![1,2,3]));
}

/// TS: "sends audio and the required model definition" (azure-transcription-model.test.ts)
#[tokio::test]
async fn speech_transcription_request() {
    for audio in [
        AudioInput::Binary(vec![1, 2, 3]),
        AudioInput::Base64("AQID".into()),
    ] {
        let fetch = MockFetch::new(vec![Canned::json(&json!({
            "combinedPhrases": [{"text": "Hello world."}], "phrases": [],
        }))]);
        provider(&fetch)
            .transcription("mai-transcribe-2")
            .do_generate(&TranscriptionCallOptions::new(audio, "audio/wav"))
            .await
            .unwrap();
        let seen = fetch.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].url,
            "https://test-resource.cognitiveservices.azure.com/speechtotext/transcriptions:transcribe?api-version=2025-10-15"
        );
        assert_eq!(seen[0].headers["ocp-apim-subscription-key"], "test-api-key");
        assert!(!seen[0].headers.contains_key("api-key"));
        assert!(seen[0].headers["user-agent"].contains("ai-sdk-azure/"));
        let body = String::from_utf8(seen[0].body.clone()).unwrap();
        assert!(body.contains("name=\"audio\"; filename=\"audio.wav\""));
        assert!(body.contains("Content-Type: audio/wav"));
        assert!(body.contains("\r\n\r\n\u{1}\u{2}\u{3}\r\n"));
        let definition = body
            .split("name=\"definition\"\r\n\r\n")
            .nth(1)
            .unwrap()
            .split("\r\n")
            .next()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(definition).unwrap(),
            json!({
                "enhancedMode": {"enabled": true, "model": "MAI-Transcribe-2", "modelOptions": {"timestamps": "segment"}},
            })
        );
    }
}
