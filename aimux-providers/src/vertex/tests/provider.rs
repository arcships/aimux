//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/google-vertex-provider.test.ts`.
//!
//! Upstream's Node provider wraps the base provider: it mocks
//! `createAuthTokenGenerator` and the base `createGoogleVertex` and checks the
//! options the wrapper passes on. aimux has one provider, so the tests look at
//! the same values inside it: the provider headers its resolver produces for a
//! request (upstream's `passedOptions.headers`), the options of the Google
//! credential resolver it built (upstream's `createAuthTokenGenerator`
//! argument), and an `access_token` producer standing in for the mocked token
//! generator. Every test clears `GOOGLE_VERTEX_API_KEY` (upstream's
//! `beforeEach`) and `GOOGLE_VERTEX_ACCESS_TOKEN` (read before the generator),
//! so all of them are serial.

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch};
use crate::vertex::{
    GoogleAuthOptions, GoogleAuthScopes, VertexAnthropicProviderSettings,
    VertexPartnerProviderSettings, VertexProvider, VertexProviderSettings, create_google_vertex,
    create_google_vertex_anthropic, create_google_vertex_maas, create_google_vertex_xai,
};
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::result::GenerateContent;
use aimux_core::shared::{AspectRatio, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};
use aimux_core::types::FinishReasonUnified;
use aimux_core::video_model::{VideoCallOptions, VideoData, VideoModel, VideoOperationStatus};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use futures::{FutureExt, StreamExt};
use serde_json::json;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const TEST_RSA_KEY: &str = include_str!("fixtures/aimux-test-rsa-key.pem");

fn clean_environment() -> Vec<EnvVar> {
    ["GOOGLE_VERTEX_API_KEY", "GOOGLE_VERTEX_ACCESS_TOKEN"]
        .into_iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

/// `vi.fn().mockResolvedValue('mock-auth-token')`, counting its calls.
fn mock_auth_token(calls: &Arc<AtomicUsize>) -> Resolvable<String> {
    let calls = calls.clone();
    Resolvable::AsyncFn(Arc::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok("mock-auth-token".to_string()) }.boxed()
    }))
}

fn create(settings: VertexProviderSettings) -> VertexProvider {
    create_google_vertex(settings).expect("valid settings")
}

/// `await resolve(passedOptions.headers)`: the provider headers of one
/// request, in the mode the provider was created in.
async fn provider_headers(provider: &VertexProvider) -> HeaderMapOpt {
    let key = provider.resolver.express_key().await.expect("api key");
    provider
        .resolver
        .headers(key.as_deref())
        .await
        .expect("headers")
}

/// Expected provider headers. aimux combines header maps with lower-cased
/// names (HTTP names are case-insensitive), so the names are lower-cased.
fn headers(entries: &[(&str, &str)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), Some((*value).to_string())))
        .collect()
}

/// The options of the provider's Google credential resolver
/// (`createAuthTokenGenerator`'s argument).
fn auth_options(provider: &VertexProvider) -> &GoogleAuthOptions {
    &provider.resolver.auth.options
}

fn scopes(options: &GoogleAuthOptions) -> Option<Vec<String>> {
    options.scopes.as_ref().map(|scopes| match scopes {
        GoogleAuthScopes::String(scope) => vec![scope.clone()],
        GoogleAuthScopes::Array(scopes) => scopes.clone(),
    })
}

