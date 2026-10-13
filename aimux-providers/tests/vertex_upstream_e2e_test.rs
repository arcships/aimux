//! In-process end-to-end subset of the pinned Vertex upstream ports.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
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

/// Executable-sourced external-account credentials are kept (an opt-in, as in
/// google-auth-library, which `@ai-sdk/google-vertex` uses for credentials on
/// Node), registered in docs/aisdk-architecture-alignment.md §5. Without
/// `GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES=1` the request fails before
/// anything is spawned and before any HTTP request is sent.
#[tokio::test]
async fn executable_credential_without_the_opt_in_fails_and_spawns_nothing() {
    use aimux_providers::vertex::GoogleAuthOptions;

    for var in [
        "GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES",
        "GOOGLE_VERTEX_ACCESS_TOKEN",
    ] {
        assert!(
            std::env::var_os(var).is_none(),
            "this test needs {var} unset"
        );
    }
    let marker = std::env::temp_dir().join(format!(
        "aimux-vertex-executable-marker-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let credentials = json!({
        "type": "external_account",
        "audience": "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/pool/providers/exec",
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "token_url": "https://sts.googleapis.com/v1/token",
        "credential_source": {
            "executable": {
                "command": format!("touch {}", marker.display()),
                "timeout_millis": 5000
            }
        }
    });
    let fetch = MockFetch::new(Vec::new());
    let provider = create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value(String::new())),
        project: Some("test-project".into()),
        location: Some("us-central1".into()),
        google_auth_options: Some(GoogleAuthOptions {
            credentials: Some(credentials),
            ..Default::default()
        }),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    let error = provider
        .chat("gemini-2.5-flash")
        .do_generate(&CallOptions::new(vec![LanguageModelMessage::user_text(
            "Hello",
        )]))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument(message)
            if message.contains("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES=1")),
        "{error:?}"
    );
    assert!(!marker.exists(), "the credential executable was spawned");
    assert!(fetch.seen().is_empty(), "a request was sent");
}

/// Upstream: @ai-sdk/google-vertex 5.0.98 src/google-vertex-image-model.ts
/// reads the image options under `googleVertex`, then the legacy `vertex`,
/// hands them to the inner language model under both keys (:190-194) and
/// writes its metadata under both (:221-224).
#[tokio::test]
async fn image_options_under_the_legacy_vertex_key_are_read_and_metadata_written_back() {
    let response = json!({
        "candidates": [{"content": {"parts": [{"inlineData": {"mimeType": "image/png", "data": "image"}}], "role": "model"}, "finishReason": "STOP"}]
    });
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let provider = create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-key".into())),
        base_url: Some("https://api.example.com".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap();
    let mut options = ImageCallOptions::new("A cat");
    options.provider_options =
        provider_namespace("vertex", json!({"imageConfig": {"imageSize": "2K"}})).unwrap();
    let result = provider
        .image("gemini-2.5-flash-image")
        .do_generate(&options)
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].json_body()["generationConfig"],
        json!({"responseModalities": ["IMAGE"], "imageConfig": {"imageSize": "2K"}})
    );
    let metadata = result.provider_metadata.unwrap();
    assert_eq!(metadata["googleVertex"], metadata["vertex"]);
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

fn partner_response(usage: serde_json::Value) -> Canned {
    Canned::json(&json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": usage
    }))
}

fn partner_settings(
    fetch: &Arc<MockFetch>,
    location: Option<&str>,
) -> aimux_providers::VertexPartnerProviderSettings {
    aimux_providers::VertexPartnerProviderSettings {
        project: Some("test-project".into()),
        location: location.map(String::from),
        headers: Some(Resolvable::Value(
            [("Custom-Header".into(), Some("custom-value".into()))].into(),
        )),
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    }
}

