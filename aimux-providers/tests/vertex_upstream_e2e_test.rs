//! In-process end-to-end subset of the pinned Vertex upstream ports.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::result::GenerateContent;
use aimux_core::shared::{AspectRatio, provider_namespace};
use aimux_core::tool::FunctionTool;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};
use aimux_core::types::FinishReasonUnified;
use aimux_core::video_model::{VideoCallOptions, VideoData, VideoModel, VideoOperationStatus};
use aimux_provider_utils::Resolvable;
use aimux_providers::vertex::{VertexProvider, VertexProviderSettings, create_google_vertex};
use mock_fetch::{Canned, MockFetch};
use serde_json::json;
use std::sync::Arc;

fn provider(fetch: &Arc<MockFetch>) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        base_url: Some("https://api.example.com".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// TS: "should preserve local JSON Schema references in tool requests" (@ai-sdk/google-vertex/src/google-vertex-language-model.test.ts)
#[tokio::test]
async fn language_generate_preserves_tool_schema_references() {
    let response = json!({
        "candidates": [{"content": {"parts": [{"text": "Done"}], "role": "model"}, "finishReason": "STOP", "index": 0}],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}
    });
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let schema = json!({
        "type": "object",
        "properties": {"locale": {"$ref": "#/$defs/Locale", "description": "Locale for formatting"}},
        "required": ["locale"], "additionalProperties": false,
        "$defs": {"Locale": {"type": "string", "enum": ["de", "en"]}}
    });
    let mut options = CallOptions::new(vec![LanguageModelMessage::user_text("Hello")]);
    options.tools = Some(vec![Tool::Function(FunctionTool {
        name: "format-date".into(),
        description: Some("Format a date".into()),
        input_schema: schema.clone(),
        strict: None,
        provider_options: None,
        input_examples: None,
    })]);
    let result = provider(&fetch)
        .chat("gemini-2.5-flash")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://api.example.com/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "test-api-key");
    let body = seen[0].json_body();
    assert_eq!(
        body["contents"],
        json!([{"role": "user", "parts": [{"text": "Hello"}]}])
    );
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0],
        json!({
            "name": "format-date", "description": "Format a date", "parametersJsonSchema": schema
        })
    );
    assert_eq!(
        result.content,
        vec![GenerateContent::Text {
            text: "Done".into(),
            provider_metadata: None
        }]
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.usage.input_tokens.total, Some(1));
    assert_eq!(result.usage.output_tokens.total, Some(1));
    assert_eq!(
        result.usage.raw,
        response["usageMetadata"].as_object().cloned()
    );
    let metadata = serde_json::to_value(result.provider_metadata.unwrap()).unwrap();
    assert_eq!(
        metadata["googleVertex"]["usageMetadata"],
        response["usageMetadata"]
    );
    assert_eq!(metadata["vertex"], metadata["googleVertex"]);
    assert_eq!(result.request.unwrap().body.unwrap(), body);
    assert_eq!(result.response.unwrap().body.unwrap(), response);
}

/// TS: "should pass the model parameters correctly" (@ai-sdk/google-vertex/src/google-vertex-embedding-model.test.ts)
/// TS: "should extract embeddings" (@ai-sdk/google-vertex/src/google-vertex-embedding-model.test.ts)
/// TS: "should extract usage" (@ai-sdk/google-vertex/src/google-vertex-embedding-model.test.ts)
#[tokio::test]
async fn embedding_request_and_result() {
    let response = json!({"predictions": [
        {"embeddings": {"statistics": {"token_count": 5, "truncated": false}, "values": [-0.017999587580561638, -0.006893285550177097, -0.036766719073057175, -0.017558680847287178, -0.019938766956329346]}},
        {"embeddings": {"statistics": {"token_count": 6, "truncated": false}, "values": [-0.06007182598114014, 0.004907649010419846, -0.00690646655857563, -0.007314121350646019, -0.048464205116033554]}}
    ], "metadata": {"billableCharacterCount": 35}});
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let mut options = EmbeddingCallOptions::new("test text one");
    options.values.push("test text two".into());
    options.provider_options = Some(provider_namespace(
        "google",
        json!({
            "outputDimensionality": 768, "taskType": "SEMANTIC_SIMILARITY", "title": "test title", "autoTruncate": false
        }),
    ).expect("provider metadata must be an object"));
    let result = provider(&fetch)
        .embedding("textembedding-gecko@001")
        .do_embed(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://api.example.com/models/textembedding-gecko@001:predict"
    );
    assert_eq!(
        seen[0].json_body(),
        json!({
            "instances": [
                {"content": "test text one", "task_type": "SEMANTIC_SIMILARITY", "title": "test title"},
                {"content": "test text two", "task_type": "SEMANTIC_SIMILARITY", "title": "test title"}
            ], "parameters": {"outputDimensionality": 768, "autoTruncate": false}
        })
    );
    let expected: Vec<Vec<f32>> = response["predictions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|prediction| {
            prediction["embeddings"]["values"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap() as f32)
                .collect()
        })
        .collect();
    assert_eq!(result.embeddings, expected);
    assert_eq!(result.usage.unwrap().tokens, 11);
    assert!(result.warnings.is_empty());
    assert!(result.provider_metadata.is_none());
    assert_eq!(result.response.unwrap().body.unwrap(), response);
}