mod google_vertex_provider {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › default headers function should return auth token
    #[tokio::test]
    #[serial]
    async fn default_headers_function_should_return_auth_token() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            access_token: Some(mock_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider_headers(&provider).await,
            headers(&[("Authorization", "Bearer mock-auth-token")])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › should use custom headers in addition to auth token when provided
    #[tokio::test]
    #[serial]
    async fn should_use_custom_headers_in_addition_to_auth_token_when_provided() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            headers: Some(Resolvable::AsyncFn(Arc::new(|| {
                async { Ok(headers(&[("Custom-Header", "custom-value")])) }.boxed()
            }))),
            access_token: Some(mock_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider_headers(&provider).await,
            headers(&[
                ("Authorization", "Bearer mock-auth-token"),
                ("Custom-Header", "custom-value"),
            ])
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › passes googleAuthOptions to createAuthTokenGenerator
    #[tokio::test]
    #[serial]
    async fn passes_google_auth_options_to_create_auth_token_generator() {
        let _env = clean_environment();
        // aimux builds the credential resolver with the provider, so there
        // is no headers call to trigger it (the key file does not exist).
        let provider = create(VertexProviderSettings {
            google_auth_options: Some(GoogleAuthOptions {
                scopes: Some(GoogleAuthScopes::Array(vec![
                    "https://www.googleapis.com/auth/cloud-platform".into(),
                ])),
                key_file: Some("path/to/key.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(
            scopes(options),
            Some(vec![
                "https://www.googleapis.com/auth/cloud-platform".to_string()
            ])
        );
        assert_eq!(options.key_file, Some(PathBuf::from("path/to/key.json")));
        assert_eq!(options.project_id, None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › passes project to createAuthTokenGenerator as projectId
    #[tokio::test]
    #[serial]
    async fn passes_project_to_create_auth_token_generator_as_project_id() {
        let _env = clean_environment();
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(options.project_id.as_deref(), Some("test-project"));
        assert_eq!(scopes(options), None);
        assert_eq!(options.key_file, None);
        assert!(options.credentials.is_none());
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › does not override explicit googleAuthOptions projectId
    #[tokio::test]
    #[serial]
    async fn does_not_override_explicit_google_auth_options_project_id() {
        let _env = clean_environment();
        let provider = create(VertexProviderSettings {
            project: Some("provider-project".into()),
            google_auth_options: Some(GoogleAuthOptions {
                project_id: Some("auth-project".into()),
                key_file: Some("path/to/key.json".into()),
                ..Default::default()
            }),
            ..Default::default()
        });

        let options = auth_options(&provider);
        assert_eq!(options.project_id.as_deref(), Some("auth-project"));
        assert_eq!(options.key_file, Some(PathBuf::from("path/to/key.json")));
        assert_eq!(scopes(options), None);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › should pass options through to base provider when apiKey is provided
    #[tokio::test]
    #[serial]
    async fn should_pass_options_through_to_base_provider_when_api_key_is_provided() {
        let _env = clean_environment();
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = create(VertexProviderSettings {
            api_key: Some(Resolvable::Value("test-api-key".into())),
            access_token: Some(mock_auth_token(&calls)),
            ..Default::default()
        });

        assert_eq!(
            provider.resolver.express_key().await.unwrap().as_deref(),
            Some("test-api-key")
        );
        // No auth-token headers: only the Express-mode key.
        let headers = provider_headers(&provider).await;
        assert!(
            !headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization")),
            "{headers:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider.test.ts › google-vertex-provider › creates the auth token generator once per provider instance
    #[tokio::test]
    #[serial]
    async fn creates_the_auth_token_generator_once_per_provider_instance() {
        let _env = clean_environment();
        // One generator per provider: its service-account token is fetched
        // once and reused by the second headers call.
        let fetch = MockFetch::new(vec![Canned::json(
            &json!({"access_token": "sa-token", "expires_in": 3600}),
        )]);
        let provider = create(VertexProviderSettings {
            project: Some("test-project".into()),
            google_auth_options: Some(GoogleAuthOptions {
                credentials: Some(json!({
                    "type": "service_account",
                    "client_email": "test@test.iam.gserviceaccount.com",
                    "private_key": TEST_RSA_KEY,
                })),
                ..Default::default()
            }),
            fetch: Some(fetch.transport()),
            ..Default::default()
        });

        let first = provider_headers(&provider).await;
        let second = provider_headers(&provider).await;

        assert_eq!(first, headers(&[("Authorization", "Bearer sa-token")]));
        assert_eq!(second, first);
        assert_eq!(fetch.seen().len(), 1);
    }
}

// ── Older ports (previously `tests/vertex_upstream_e2e_test.rs`) ──
//
// In-process end-to-end subset of the pinned Vertex upstream ports.

fn provider(fetch: &Arc<MockFetch>) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        base_url: Some("https://api.example.com".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// partial: `should preserve local JSON Schema references in tool requests`; full port in src/vertex/tests/language_model.rs.
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

/// partial: `should pass the model parameters correctly`; full port in src/vertex/tests/embedding_model.rs.
/// partial: `should extract embeddings`; full port in src/vertex/tests/embedding_model.rs.
/// partial: `should extract usage`; full port in src/vertex/tests/embedding_model.rs.
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

/// partial: `should send response modalities, aspect ratio, seed, and headers`; full port in src/vertex/tests/image_model.rs.
/// partial: `should use the language model endpoint and extract generated images`; full port in src/vertex/tests/image_model.rs.
/// partial: `should include usage and response metadata`; full port in src/vertex/tests/image_model.rs.
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

/// partial: `should send the model, languageCodes, features and base64 content`; full port in src/vertex/tests/transcription_model.rs.
/// partial: `should extract text, segments, language and duration`; full port in src/vertex/tests/transcription_model.rs.
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
            {"text": "hello", "startSecond": 0.0, "endSecond": 0.5},
            {"text": "world", "startSecond": 0.5, "endSecond": 1.0}
        ])
    );
    assert_eq!(result.language.as_deref(), Some("en"));
    assert_eq!(result.duration_in_seconds, Some(1.0));
}

/// partial: `should pass correct request body`; full port in src/vertex/tests/video_model.rs.
/// partial: `should return operation with operationName`; full port in src/vertex/tests/video_model.rs.
/// partial: `should return completed with video data when done`; full port in src/vertex/tests/video_model.rs.
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

/// partial: `should throw when no operation name returned`; full port in src/vertex/tests/video_model.rs.
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

/// partial: `transcribes audio via Vertex generateContent with audioTranscriptionConfig`; full port in src/vertex/tests/gemini_transcription/gemini_transcription_model.rs.
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
    use crate::vertex::GoogleAuthOptions;

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
        matches!(&error, AiMuxError::InvalidArgument { message, .. }
            if message.contains("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES=1")),
        "{error:?}"
    );
    assert!(!marker.exists(), "the credential executable was spawned");
    assert!(fetch.seen().is_empty(), "a request was sent");
}

/// partial: `should accept the legacy vertex options key and prefer googleVertex`; full port in src/vertex/tests/image_model.rs.
/// Upstream: @ai-sdk/google-vertex 5.0.100 src/google-vertex-image-model.ts
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
) -> VertexPartnerProviderSettings {
    VertexPartnerProviderSettings {
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

/// partial: `should create a provider with correct base URL for regional location`, `should default to global location when not specified`, `should default Llama 4 max tokens without overriding explicit or other model settings`, `should generate auth token and add to request headers`; full ports in src/vertex/tests/maas/maas_provider.rs and src/vertex/tests/maas/maas_provider_node.rs.
/// Upstream: @ai-sdk/google-vertex 5.0.100 src/maas/google-vertex-maas-provider.ts:79-130
/// (the project/location URL, `global` by default, the `vertex.maas` name,
/// the llama-4 `max_tokens` default at :14-25) and
/// google-vertex-maas-provider-node.ts:34-58 (the headers, then the bearer token).
#[tokio::test]
#[serial]
async fn maas_sub_provider_builds_on_openai_compatible() {
    let fetch = MockFetch::new(vec![
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
    ]);
    let regional =
        create_google_vertex_maas(partner_settings(&fetch, Some("us-central1"))).unwrap();
    let model = regional.chat("meta/llama-4-scout-17b-16e-instruct-maas");
    assert_eq!(model.provider(), "vertex.maas.chat");
    model.do_generate(&prompt()).await.unwrap();
    let global = create_google_vertex_maas(partner_settings(&fetch, None)).unwrap();
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

/// partial: `should create a provider with correct base URL for regional location`, `should strip reasoning_effort from request bodies`, `should count Grok reasoning tokens separately from completion tokens`, `should support HTTP image URLs`, `should throw NoSuchModelError for embedding and image models`; full ports in src/vertex/tests/xai/xai_provider.rs.
/// Upstream: @ai-sdk/google-vertex 5.0.100 src/xai/google-vertex-xai-provider.ts:
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
    let provider = create_google_vertex_xai(partner_settings(&fetch, Some("us-central1"))).unwrap();
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
            .await
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
/// (google-vertex-maas-provider.ts:95-101): no project, no request.
#[tokio::test]
async fn partner_sub_providers_require_a_project() {
    assert!(
        std::env::var_os("GOOGLE_VERTEX_PROJECT").is_none(),
        "the test needs GOOGLE_VERTEX_PROJECT unset"
    );
    let fetch = MockFetch::new(vec![]);
    let settings = VertexPartnerProviderSettings {
        project: None,
        ..partner_settings(&fetch, None)
    };
    let error = create_google_vertex_maas(settings)
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

/// partial: `should merge custom headers with auth header`, `should preserve headers from init parameter`; full ports in src/vertex/tests/maas/maas_provider_node.rs (the first also in src/vertex/tests/xai/xai_provider_node.rs).
/// Upstream: @ai-sdk/google-vertex 5.0.100 maas/google-vertex-maas-provider-node.ts:34-58
/// and xai/google-vertex-xai-provider-node.ts:33-55 hand no headers to
/// `createOpenAICompatible`; their fetch spreads the provider headers, then
/// `Authorization`, over the request's headers. So a provider header and the
/// bearer token win over a per-call header of the same name, a per-call header
/// of another name is kept, and the user-agent is the request's own.
#[tokio::test]
#[serial]
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
    create_google_vertex_maas(partner_settings(&fetch, None))
        .unwrap()
        .chat("meta/llama-3.3-70b-instruct-maas")
        .do_generate(&options)
        .await
        .unwrap();
    create_google_vertex_xai(partner_settings(&fetch, None))
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
            "ai-sdk-openai-compatible/3.0.61 ai-sdk-provider-utils/5.0.53",
            "{}",
            seen.url
        );
    }
}

/// Upstream: the Node wrappers' fetch spreads the provider headers over
/// `init.headers` after `postToApi` built the user-agent
/// (google-vertex-maas-provider-node.ts:43-49,
/// google-vertex-xai-provider-node.ts:41-47; provider-utils
/// post-to-api.ts:100-104). A provider-level `user-agent` therefore replaces
/// the whole header: no package part, no `ai-sdk-provider-utils` part, and
/// not the per-call one.
#[tokio::test]
#[serial]
async fn partner_user_agent_header_replaces_the_whole_user_agent() {
    let fetch = MockFetch::new(vec![
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
        partner_response(json!({"prompt_tokens": 1, "completion_tokens": 1})),
    ]);
    let settings = || {
        let mut settings = partner_settings(&fetch, None);
        settings.headers = Some(Resolvable::Value(
            [("User-Agent".into(), Some("my-agent/1.0 (vertex)".into()))].into(),
        ));
        settings
    };
    let mut options = prompt();
    options.headers = Some([("user-agent".to_string(), "call-agent/2.0".to_string())].into());
    create_google_vertex_maas(settings())
        .unwrap()
        .chat("meta/llama-3.3-70b-instruct-maas")
        .do_generate(&options)
        .await
        .unwrap();
    create_google_vertex_xai(settings())
        .unwrap()
        .chat("xai/grok-4.1-fast-reasoning")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 2);
    for seen in seen {
        assert_eq!(
            seen.headers["user-agent"], "my-agent/1.0 (vertex)",
            "{}",
            seen.url
        );
        assert_eq!(seen.headers["authorization"], "Bearer test-token");
    }
}

/// Upstream: @ai-sdk/google-vertex 5.0.100 src/google-vertex-provider-base.ts:283-289
/// appends `ai-sdk-google-vertex/<v>` to the Gemini models' headers; the
/// Anthropic sub-provider hands the caller's headers to the Anthropic model
/// as they are (anthropic/google-vertex-anthropic-provider.ts:216-220), so
/// only the helper's `ai-sdk-provider-utils/<v>` part is sent.
#[tokio::test]
async fn only_gemini_models_send_the_google_vertex_user_agent_part() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))]);
    let anthropic = create_google_vertex_anthropic(VertexAnthropicProviderSettings {
        project: Some("test-project".into()),
        location: Some("us-east5".into()),
        fetch: Some(fetch.transport()),
        generate_auth_token: Some(Resolvable::Value(Some("test-token".into()))),
        ..Default::default()
    })
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

/// `id-0`, `id-1`, ... in call order, like the AI SDK tests' `generateId` mock.
fn counting_provider(fetch: &Arc<MockFetch>) -> VertexProvider {
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        base_url: Some("https://api.example.com".into()),
        fetch: Some(fetch.transport()),
        generate_id: Some(Arc::new(move || {
            format!(
                "id-{}",
                next.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            )
        })),
        ..Default::default()
    })
    .unwrap()
}