/// Upstream: @ai-sdk/google-vertex 5.0.98 src/maas/google-vertex-maas-provider.ts:77-118
/// (the project/location URL, `global` by default, the `vertex.maas` name,
/// the llama-4 `max_tokens` default at :14-25) and
/// google-vertex-maas-provider-node.ts:34-58 (the headers, then the bearer token).
#[tokio::test]
async fn maas_sub_provider_builds_on_openai_compatible() {
    let fetch = MockFetch::new(vec![
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
    ]);
    let regional =
        aimux_providers::create_google_vertex_maas(partner_settings(&fetch, Some("us-central1")))
            .unwrap();
    let model = regional.chat("meta/llama-4-scout-17b-16e-instruct-maas");
    assert_eq!(model.provider(), "vertex.maas.chat");
    model.do_generate(&prompt()).await.unwrap();
    let global =
        aimux_providers::create_google_vertex_maas(partner_settings(&fetch, None)).unwrap();
    let mut options = prompt();
    options.max_output_tokens = Some(100);
    global
        .chat("meta/llama-4-scout-17b-16e-instruct-maas")
        .do_generate(&options)
        .await
        .unwrap();

    let seen = fetch.seen();
    assert_eq!(
        seen[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
    );
    assert_eq!(
        seen[1].url,
        "https://aiplatform.googleapis.com/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer test-token");
    assert_eq!(seen[0].headers["custom-header"], "custom-value");
    assert!(seen[0].headers["user-agent"].starts_with("ai-sdk-openai-compatible/"));
    assert_eq!(seen[0].json_body()["max_tokens"], 8192);
    assert_eq!(seen[1].json_body()["max_tokens"], 100);
}

/// Upstream: @ai-sdk/google-vertex 5.0.98 src/xai/google-vertex-xai-provider.ts:
/// the global host whatever the location (:159-164), the `googleVertex.xai`
/// name with usage in streams and structured outputs (:172-184), no
/// `reasoning_effort` (:131-134), reasoning tokens on top of
/// `completion_tokens` (:89-129) and no embedding or image models (:203-209).
#[tokio::test]
async fn xai_sub_provider_builds_on_openai_compatible() {
    let fetch = MockFetch::new(vec![partner_response(json!({
        "prompt_tokens": 10, "completion_tokens": 30,
        "prompt_tokens_details": {"cached_tokens": 4},
        "completion_tokens_details": {"reasoning_tokens": 12}
    }))]);
    let provider =
        aimux_providers::create_google_vertex_xai(partner_settings(&fetch, Some("us-central1")))
            .unwrap();
    let model = provider.chat("xai/grok-4.1-fast-reasoning");
    assert_eq!(model.provider(), "googleVertex.xai.chat");
    let mut options = prompt();
    options.reasoning = Some(aimux_core::types::ReasoningEffort::High);
    let result = model.do_generate(&options).await.unwrap();

    let seen = fetch.seen();
    assert_eq!(
        seen[0].url,
        "https://aiplatform.googleapis.com/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer test-token");
    assert!(seen[0].json_body().get("reasoning_effort").is_none());
    assert_eq!(result.usage.input_tokens.total, Some(10));
    assert_eq!(result.usage.input_tokens.no_cache, Some(6));
    assert_eq!(result.usage.input_tokens.cache_read, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(42));
    assert_eq!(result.usage.output_tokens.text, Some(30));
    assert_eq!(result.usage.output_tokens.reasoning, Some(12));
    assert!(
        model
            .supported_urls()
            .0
            .get("image/*")
            .is_some_and(|patterns| patterns[0].is_match("https://example.com/a.png"))
    );
    assert!(matches!(
        aimux_core::provider::Provider::embedding_model(&provider, "e"),
        Err(AiMuxError::NoSuchModel { .. })
    ));
}

/// Upstream: both sub-providers load the project with `loadSetting`
/// (google-vertex-maas-provider.ts:83-89): no project, no request.
#[tokio::test]
async fn partner_sub_providers_require_a_project() {
    assert!(
        std::env::var_os("GOOGLE_VERTEX_PROJECT").is_none(),
        "the test needs GOOGLE_VERTEX_PROJECT unset"
    );
    let fetch = MockFetch::new(vec![]);
    let settings = aimux_providers::VertexPartnerProviderSettings {
        project: None,
        ..partner_settings(&fetch, None)
    };
    let error = aimux_providers::create_google_vertex_maas(settings)
        .unwrap()
        .chat("m")
        .do_generate(&prompt())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::LoadSetting { env_var, .. } if env_var == "GOOGLE_VERTEX_PROJECT"),
        "{error:?}"
    );
    assert!(fetch.seen().is_empty());
}