/// TS: "should send response modalities, aspect ratio, seed, and headers" (@ai-sdk/google-vertex/src/google-vertex-image-model.test.ts)
/// TS: "should use the language model endpoint and extract generated images" (@ai-sdk/google-vertex/src/google-vertex-image-model.test.ts)
/// TS: "should include usage and response metadata" (@ai-sdk/google-vertex/src/google-vertex-image-model.test.ts)
#[tokio::test]
async fn image_generate_request_and_result() {
    let response = json!({
        "candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "image/png", "data": "base64-generated-image"}}], "role": "model"}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 20, "candidatesTokenCount": 200, "totalTokenCount": 220}
    });
    let mut canned = Canned::json(&response);
    canned
        .headers
        .push(("request-id".into(), "test-request-id".into()));
    let fetch = MockFetch::new(vec![canned]);
    let provider = create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-key".into())),
        base_url: Some("https://api.example.com".into()),
        headers: Some(Resolvable::Value(
            [(
                "Custom-Provider-Header".into(),
                Some("provider-header-value".into()),
            )]
            .into(),
        )),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options = ImageCallOptions::new("A beautiful sunset");
    options.aspect_ratio = Some(AspectRatio::new(21, 9));
    options.seed = Some(12345);
    options.headers = Some(
        [(
            "Custom-Request-Header".into(),
            "request-header-value".into(),
        )]
        .into(),
    );
    let result = provider
        .image("gemini-2.5-flash-image")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://api.example.com/models/gemini-2.5-flash-image:generateContent"
    );
    assert_eq!(
        seen[0].headers["custom-provider-header"],
        "provider-header-value"
    );
    assert_eq!(
        seen[0].headers["custom-request-header"],
        "request-header-value"
    );
    assert_eq!(
        seen[0].json_body(),
        json!({
            "contents": [{"role": "user", "parts": [{"text": "A beautiful sunset"}]}],
            "generationConfig": {"responseModalities": ["IMAGE"], "imageConfig": {"aspectRatio": "21:9"}, "seed": 12345}
        })
    );
    assert!(
        matches!(result.images, ImageOutputs::Base64(images) if images == ["base64-generated-image"])
    );
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({"googleVertex": {"images": [{}]}, "vertex": {"images": [{}]}})
    );
    let usage = result.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(20));
    assert_eq!(usage.output_tokens, Some(200));
    assert_eq!(usage.total_tokens, Some(220));
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("gemini-2.5-flash-image")
    );
    assert_eq!(
        result.response.headers.unwrap()["request-id"],
        "test-request-id"
    );
}

/// TS: "should send the model, languageCodes, features and base64 content" (@ai-sdk/google-vertex/src/google-vertex-transcription-model.test.ts)
/// TS: "should extract text, segments, language and duration" (@ai-sdk/google-vertex/src/google-vertex-transcription-model.test.ts)
#[tokio::test]
async fn transcription_generate_request_and_result() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "results": [{"alternatives": [{"transcript": "hello world", "words": [
            {"word": "hello", "startOffset": "0s", "endOffset": "0.500s"},
            {"word": "world", "startOffset": "0.500s", "endOffset": "1s"}
        ]}], "languageCode": "en-US"}], "metadata": {"totalBilledDuration": "1s"}
    }))]);
    let provider = create_google_vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("test-token".into())),
        project: Some("test-project".into()),
        location: Some("us-central1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    let options = TranscriptionCallOptions::new(
        AudioInput::Binary(vec![1, 2, 3, 4, 5, 6, 7, 8]),
        "audio/wav",
    );
    let result = provider
        .transcription("chirp_2")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://us-central1-speech.googleapis.com/v2/projects/test-project/locations/us-central1/recognizers/_:recognize"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer test-token");
    assert_eq!(
        seen[0].json_body(),
        json!({
            "config": {"model": "chirp_2", "languageCodes": ["auto"], "autoDecodingConfig": {}, "features": {"enableWordTimeOffsets": true, "enableAutomaticPunctuation": true}},
            "content": "AQIDBAUGBwg="
        })
    );
    assert_eq!(result.text, "hello world");
    assert_eq!(
        serde_json::to_value(result.segments).unwrap(),
        json!([
            {"text": "hello", "start_second": 0.0, "end_second": 0.5},
            {"text": "world", "start_second": 0.5, "end_second": 1.0}
        ])
    );
    assert_eq!(result.language.as_deref(), Some("en"));
    assert_eq!(result.duration_in_seconds, Some(1.0));
}

