//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/google-vertex-provider-base.test.ts`.
//!
//! Upstream replaces every model class with a `vi.fn()` and checks the
//! constructor arguments (`provider`, `baseURL`, `headers`, `generateId`).
//! aimux builds the model configuration privately, so these tests observe the
//! same values on the model and on the request it sends: `provider()` is the
//! `provider` argument, the request URL starts with `baseURL`, the
//! `Authorization` header of an injected access token shows the `headers`
//! function, and a tool-call id shows a custom `generateId`. Upstream's
//! `beforeEach` deletes `GOOGLE_VERTEX_API_KEY`; every test does the same
//! through an [`EnvVar`] guard, so all of them are serial.

use std::sync::Arc;

use serde_json::json;
use serial_test::serial;

use crate::test_support::mock_fetch::{Canned, EnvVar, MockFetch, Seen};
use crate::vertex::{VertexProvider, VertexProviderSettings, create_google_vertex};
use aimux_core::AiMuxError;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::GenerateContent;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};
use aimux_core::video_model::{VideoCallOptions, VideoModel};
use aimux_provider_utils::Resolvable;

/// `https://test-location-aiplatform.googleapis.com/…/publishers/google`.
const TEST_LOCATION_BASE_URL: &str = "https://test-location-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/test-location/publishers/google";
const GLOBAL_BASE_URL: &str = "https://aiplatform.googleapis.com/v1beta1/projects/test-project/locations/global/publishers/google";
const CUSTOM_BASE_URL: &str = "https://custom-endpoint.example.com";

/// The upstream `beforeEach`/`afterEach`: no `GOOGLE_VERTEX_API_KEY`.
fn clean_environment() -> EnvVar {
    EnvVar::set("GOOGLE_VERTEX_API_KEY", None)
}

/// `createGoogleVertex({ project, location, ...})` with a fixed access token
/// (upstream's base provider takes the auth headers from its caller) and the
/// mock transport.
fn settings(location: &str, fetch: &Arc<MockFetch>) -> VertexProviderSettings {
    VertexProviderSettings {
        project: Some("test-project".into()),
        location: Some(location.into()),
        access_token: Some("test-token".to_string().into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

/// `createGoogleVertex({ apiKey: 'test-api-key' })` with the mock transport.
fn express_settings(fetch: &Arc<MockFetch>) -> VertexProviderSettings {
    VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    }
}

fn provider(settings: VertexProviderSettings) -> VertexProvider {
    create_google_vertex(settings).expect("valid settings")
}

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// A Gemini response whose only part is a function call without an id, so
/// the model takes the id from `generateId`.
fn function_call_response() -> Canned {
    Canned::json(&json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{"functionCall": {"name": "weather", "args": {"city": "Paris"}}}]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}
    }))
}

/// The first request the transport saw.
fn only_request(fetch: &MockFetch) -> Seen {
    let seen = fetch.seen();
    assert!(!seen.is_empty(), "no request was sent");
    seen[0].clone()
}

/// The request of a `doGenerate` call of a language model.
async fn chat_request(model: &dyn LanguageModel, fetch: &MockFetch) -> Seen {
    let _ = model.do_generate(&prompt()).await;
    only_request(fetch)
}

/// `expect.objectContaining({ baseURL })`: the request goes below `base_url`.
fn assert_base_url(seen: &Seen, base_url: &str) {
    assert!(
        seen.url.starts_with(&format!("{base_url}/")),
        "{} is not below {base_url}",
        seen.url
    );
}

/// `headers: expect.any(Function)`: the provider headers reached the request.
fn assert_provider_headers(seen: &Seen) {
    assert_eq!(
        seen.headers.get("authorization").map(String::as_str),
        Some("Bearer test-token"),
        "{:?}",
        seen.headers
    );
}