/// Upstream: @ai-sdk/google-vertex 5.0.98 maas/google-vertex-maas-provider-node.ts:34-58
/// and xai/google-vertex-xai-provider-node.ts:33-55 hand no headers to
/// `createOpenAICompatible`; their fetch spreads the provider headers, then
/// `Authorization`, over the request's headers. So a provider header and the
/// bearer token win over a per-call header of the same name, a per-call header
/// of another name is kept, and the user-agent is the request's own.
#[tokio::test]
async fn partner_headers_win_over_per_call_headers() {
    let fetch = MockFetch::new(vec![
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
    ]);
    let mut options = prompt();
    options.headers = Some(
        [
            ("custom-header", "call-value"),
            ("authorization", "Bearer call-token"),
            ("x-call-only", "kept"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect(),
    );
    aimux_providers::create_google_vertex_maas(partner_settings(&fetch, None))
        .unwrap()
        .chat("meta/llama-3.3-70b-instruct-maas")
        .do_generate(&options)
        .await
        .unwrap();
    aimux_providers::create_google_vertex_xai(partner_settings(&fetch, None))
        .unwrap()
        .chat("xai/grok-4.1-fast-reasoning")
        .do_generate(&options)
        .await
        .unwrap();
    for seen in fetch.seen() {
        assert_eq!(
            seen.headers["custom-header"], "custom-value",
            "{}",
            seen.url
        );
        assert_eq!(
            seen.headers["authorization"], "Bearer test-token",
            "{}",
            seen.url
        );
        assert_eq!(seen.headers["x-call-only"], "kept", "{}", seen.url);
        assert_eq!(
            seen.headers["user-agent"],
            "ai-sdk-openai-compatible/3.0.59 ai-sdk-provider-utils/5.0.51",
            "{}",
            seen.url
        );
    }
}

/// Upstream: @ai-sdk/google-vertex 5.0.98 src/google-vertex-provider-base.ts:270-276
/// appends `ai-sdk-google-vertex/<v>` to the Gemini models' headers; the
/// Anthropic sub-provider hands the caller's headers to the Anthropic model
/// as they are (anthropic/google-vertex-anthropic-provider.ts:206-210), so
/// only the helper's `ai-sdk-provider-utils/<v>` part is sent.
#[tokio::test]
async fn only_gemini_models_send_the_google_vertex_user_agent_part() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let anthropic = aimux_providers::create_google_vertex_anthropic(
        aimux_providers::VertexAnthropicProviderSettings {
            project: Some("test-project".into()),
            location: Some("us-east5".into()),
            fetch: Some(fetch.transport()),
            generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
            ..Default::default()
        },
    )
    .unwrap();
    let mut options = prompt();
    options.max_output_tokens = Some(16);
    anthropic
        .call("claude-sonnet-4-5")
        .do_generate(&options)
        .await
        .unwrap();
    let agent = &fetch.seen()[0].headers["user-agent"];
    assert!(agent.starts_with("ai-sdk-provider-utils/"), "{agent}");
    assert!(!agent.contains("ai-sdk-google-vertex/"), "{agent}");

    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "candidates": [{"content": {"parts": [{"text": "ok"}], "role": "model"}, "finishReason": "STOP"}]
    }))]);
    provider(&fetch)
        .chat("gemini-2.5-flash")
        .do_generate(&prompt())
        .await
        .unwrap();
    let agent = &fetch.seen()[0].headers["user-agent"];
    assert!(agent.starts_with("ai-sdk-google-vertex/"), "{agent}");
}
