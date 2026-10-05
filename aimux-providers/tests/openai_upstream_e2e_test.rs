//! Small end-to-end subset of the pinned upstream tests, using an in-process transport.
#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
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
use aimux_provider_utils::Resolvable;
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        base_url: Some("https://example.test/v1".into()),
        api_key: Some(Resolvable::Value("test-key".into())),
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
    assert_eq!(result.usage.raw, Some(chat_response()["usage"].clone()));
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
            provider_metadata: Some(provider_namespace("openai", json!({"itemId":"msg_test"})))
        }]
    );
    assert_eq!(result.usage.input_tokens.total, Some(345));
    assert_eq!(result.usage.input_tokens.cache_read, Some(234));
    assert_eq!(result.usage.input_tokens.cache_write, Some(45));
    assert_eq!(result.usage.input_tokens.no_cache, Some(66));
    assert_eq!(result.usage.output_tokens.total, Some(538));
    assert_eq!(result.usage.output_tokens.reasoning, Some(123));
    assert_eq!(result.usage.output_tokens.text, Some(415));
    assert_eq!(result.usage.raw, Some(response["usage"].clone()));
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
    let deltas = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { id, delta, .. } => Some((id.as_str(), delta.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deltas,
        vec![
            ("msg_67c9a81dea8c8190b79651a2b3adf91e", "Hello,"),
            ("msg_67c9a81dea8c8190b79651a2b3adf91e", " World!")
        ]
    );
    assert!(parts.iter().any(|part|matches!(part,StreamPart::TextEnd{id,..} if id=="msg_67c9a81dea8c8190b79651a2b3adf91e")));
    assert!(
        matches!(parts.last(),Some(StreamPart::Finish{finish_reason,usage,provider_metadata:Some(metadata)}) if finish_reason.unified==FinishReasonUnified::Stop && usage.input_tokens.total==Some(543) && usage.input_tokens.cache_read==Some(234) && usage.output_tokens.reasoning==Some(123) && metadata["openai"]["responseId"]=="resp_67c9a81b6a048190a9ee441c5755a4e8")
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
    opts.provider_options = provider_namespace("openai", json!({"style":"vivid"}));
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
