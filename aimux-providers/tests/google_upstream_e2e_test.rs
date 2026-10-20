//! Small public-factory end-to-end subset of the pinned Google upstream tests.
#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::FinishReasonUnified;
use aimux_core::video_model::{VideoCallOptions, VideoData, VideoModel, VideoOperationStatus};
use aimux_providers::google::{GoogleProviderSettings, create_google};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};
use std::sync::Arc;

const BASE: &str = "https://api.example.com/v1beta";

fn settings(mock: &Arc<MockFetch>) -> GoogleProviderSettings {
    GoogleProviderSettings {
        base_url: Some(BASE.into()),
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        generate_id: Some(Arc::new(|| "test-id".into())),
        ..Default::default()
    }
}

fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn basic_request() -> Value {
    json!({"contents":[{"role":"user","parts":[{"text":"Hello"}]}],"generationConfig":{}})
}

/// partial: `should preserve complete raw usage metadata`; full port in src/google/tests/language_model.rs.
#[tokio::test]
async fn generate_preserves_usage_and_metadata() {
    let usage: Value = serde_json::from_str(r##"{"promptTokenCount": 12, "cachedContentTokenCount": 4, "candidatesTokenCount": 71, "toolUsePromptTokenCount": 65, "thoughtsTokenCount": 89, "totalTokenCount": 237, "promptTokensDetails": [{"modality": "TEXT", "tokenCount": 12, "nestedSentinel": "prompt"}], "cacheTokensDetails": [{"modality": "TEXT", "tokenCount": 4, "nestedSentinel": "cache"}], "candidatesTokensDetails": [{"modality": "TEXT", "tokenCount": 71, "nestedSentinel": "candidate"}], "toolUsePromptTokensDetails": [{"modality": "TEXT", "tokenCount": 65, "nestedSentinel": "tool"}], "serviceTier": "standard", "topLevelSentinel": "preserve-me"}"##).unwrap();
    let body = json!({"candidates":[{"content":{"parts":[{"text":"Blue."}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":usage});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let result = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-pro")
        .do_generate(&options())
        .await
        .unwrap();
    assert_eq!(mock.seen().len(), 1);
    let sent = &mock.seen()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(
        sent.url,
        format!("{BASE}/models/gemini-pro:generateContent")
    );
    assert_eq!(sent.headers["x-goog-api-key"], "test-api-key");
    assert_eq!(sent.json_body(), basic_request());
    assert_eq!(result.request.as_ref().unwrap().body, Some(basic_request()));
    assert_eq!(result.response.as_ref().unwrap().body, Some(body));
    assert!(matches!(&result.content[..], [GenerateContent::Text { text, .. }] if text == "Blue."));
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(77));
    assert_eq!(result.usage.input_tokens.no_cache, Some(73));
    assert_eq!(result.usage.input_tokens.cache_read, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(160));
    assert_eq!(result.usage.output_tokens.text, Some(71));
    assert_eq!(result.usage.output_tokens.reasoning, Some(89));
    assert_eq!(result.usage.raw, usage.as_object().cloned());
    let metadata = result.provider_metadata.unwrap();
    assert_eq!(metadata["google"]["usageMetadata"], usage);
    assert_eq!(metadata["google"]["serviceTier"], "standard");
}

/// partial: `should stream text deltas`; full port in src/google/tests/language_model.rs.
#[tokio::test]
async fn stream_text_and_finish_usage() {
    let chunks = r##"{"candidates":[{"content":{"parts":[{"text":"There are **3**"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":5,"totalTokenCount":199,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}
{"candidates":[{"content":{"parts":[{"text":" \"r\"s in strawberry.\n\nst**r**awbe**rr**y"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":23,"totalTokenCount":217,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}
{"candidates":[{"content":{"parts":[{"text":"","thoughtSignature":"EqsFCqgFAb4+9vvtAF5n87lB4OGDOoTRMOqp35jW65XsYXh6BySMwl9nvrbAvPcl2U0xITaYUyV4CmREEDB1z0ZPpCg7iEwiZcj40Eh1jXoL8Y/BbPqxdgZKvKxdBsJx92y2ML5ytajQHVFQb9ohEMMnjs9uNadLAhDEsOU1nC5tl3FQkx94uaGfWvg61bJT3Y9OxFdo/kbpm4RBngvYhVkBzHKkHBj72T2bUd8J4HPssi7ORC5iPosPRIOyH/CAVHEtMzFYMwb7OhRu+CW8Z9u7gDieME5iJjXtJtLrNGDxgR7XtWfRRyGjsj6uDS+KvjR3SUSWPdn5eeH6w+LXZm1X///Hvhhcx+NHxsuGjF3fGhyzTVAoIzk0lxyB4+/A9I4Xa0o/T4coVDiewMzGZDwmket//ig8x9UC8cyWr/hy1joZWUO7ooJlLncv8gy4Ng+y1JdievZokSFDNWfMMNAQr3kgUwJDucqDp44C1xMtgR3lhJ75IBBnprHCE/ThgvNXujmqNkwAjp5dS4PjVbrw8fqSylfE80tvU0g9dXqg4pEyG+hGIxbANLhsWjAKLqh69hyqvVLg2Ds3wppphf61IfC4VoeLWj85CjBZMf+k85NsUIJQ6+DQS9IPNbM29ZOzpUbHoWKJB6VzNCSJse7Pi07L+pd6skl77km00y4lJdHIGHfEgi8PaOonakBcxbRqKzGJAA/urlP0tiWya2fTWrvNZOybJHyyofNNSI4s5y76yKEjP1wnPqC7ujrQk6xb7eyCeqH9ekByy3vv0JfgERFptoSUoG2toIr9M3lS/LKpnwfCvZh+z3J0iMb83d4MaPKhGhE49J4660XUsEmjygAZNi9HnjfC3KtaU/07Sx4JCezMtpsLKUxBgy4xaNqwew3FwAG37eeWcow="}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":23,"totalTokenCount":217,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"thoughtsTokenCount":185},"modelVersion":"gemini-3-pro-preview","responseId":"bH6LaZW8Fp_3nsEPqtaSwQ4"}"##;
    let mock = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks
            .lines()
            .map(|line| format!("data: {line}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }]);
    let result = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-pro")
        .do_stream(&options())
        .await
        .unwrap();
    assert_eq!(result.request.as_ref().unwrap().body, Some(basic_request()));
    assert_eq!(
        result.response.as_ref().unwrap().headers.as_ref().unwrap()["content-type"],
        "text/event-stream"
    );
    let events: Vec<_> = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(mock.seen().len(), 1);
    let sent = &mock.seen()[0];
    assert_eq!(
        sent.url,
        format!("{BASE}/models/gemini-pro:streamGenerateContent?alt=sse")
    );
    assert_eq!(sent.json_body(), basic_request());
    let deltas: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec![
            "There are **3**",
            " \"r\"s in strawberry.\n\nst**r**awbe**rr**y",
            ""
        ]
    );
    let last_chunk: Value = serde_json::from_str(chunks.lines().last().unwrap()).unwrap();
    assert!(events.iter().any(|event| matches!(event,
        StreamPart::TextDelta { delta, provider_metadata: Some(metadata), .. }
        if delta.is_empty() && metadata["google"]["thoughtSignature"] == last_chunk["candidates"][0]["content"]["parts"][0]["thoughtSignature"]
    )));
    let (finish, usage, metadata) = events
        .iter()
        .find_map(|event| match event {
            StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata,
            } => Some((finish_reason, usage, provider_metadata.as_ref().unwrap())),
            _ => None,
        })
        .unwrap();
    assert_eq!(finish.unified, FinishReasonUnified::Stop);
    assert_eq!(usage.input_tokens.total, Some(9));
    assert_eq!(usage.output_tokens.total, Some(208));
    assert_eq!(usage.output_tokens.text, Some(23));
    assert_eq!(usage.output_tokens.reasoning, Some(185));
    assert_eq!(metadata["google"]["usageMetadata"]["totalTokenCount"], 217);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamPart::TextStart { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamPart::TextEnd { .. }))
            .count(),
        1
    );
}

/// partial: `should extract tool calls`; full port in src/google/tests/language_model.rs.
#[tokio::test]
async fn generate_function_call_with_signature() {
    let body: Value = serde_json::from_str(r##"{
  "candidates": [
    {
      "content": {
        "parts": [
          {
            "functionCall": {
              "name": "weather",
              "args": {
                "location": "San Francisco"
              }
            },
            "thoughtSignature": "EskgCsYgAb4+9vtF7/499YQS2bjZs3xcQI+iAl+ILn29nK1j0Kg6su7QsUUUk3nrAAfnS2w5WiVvlcCqu9fAebJ2cvfaEyBahEt5"
          }
        ],
        "role": "model"
      },
      "finishReason": "STOP",
      "index": 0,
      "finishMessage": "Model generated function call(s)."
    }
  ],
  "usageMetadata": {
    "promptTokenCount": 29,
    "candidatesTokenCount": 15,
    "totalTokenCount": 937,
    "promptTokensDetails": [
      {
        "modality": "TEXT",
        "tokenCount": 29
      }
    ],
    "thoughtsTokenCount": 893
  },
  "modelVersion": "gemini-3-pro-preview",
  "responseId": "m36LaZGyCLz1xs0PtNSB-QU"
}
"##).unwrap();
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let schema = json!({"type":"object","properties":{"location":{"type":"string"}},"required":["location"],"additionalProperties":false,"$schema":"http://json-schema.org/draft-07/schema#"});
    let mut opts = options();
    opts.tools = Some(vec![Tool::Function(FunctionTool::new(
        "weather",
        schema.clone(),
    ))]);
    let result = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-pro")
        .do_generate(&opts)
        .await
        .unwrap();
    let mut expected = basic_request();
    expected["tools"] = json!([{"functionDeclarations":[{"name":"weather","description":"","parametersJsonSchema":schema}]}]);
    assert_eq!(mock.seen()[0].json_body(), expected);
    let [GenerateContent::ToolCall(call)] = &result.content[..] else {
        panic!("expected one tool call")
    };
    assert_eq!(call.tool_call_id, "test-id");
    assert_eq!(call.tool_name, "weather");
    assert_eq!(call.input, r#"{"location":"San Francisco"}"#);
    assert_eq!(
        call.provider_metadata.as_ref().unwrap()["google"]["thoughtSignature"],
        body["candidates"][0]["content"]["parts"][0]["thoughtSignature"]
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.usage.input_tokens.total, Some(29));
    assert_eq!(result.usage.output_tokens.total, Some(908));
}

/// partial: `preserves google.rpc.RetryInfo in APICallError.data`; full port in src/google/tests/error.rs.
#[tokio::test]
async fn error_preserves_retry_info() {
    let body = json!({"error":{"code":429,"message":"You exceeded your current quota, please check your plan.","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaId":"GenerateRequestsPerMinutePerProjectPerModel-FreeTier"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"34.4s"}]}});
    let mut canned = Canned::json(&body);
    canned.status = 429;
    let mock = MockFetch::new(vec![canned]);
    let error = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-2.5-flash")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert_eq!(mock.seen().len(), 1);
    assert_eq!(mock.seen()[0].json_body(), basic_request());
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected API call error")
    };
    assert_eq!(error.status_code, Some(429));
    assert_eq!(
        error.message,
        "You exceeded your current quota, please check your plan."
    );
    assert_eq!(error.data, Some(body));
}

/// partial: `should extract embedding`; full port in src/google/tests/embedding_model.rs.
/// partial: `should pass the model and the values`; full port in src/google/tests/embedding_model.rs.
#[tokio::test]
async fn embed_batch_request_and_vectors() {
    let body =
        json!({"embeddings":[{"values":[0.1,0.2,0.3,0.4,0.5]},{"values":[0.6,0.7,0.8,0.9,1.0]}]});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let mut opts = EmbeddingCallOptions::new("sunny day at the beach");
    opts.values.push("rainy day in the city".into());
    let result = create_google(settings(&mock))
        .unwrap()
        .embedding("gemini-embedding-001")
        .do_embed(&opts)
        .await
        .unwrap();
    let sent = &mock.seen()[0];
    assert_eq!(
        sent.url,
        format!("{BASE}/models/gemini-embedding-001:batchEmbedContents")
    );
    assert_eq!(
        sent.json_body(),
        json!({"requests":[{"model":"models/gemini-embedding-001","content":{"role":"user","parts":[{"text":"sunny day at the beach"}]}},{"model":"models/gemini-embedding-001","content":{"role":"user","parts":[{"text":"rainy day in the city"}]}}]})
    );
    assert_eq!(
        result.embeddings,
        vec![vec![0.1, 0.2, 0.3, 0.4, 0.5], vec![0.6, 0.7, 0.8, 0.9, 1.0]]
    );
    assert!(result.usage.is_none());
    assert_eq!(result.response.unwrap().body, Some(body));
}

/// partial: `should use the language model endpoint and extract generated images`; full port in src/google/tests/image_model.rs.
#[tokio::test]
async fn image_request_and_metadata() {
    let usage = json!({"promptTokenCount":10,"candidatesTokenCount":100,"totalTokenCount":110});
    let body = json!({"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"base64-generated-image"}}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":usage});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let result = create_google(settings(&mock))
        .unwrap()
        .image("gemini-2.5-flash-image")
        .do_generate(&ImageCallOptions::new("A beautiful sunset"))
        .await
        .unwrap();
    let sent = &mock.seen()[0];
    assert_eq!(
        sent.url,
        format!("{BASE}/models/gemini-2.5-flash-image:generateContent")
    );
    assert_eq!(
        sent.json_body(),
        json!({"contents":[{"role":"user","parts":[{"text":"A beautiful sunset"}]}],"generationConfig":{"responseModalities":["IMAGE"]}})
    );
    let ImageOutputs::Base64(images) = result.images else {
        panic!("expected base64 images")
    };
    assert_eq!(images, vec!["base64-generated-image"]);
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({"google":{"finishMessage":null,"finishReason":"STOP","groundingMetadata":null,"images":[{}],"promptFeedback":null,"safetyRatings":null,"serviceTier":null,"urlContextMetadata":null,"usageMetadata":usage}})
    );
    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(10));
    assert_eq!(usage.output_tokens, Some(100));
    assert_eq!(usage.total_tokens, Some(110));
}

/// partial: `should return operation with operationName`; full port in src/google/tests/video_model.rs.
/// partial: `should pass correct request body`; full port in src/google/tests/video_model.rs.
/// partial: `should return completed with video data when done`; full port in src/google/tests/video_model.rs.
#[tokio::test]
async fn video_start_and_completed_status() {
    let mock = MockFetch::new(vec![
        Canned::json(&json!({"name":"operations/start-test-op","done":false})),
        Canned::json(
            &json!({"done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"https://api.example.com/files/video-456.mp4"}}]}}}),
        ),
    ]);
    let model = create_google(settings(&mock))
        .unwrap()
        .video("veo-3.1-generate-preview");
    let opts = VideoCallOptions::new("A futuristic city with flying cars");
    let start = model.do_start(&opts).await.unwrap();
    assert_eq!(
        start.operation,
        json!({"operationName":"operations/start-test-op"})
    );
    let sent = &mock.seen()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(
        sent.url,
        format!("{BASE}/models/veo-3.1-generate-preview:predictLongRunning")
    );
    assert_eq!(
        sent.json_body(),
        json!({"instances":[{"prompt":"A futuristic city with flying cars"}],"parameters":{"sampleCount":1}})
    );
    let status = model.do_status(&start.operation, &opts).await.unwrap();
    assert_eq!(mock.seen().len(), 2);
    assert_eq!(mock.seen()[1].method, "GET");
    assert_eq!(
        mock.seen()[1].url,
        format!("{BASE}/operations/start-test-op")
    );
    let VideoOperationStatus::Completed(result) = status else {
        panic!("expected completed video")
    };
    assert_eq!(result.videos.len(), 1);
    assert!(
        matches!(&result.videos[0], VideoData::Url { url, media_type } if url == "https://api.example.com/files/video-456.mp4?key=test-api-key" && media_type == "video/mp4")
    );
}

/// partial: `should throw when no videos in response`; full port in src/google/tests/video_model.rs.
#[tokio::test]
async fn video_completed_response_without_videos_errors() {
    let mock = MockFetch::new(vec![Canned::json(
        &json!({"done":true,"response":{"generateVideoResponse":{"generatedSamples":[]}}}),
    )]);
    let error = create_google(settings(&mock))
        .unwrap()
        .video("veo-3.1-generate-preview")
        .do_status(
            &json!({"operationName":"operations/status-test-op"}),
            &VideoCallOptions::new("A futuristic city with flying cars"),
        )
        .await
        .unwrap_err();
    assert_eq!(mock.seen()[0].method, "GET");
    assert_eq!(
        mock.seen()[0].url,
        format!("{BASE}/operations/status-test-op")
    );
    assert!(error.to_string().contains("No videos in response"));
}

const THOUGHT_SIGNATURE: &str = "EuIDCt8DARFNMg/aRDRK3THWhBjzltCEy5/VM6ImWLJU8oHmnC75abdcZBMH";

/// user → assistant tool call (with `signature` in providerOptions.google)
/// → tool result: the second turn that issue #9 saw rejected with HTTP 400.
fn second_turn(signature: Option<&str>) -> Vec<LanguageModelMessage> {
    let mut call = json!({
        "type": "tool-call",
        "toolCallId": "call-1",
        "toolName": "weather",
        "input": {"location": "San Francisco"}
    });
    if let Some(signature) = signature {
        call["providerOptions"] = json!({"google": {"thoughtSignature": signature}});
    }
    serde_json::from_value(json!([
        {"role": "user", "content": [{"type": "text", "text": "weather in SF?"}]},
        {"role": "assistant", "content": [call]},
        {"role": "tool", "content": [{
            "type": "tool-result",
            "toolCallId": "call-1",
            "toolName": "weather",
            "output": {"type": "json", "value": {"temp": 70}}
        }]}
    ]))
    .unwrap()
}

/// The `parts` of the model turn that `model_id` was sent for `prompt`.
async fn replayed_model_parts(model_id: &str, prompt: Vec<LanguageModelMessage>) -> Value {
    let body = json!({"candidates":[{"content":{"parts":[{"text":"Done."}],"role":"model"},"finishReason":"STOP"}]});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    create_google(settings(&mock))
        .unwrap()
        .chat(model_id)
        .do_generate(&CallOptions::new(prompt))
        .await
        .unwrap();
    let sent = mock.seen()[0].json_body();
    assert_eq!(sent["contents"][1]["role"], "model", "{sent}");
    sent["contents"][1]["parts"].clone()
}

/// convert-to-google-messages.ts:404-411: a tool call's
/// providerOptions.google.thoughtSignature goes back as a sibling of
/// `functionCall` in the same part, not inside it (issue #9).
#[tokio::test]
async fn replays_a_thought_signature_beside_the_function_call() {
    let parts = replayed_model_parts("gemini-2.5-pro", second_turn(Some(THOUGHT_SIGNATURE))).await;
    assert_eq!(
        parts,
        json!([{
            "functionCall": {"id": "call-1", "name": "weather", "args": {"location": "San Francisco"}},
            "thoughtSignature": THOUGHT_SIGNATURE
        }])
    );
}

/// An unsigned call on a model before Gemini 3 is sent without the field.
#[tokio::test]
async fn an_unsigned_call_has_no_thought_signature_before_gemini_3() {
    let parts = replayed_model_parts("gemini-2.5-pro", second_turn(None)).await;
    assert!(parts[0].get("functionCall").is_some(), "{parts}");
    assert!(parts[0].get("thoughtSignature").is_none(), "{parts}");
}

/// convert-to-google-messages.ts:540-552: on Gemini 3 an unsigned call gets
/// the documented skip sentinel, while a signed call keeps its own signature.
#[tokio::test]
async fn gemini_3_fills_only_a_missing_thought_signature() {
    let parts = replayed_model_parts("gemini-3-pro-preview", second_turn(None)).await;
    assert_eq!(
        parts[0]["thoughtSignature"],
        "skip_thought_signature_validator"
    );

    let parts =
        replayed_model_parts("gemini-3-pro-preview", second_turn(Some(THOUGHT_SIGNATURE))).await;
    assert_eq!(parts[0]["thoughtSignature"], THOUGHT_SIGNATURE);
}

/// Upstream: `candidates` is `.nullish()` (@ai-sdk/google@4.0.87 src/google-language-model.ts:1744),
/// so a blocked prompt answered with `"candidates": null` finishes as a content filter.
#[tokio::test]
async fn generate_reads_null_candidates_as_none() {
    let body = json!({"candidates": null, "promptFeedback": {"blockReason": "SAFETY"}});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let result = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-pro")
        .do_generate(&options())
        .await
        .unwrap();
    assert!(result.content.is_empty());
    assert_eq!(
        result.finish_reason.unified,
        FinishReasonUnified::ContentFilter
    );
    assert_eq!(result.finish_reason.raw.as_deref(), Some("SAFETY"));
}

/// Upstream: server tool metadata carries `thoughtSignature` only when it is
/// not empty (@ai-sdk/google@4.0.87 src/google-language-model.ts:585-594,
/// :604-613).
#[tokio::test]
async fn server_tool_metadata_leaves_out_an_empty_thought_signature() {
    let body = json!({"candidates": [{"content": {"role": "model", "parts": [
        {"toolCall": {"id": "s1", "toolType": "GOOGLE_SEARCH_WEB", "args": {}}, "thoughtSignature": ""},
        {"toolResponse": {"id": "s1", "toolType": "GOOGLE_SEARCH_WEB", "response": {}}, "thoughtSignature": "sig"}
    ]}, "finishReason": "STOP"}]});
    let mock = MockFetch::new(vec![Canned::json(&body)]);
    let result = create_google(settings(&mock))
        .unwrap()
        .chat("gemini-pro")
        .do_generate(&options())
        .await
        .unwrap();
    let content = serde_json::to_value(&result.content).unwrap();
    assert_eq!(
        content[0]["providerMetadata"]["google"],
        json!({"serverToolCallId": "s1", "serverToolType": "GOOGLE_SEARCH_WEB"})
    );
    assert_eq!(
        content[1]["providerMetadata"]["google"]["thoughtSignature"],
        "sig"
    );
}