fn sse(chunks: &[serde_json::Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}

/// Upstream: the Vertex chat model is `GoogleLanguageModel`
/// (@ai-sdk/google-vertex@5.0.98 src/google-vertex-provider-base.ts:297-299), so ids
/// come from `generateId` (@ai-sdk/google@4.0.87 src/google-language-model.ts:506,
/// :576, :598, :553, extractSources :1457) and `serviceTier` is always written (:651).
#[tokio::test]
async fn gemini_generate_takes_ids_and_metadata_from_the_google_model() {
    let response = json!({
        "candidates": [{
            "content": {"role": "model", "parts": [
                {"executableCode": {"language": "PYTHON", "code": "print(1)"}},
                {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "1"}},
                {"toolCall": {"toolType": "GOOGLE_SEARCH_WEB", "args": {"q": "x"}}},
                {"toolResponse": {"toolType": "GOOGLE_SEARCH_WEB", "response": {"ok": true}}},
                {"functionCall": {"name": "weather", "args": {"city": "Paris"}}, "thoughtSignature": ""}
            ]},
            "finishReason": "STOP",
            "groundingMetadata": {"groundingChunks": [{"web": {"uri": "https://example.com", "title": "Example"}}]}
        }],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}
    });
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let result = counting_provider(&fetch)
        .chat("gemini-2.5-flash")
        .do_generate(&prompt())
        .await
        .unwrap();
    let content = serde_json::to_value(&result.content).unwrap();
    let ids: Vec<_> = content
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part.get("toolCallId").or_else(|| part.get("id")).unwrap())
        .collect();
    assert_eq!(ids, ["id-0", "id-0", "id-1", "id-1", "id-2", "id-3"]);
    // Code execution carries no server-tool metadata; an empty signature none.
    assert!(content[0].get("providerMetadata").is_none(), "{content}");
    assert!(content[1].get("providerMetadata").is_none(), "{content}");
    assert!(content[4].get("providerMetadata").is_none(), "{content}");
    assert_eq!(
        content[2]["providerMetadata"]["googleVertex"]["serverToolCallId"],
        "id-1"
    );
    let metadata = serde_json::to_value(result.provider_metadata.unwrap()).unwrap();
    assert_eq!(metadata["googleVertex"]["serviceTier"], json!(null));
    assert_eq!(metadata["vertex"], metadata["googleVertex"]);
}