mod google_vertex_provider_base {
    use super::*;

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a language model with default settings
    #[tokio::test]
    #[serial]
    async fn should_create_a_language_model_with_default_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).call("test-model-id");

        assert_eq!(model.model_id(), "test-model-id");
        assert_eq!(model.provider(), "google.vertex.chat");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.url,
            format!("{TEST_LOCATION_BASE_URL}/models/test-model-id:generateContent")
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create an embedding model with correct settings
    #[tokio::test]
    #[serial]
    async fn should_create_an_embedding_model_with_correct_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).embedding("test-embedding-model");

        assert_eq!(model.model_id(), "test-embedding-model");
        assert_eq!(model.provider(), "google.vertex.embedding");
        let _ = model.do_embed(&EmbeddingCallOptions::new("Hello")).await;
        let seen = only_request(&fetch);
        assert!(
            seen.url.starts_with(&format!(
                "{TEST_LOCATION_BASE_URL}/models/test-embedding-model:"
            )),
            "{}",
            seen.url
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a speech model with correct settings
    #[tokio::test]
    #[serial]
    async fn should_create_a_speech_model_with_correct_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch))
            .speech("gemini-2.5-flash-tts")
            .expect("a Gemini speech model");

        assert_eq!(model.model_id(), "gemini-2.5-flash-tts");
        assert_eq!(model.provider(), "google.vertex.speech");
        let _ = model.do_generate(&SpeechCallOptions::new("Hello")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, TEST_LOCATION_BASE_URL);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a speech model via speechModel()
    #[tokio::test]
    #[serial]
    async fn should_create_a_speech_model_via_speech_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch))
            .speech_model("gemini-2.5-pro-tts")
            .expect("the provider has speech models")
            .expect("a Gemini speech model");

        assert_eq!(model.model_id(), "gemini-2.5-pro-tts");
        assert_eq!(model.provider(), "google.vertex.speech");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should keep routing gemini speech models to the Gemini speech model
    #[tokio::test]
    #[serial]
    async fn should_keep_routing_gemini_speech_models_to_the_gemini_speech_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch))
            .speech_model("gemini-2.5-flash-tts")
            .expect("the provider has speech models")
            .expect("a Gemini speech model");

        assert_eq!(model.model_id(), "gemini-2.5-flash-tts");
        assert_eq!(model.provider(), "google.vertex.speech");
        // Not the Cloud Text-to-Speech model: the request is a Gemini
        // `generateContent` call below the Gemini publisher URL.
        let _ = model.do_generate(&SpeechCallOptions::new("Hello")).await;
        let seen = only_request(&fetch);
        assert_eq!(
            seen.url,
            format!("{TEST_LOCATION_BASE_URL}/models/gemini-2.5-flash-tts:generateContent")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a transcription model with correct settings
    #[tokio::test]
    #[serial]
    async fn should_create_a_transcription_model_with_correct_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("us-central1", &fetch)).transcription("chirp_2");

        assert_eq!(model.model_id(), "chirp_2");
        assert_eq!(model.provider(), "google.vertex.transcription");
        let options = TranscriptionCallOptions::new(AudioInput::Binary(vec![0, 1, 2]), "audio/wav");
        let _ = model.do_generate(&options).await;
        let seen = only_request(&fetch);
        // `project: 'test-project'`, `location: 'us-central1'`.
        assert!(
            seen.url
                .contains("/projects/test-project/locations/us-central1/"),
            "{}",
            seen.url
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a transcription model via transcriptionModel()
    #[tokio::test]
    #[serial]
    async fn should_create_a_transcription_model_via_transcription_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("us-central1", &fetch))
            .transcription_model("chirp_3")
            .expect("the provider has transcription models")
            .expect("a transcription model");

        assert_eq!(model.model_id(), "chirp_3");
        assert_eq!(model.provider(), "google.vertex.transcription");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should reject Express Mode for transcription models
    #[tokio::test]
    #[serial]
    async fn should_reject_express_mode_for_transcription_models() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(express_settings(&fetch)).transcription("chirp_3");

        // Upstream throws when the model is created; aimux fails its first
        // request, before anything is sent.
        let options = TranscriptionCallOptions::new(AudioInput::Binary(vec![0, 1, 2]), "audio/wav");
        let error = model.do_generate(&options).await.unwrap_err();
        assert!(
            error.to_string().contains(
                "Google Vertex transcription models do not support Express Mode API keys. Use standard Google Cloud credentials instead."
            ),
            "{error}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should pass custom headers to the model constructor
    #[tokio::test]
    #[serial]
    async fn should_pass_custom_headers_to_the_model_constructor() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(VertexProviderSettings {
            headers: Some(Resolvable::Value(
                [(
                    "Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                )]
                .into_iter()
                .collect(),
            )),
            ..settings("test-location", &fetch)
        })
        .call("test-model-id");

        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.headers.get("custom-header").map(String::as_str),
            Some("custom-value")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should pass custom generateId function to the model constructor
    #[tokio::test]
    #[serial]
    async fn should_pass_custom_generate_id_function_to_the_model_constructor() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![function_call_response()]);
        let model = provider(VertexProviderSettings {
            generate_id: Some(Arc::new(|| "custom-id".to_string())),
            ..settings("test-location", &fetch)
        })
        .call("test-model-id");

        let result = model.do_generate(&prompt()).await.expect("generated");
        let ids: Vec<&str> = result
            .content
            .iter()
            .filter_map(|content| match content {
                GenerateContent::ToolCall(call) => Some(call.tool_call_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["custom-id"]);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use languageModel method to create a model
    #[tokio::test]
    #[serial]
    async fn should_use_language_model_method_to_create_a_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch))
            .language_model("test-model-id")
            .expect("a language model");

        assert_eq!(model.model_id(), "test-model-id");
        assert_eq!(model.provider(), "google.vertex.chat");
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use custom baseURL when provided
    #[tokio::test]
    #[serial]
    async fn should_use_custom_base_url_when_provided() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(VertexProviderSettings {
            base_url: Some(CUSTOM_BASE_URL.into()),
            ..settings("test-location", &fetch)
        })
        .call("test-model-id");

        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_base_url(&seen, CUSTOM_BASE_URL);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use a tuned model endpoints/ id with no publishers/google suffix
    #[tokio::test]
    #[serial]
    async fn should_use_a_tuned_model_endpoints_id_with_no_publishers_google_suffix() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).call("endpoints/1234567890");

        assert_eq!(model.model_id(), "endpoints/1234567890");
        assert_eq!(model.provider(), "google.vertex.chat");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.url,
            "https://test-location-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/test-location/endpoints/1234567890:generateContent"
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should leave base model ids and the publishers/google base URL unchanged
    #[tokio::test]
    #[serial]
    async fn should_leave_base_model_ids_and_the_publishers_google_base_url_unchanged() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).call("gemini-2.5-pro");

        assert_eq!(model.model_id(), "gemini-2.5-pro");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.url,
            format!("{TEST_LOCATION_BASE_URL}/models/gemini-2.5-pro:generateContent")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should respect a custom baseURL for tuned model endpoints/ ids
    #[tokio::test]
    #[serial]
    async fn should_respect_a_custom_base_url_for_tuned_model_endpoints_ids() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(VertexProviderSettings {
            base_url: Some(CUSTOM_BASE_URL.into()),
            ..settings("test-location", &fetch)
        })
        .call("endpoints/1234567890");

        assert_eq!(model.model_id(), "endpoints/1234567890");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.url,
            format!("{CUSTOM_BASE_URL}/endpoints/1234567890:generateContent")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should reject Express Mode for tuned models
    #[tokio::test]
    #[serial]
    async fn should_reject_express_mode_for_tuned_models() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(express_settings(&fetch)).call("endpoints/1234567890");

        // Upstream throws when the model is created; aimux fails its first
        // request, before anything is sent.
        let error = model.do_generate(&prompt()).await.unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument(_)),
            "{error:?}"
        );
        assert!(
            error.to_string().contains(
                "Google Vertex tuned models do not support Express Mode API keys. Use standard Google Cloud credentials instead."
            ),
            "{error}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create an image model with default settings
    #[tokio::test]
    #[serial]
    async fn should_create_an_image_model_with_default_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).image("gemini-2.5-flash-image");

        assert_eq!(model.model_id(), "gemini-2.5-flash-image");
        assert_eq!(model.provider(), "google.vertex.image");
        let _ = model.do_generate(&ImageCallOptions::new("A cat")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, TEST_LOCATION_BASE_URL);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use correct URL for global region
    #[tokio::test]
    #[serial]
    async fn should_use_correct_url_for_global_region() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("global", &fetch)).call("test-model-id");

        assert_eq!(model.provider(), "google.vertex.chat");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_eq!(
            seen.url,
            format!("{GLOBAL_BASE_URL}/models/test-model-id:generateContent")
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use correct URL for global region with embedding model
    #[tokio::test]
    #[serial]
    async fn should_use_correct_url_for_global_region_with_embedding_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("global", &fetch)).embedding("test-embedding-model");

        assert_eq!(model.provider(), "google.vertex.embedding");
        let _ = model.do_embed(&EmbeddingCallOptions::new("Hello")).await;
        let seen = only_request(&fetch);
        assert!(
            seen.url
                .starts_with(&format!("{GLOBAL_BASE_URL}/models/test-embedding-model:")),
            "{}",
            seen.url
        );
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use correct URL for global region with image model
    #[tokio::test]
    #[serial]
    async fn should_use_correct_url_for_global_region_with_image_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("global", &fetch)).image("gemini-2.5-flash-image");

        assert_eq!(model.provider(), "google.vertex.image");
        let _ = model.do_generate(&ImageCallOptions::new("A cat")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, GLOBAL_BASE_URL);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use region-prefixed URL for non-global regions
    #[tokio::test]
    #[serial]
    async fn should_use_region_prefixed_url_for_non_global_regions() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("us-central1", &fetch)).call("test-model-id");

        assert_eq!(model.provider(), "google.vertex.chat");
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_base_url(
            &seen,
            "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/test-project/locations/us-central1/publishers/google",
        );
        assert_provider_headers(&seen);
    }

    /// The parts of the REP inline snapshots aimux exposes: `baseURL`,
    /// `provider`, `headers` and `supportedUrls`.
    async fn assert_multi_region(location: &str, base_url: &str) {
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings(location, &fetch)).call("test-model-id");

        assert_eq!(model.provider(), "google.vertex.chat");
        assert!(!model.supported_urls().await.is_empty());
        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_base_url(&seen, base_url);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use multi-region REP URL for us location
    #[tokio::test]
    #[serial]
    async fn should_use_multi_region_rep_url_for_us_location() {
        let _env = clean_environment();
        assert_multi_region(
            "us",
            "https://aiplatform.us.rep.googleapis.com/v1beta1/projects/test-project/locations/us/publishers/google",
        )
        .await;
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use multi-region REP URL for eu location
    #[tokio::test]
    #[serial]
    async fn should_use_multi_region_rep_url_for_eu_location() {
        let _env = clean_environment();
        assert_multi_region(
            "eu",
            "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/test-project/locations/eu/publishers/google",
        )
        .await;
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use express mode base URL when apiKey is provided
    #[tokio::test]
    #[serial]
    async fn should_use_express_mode_base_url_when_api_key_is_provided() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(express_settings(&fetch)).call("test-model-id");

        let seen = chat_request(model.as_ref(), &fetch).await;
        assert_base_url(
            &seen,
            "https://aiplatform.googleapis.com/v1/publishers/google",
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should add API key as query parameter via custom fetch
    #[tokio::test]
    #[serial]
    async fn should_add_api_key_as_query_parameter_via_custom_fetch() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        // Upstream calls the provider's fetch wrapper directly with a
        // streaming URL; aimux's wrapper is private, so a streaming request
        // of the same model goes through it.
        let model = provider(express_settings(&fetch)).call("gemini-2.5-pro");

        let _ = model.do_stream(&prompt()).await;
        let seen = only_request(&fetch);
        let (url, query) = seen.url.split_once('?').unwrap_or((&seen.url, ""));
        assert_eq!(
            url,
            "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:streamGenerateContent"
        );
        assert!(!query.contains("test-api-key"), "{}", seen.url);
        assert_eq!(
            seen.headers.get("x-goog-api-key").map(String::as_str),
            Some("test-api-key")
        );
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should create a video model with default settings
    #[tokio::test]
    #[serial]
    async fn should_create_a_video_model_with_default_settings() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("test-location", &fetch)).video("veo-2.0-generate-001");

        assert_eq!(model.model_id(), "veo-2.0-generate-001");
        assert_eq!(model.provider(), "google.vertex.video");
        let _ = model.do_start(&VideoCallOptions::new("A cat")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, TEST_LOCATION_BASE_URL);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use correct URL for global region with video model
    #[tokio::test]
    #[serial]
    async fn should_use_correct_url_for_global_region_with_video_model() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(settings("global", &fetch)).video("veo-3.0-generate-001");

        assert_eq!(model.model_id(), "veo-3.0-generate-001");
        assert_eq!(model.provider(), "google.vertex.video");
        let _ = model.do_start(&VideoCallOptions::new("A cat")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, GLOBAL_BASE_URL);
        assert_provider_headers(&seen);
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-provider-base.test.ts › google-vertex-provider-base › should use custom baseURL for video model when provided
    #[tokio::test]
    #[serial]
    async fn should_use_custom_base_url_for_video_model_when_provided() {
        let _env = clean_environment();
        let fetch = MockFetch::new(vec![]);
        let model = provider(VertexProviderSettings {
            base_url: Some(CUSTOM_BASE_URL.into()),
            ..settings("test-location", &fetch)
        })
        .video("veo-2.0-generate-001");

        let _ = model.do_start(&VideoCallOptions::new("A cat")).await;
        let seen = only_request(&fetch);
        assert_base_url(&seen, CUSTOM_BASE_URL);
    }
}