/// TS: "should pass correct request body" (@ai-sdk/google-vertex/src/google-vertex-video-model.test.ts)
/// TS: "should return operation with operationName" (@ai-sdk/google-vertex/src/google-vertex-video-model.test.ts)
/// TS: "should return completed with video data when done" (@ai-sdk/google-vertex/src/google-vertex-video-model.test.ts)
#[tokio::test]
async fn video_start_and_completed_result() {
    let fetch = MockFetch::new(vec![
        Canned::json(&json!({"name": "operations/my-op-123", "done": false})),
        Canned::json(
            &json!({"done": true, "response": {"videos": [{"bytesBase64Encoded": "base64-video-data", "mimeType": "video/mp4"}]}}),
        ),
    ]);
    let model = provider(&fetch).video("veo-2.0-generate-001");
    let options = VideoCallOptions::new("A futuristic city with flying cars");
    let start = model.do_start(&options).await.unwrap();
    assert_eq!(
        start.operation,
        json!({"operationName": "operations/my-op-123"})
    );
    let status = model.do_status(&start.operation, &options).await.unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0].url,
        "https://api.example.com/models/veo-2.0-generate-001:predictLongRunning"
    );
    assert_eq!(
        seen[0].json_body(),
        json!({"instances": [{"prompt": "A futuristic city with flying cars"}], "parameters": {"sampleCount": 1}})
    );
    assert_eq!(seen[1].method, "POST");
    assert_eq!(
        seen[1].url,
        "https://api.example.com/models/veo-2.0-generate-001:fetchPredictOperation"
    );
    assert_eq!(
        seen[1].json_body(),
        json!({"operationName": "operations/my-op-123"})
    );
    let VideoOperationStatus::Completed(result) = status else {
        panic!("expected completed video")
    };
    assert!(
        matches!(&result.videos[..], [VideoData::Base64 { data, media_type }] if data == "base64-video-data" && media_type == "video/mp4")
    );
    let metadata = json!({"videos": [{"mimeType": "video/mp4"}]});
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({"googleVertex": metadata, "vertex": metadata, "google-vertex": metadata})
    );
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("veo-2.0-generate-001")
    );
}

/// TS: "should throw when no operation name returned" (@ai-sdk/google-vertex/src/google-vertex-video-model.test.ts)
#[tokio::test]
async fn video_missing_operation_name_is_an_error() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({"done": false}))]);
    let error = provider(&fetch)
        .video("veo-2.0-generate-001")
        .do_start(&VideoCallOptions::new("A futuristic city with flying cars"))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("No operation name returned from API")
    );
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].json_body(),
        json!({"instances": [{"prompt": "A futuristic city with flying cars"}], "parameters": {"sampleCount": 1}})
    );
}

/// TS: "transcribes audio via Vertex generateContent with audioTranscriptionConfig"
/// (google-vertex/src/gemini-transcription/google-vertex-gemini-transcription-model.test.ts)
#[tokio::test]
async fn gemini_transcription_generate_request_and_result() {
    let usage = json!({"promptTokenCount": 10, "candidatesTokenCount": 4});
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "candidates": [{"content": {"parts": [{"text": "Hello "}, {"text": "world."}]}}],
        "usageMetadata": usage,
    }))]);
    let provider = create_google_vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("test-oauth-token".into())),
        project: Some("test-project".into()),
        location: Some("us-central1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options =
        TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3, 4]), "audio/wav");
    options.provider_options = Some(provider_namespace(
        "googleVertex",
        json!({
            "customVocabulary": ["Gemini", "Kubernetes"], "languageCodes": ["es-ES"], "mode": "SMART",
        }),
    ).expect("provider metadata must be an object"));
    let result = provider
        .transcription("gemini-3.5-transcribe")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google/models/gemini-3.5-transcribe:generateContent"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer test-oauth-token");
    assert_eq!(
        seen[0].json_body(),
        json!({
            "contents": [{"role": "user", "parts": [{"inlineData": {"mimeType": "audio/wav", "data": "AQIDBA=="}}]}],
            "generationConfig": {"audioTranscriptionConfig": {"languageCodes": ["es-ES"], "customVocabulary": ["Gemini", "Kubernetes"], "mode": "SMART"}},
        })
    );
    assert_eq!(result.text, "Hello world.");
    assert_eq!(
        serde_json::to_value(result.provider_metadata.unwrap()).unwrap(),
        json!({"google": {"usageMetadata": usage}})
    );
}