/// Upstream: block ids count only text and reasoning blocks
/// (@ai-sdk/google@4.0.87 src/google-language-model.ts:977, :1001); code
/// execution and sources take `generateId` (:916, extractSources); usage is
/// read from the parsed chunk (:849).
#[tokio::test]
async fn gemini_stream_numbers_only_text_blocks_and_takes_ids_from_generate_id() {
    let fetch = MockFetch::new(vec![sse(&[
        json!({"candidates": [{"content": {"role": "model", "parts": [
            {"executableCode": {"language": "PYTHON", "code": "print(1)"}},
            {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "1"}},
            {"text": "a"}
        ]}}]}),
        json!({"candidates": [{"content": {"role": "model", "parts": [
            {"toolCall": {"toolType": "GOOGLE_SEARCH_WEB", "args": {}}},
            {"toolResponse": {"toolType": "GOOGLE_SEARCH_WEB", "response": {}}},
            {"text": "r", "thought": true},
            {"text": "b"}
        ]}, "finishReason": "STOP",
        "groundingMetadata": {"groundingChunks": [{"web": {"uri": "https://example.com", "title": "Example"}}]}}],
        "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 2, "totalTokenCount": 5}}),
    ])]);
    let result = counting_provider(&fetch)
        .chat("gemini-2.5-flash")
        .do_stream(&prompt())
        .await
        .unwrap();
    let mut code_execution = Vec::new();
    let mut blocks = Vec::new();
    let mut sources = Vec::new();
    let mut usage = None;
    for part in result.stream.collect::<Vec<_>>().await {
        match part.unwrap() {
            StreamPart::ToolCall(call) if call.dynamic.is_none() => {
                assert!(call.provider_metadata.is_none());
                code_execution.push(call.tool_call_id);
            }
            StreamPart::TextStart { id, .. } => blocks.push(format!("text {id}")),
            StreamPart::ReasoningStart { id, .. } => blocks.push(format!("reasoning {id}")),
            StreamPart::Source(aimux_core::result::Source::Url { id, .. }) => sources.push(id),
            StreamPart::Finish { usage: finish, .. } => usage = Some(finish),
            _ => {}
        }
    }
    assert_eq!(code_execution, ["id-0"]);
    assert_eq!(blocks, ["text 0", "reasoning 1", "text 2"]);
    // The source is emitted before the parts of its chunk: after the server tool call.
    assert_eq!(sources, ["id-1"]);
    let usage = usage.unwrap();
    assert_eq!(usage.input_tokens.total, Some(3));
    assert_eq!(usage.output_tokens.total, Some(2));
}

/// Upstream: request warnings are the option warnings, then the tool warnings
/// (@ai-sdk/google@4.0.87 src/google-language-model.ts:444), the reasoning mapping
/// included.
#[tokio::test]
async fn gemini_keeps_the_request_builder_warnings() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "candidates": [{"content": {"parts": [{"text": "ok"}], "role": "model"}, "finishReason": "STOP"}]
    }))]);
    let mut options = prompt();
    options.reasoning = Some(aimux_core::types::ReasoningEffort::Xhigh);
    let result = provider(&fetch)
        .chat("gemini-3-pro-preview")
        .do_generate(&options)
        .await
        .unwrap();
    assert!(
        result.warnings.iter().any(|warning| matches!(
            warning,
            aimux_core::types::Warning::Compatibility { feature, .. } if feature == "reasoning"
        )),
        "{:?}",
        result.warnings
    );
}

/// Upstream: Vertex AI streams function-call arguments when
/// `streamFunctionCallArguments` is set (@ai-sdk/google@4.0.87
/// src/google-language-model.ts:361-385), and the stream accumulates
/// `partialArgs` until the empty chunk that ends the call (:1099-1187).
#[tokio::test]
async fn gemini_stream_accumulates_streamed_function_call_arguments() {
    let call = |function_call: serde_json::Value| json!({"candidates": [{"content": {"role": "model", "parts": [{"functionCall": function_call}]}}]});
    let fetch = MockFetch::new(vec![sse(&[
        call(json!({"name": "weather", "willContinue": true})),
        call(
            json!({"partialArgs": [{"jsonPath": "$.city", "stringValue": "Par", "willContinue": true}], "willContinue": true}),
        ),
        call(
            json!({"partialArgs": [{"jsonPath": "$.city", "stringValue": "is"}], "willContinue": true}),
        ),
        call(json!({})),
        json!({"candidates": [{"content": {"role": "model", "parts": [{"text": ""}]}, "finishReason": "STOP"}]}),
    ])]);
    let mut options = prompt();
    options.provider_options = Some(
        provider_namespace("googleVertex", json!({"streamFunctionCallArguments": true})).unwrap(),
    );
    let result = counting_provider(&fetch)
        .chat("gemini-2.5-flash")
        .do_stream(&options)
        .await
        .unwrap();
    let parts: Vec<_> = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        fetch.seen()[0].json_body()["toolConfig"]["functionCallingConfig"]["streamFunctionCallArguments"],
        true
    );
    let calls: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1, "{parts:?}");
    assert_eq!(calls[0].tool_call_id, "id-0");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&calls[0].input).unwrap(),
        json!({"city": "Paris"})
    );
    let finish = parts.last().unwrap();
    assert!(
        matches!(finish, StreamPart::Finish { finish_reason, .. } if finish_reason.unified == FinishReasonUnified::ToolCalls),
        "{finish:?}"
    );
}
