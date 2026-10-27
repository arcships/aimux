//! `createGoogle` (`google-provider.test.ts`).
//!
//! Upstream mocks the model constructors and inspects the config they were
//! given. aimux models keep their config private, so each case observes it
//! the way a caller does: `provider()` / `model_id()` / `supported_urls()` on
//! the returned model, and the base URL and headers on the one request the
//! model sends through a recording transport (`MockFetch` with no canned
//! response: the request is recorded, then the call fails).
//!
//! The user-agent is aimux's pinned `ai-sdk-google/4.0.87` (upstream mocks
//! `0.0.0-test`); on the wire the request also carries the provider-utils
//! part, so the full pinned value is compared.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::result::GenerateContent;
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};
use aimux_core::tool::RawToolCall;
use aimux_core::video_model::{VideoCallOptions, VideoModel};

use crate::google::{GoogleImageSettings, GoogleProvider, GoogleProviderSettings, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const PINNED_USER_AGENT: &str = "ai-sdk-google/4.0.87 ai-sdk-provider-utils/5.0.53";

/// `createGoogle({ apiKey: 'test-api-key', ...settings })` over a recording
/// transport.
fn create(settings: GoogleProviderSettings) -> (GoogleProvider, Arc<MockFetch>) {
    create_answering(settings, Vec::new())
}

fn create_answering(
    settings: GoogleProviderSettings,
    canned: Vec<Canned>,
) -> (GoogleProvider, Arc<MockFetch>) {
    let mock = MockFetch::new(canned);
    let provider = create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .expect("valid settings");
    (provider, mock)
}

fn first_request(mock: &MockFetch) -> Seen {
    mock.seen()
        .into_iter()
        .next()
        .expect("the model sent a request")
}

/// The request a language model sends for a one-message prompt.
async fn language_request(model: &dyn LanguageModel, mock: &MockFetch) -> Seen {
    let _ = model
        .do_generate(&CallOptions::new(vec![
            aimux_core::language_model_message::LanguageModelMessage::user_text("Hello"),
        ]))
        .await;
    first_request(mock)
}

async fn embedding_request(model: &dyn EmbeddingModel, mock: &MockFetch) -> Seen {
    let _ = model.do_embed(&EmbeddingCallOptions::new("Hello")).await;
    first_request(mock)
}

async fn image_request(model: &dyn ImageModel, mock: &MockFetch) -> Seen {
    let _ = model.do_generate(&ImageCallOptions::new("A cat")).await;
    first_request(mock)
}

async fn video_request(model: &dyn VideoModel, mock: &MockFetch) -> Seen {
    let _ = model.do_start(&VideoCallOptions::new("A cat")).await;
    first_request(mock)
}

async fn speech_request(model: &dyn SpeechModel, mock: &MockFetch) -> Seen {
    let _ = model.do_generate(&SpeechCallOptions::new("Hello")).await;
    first_request(mock)
}

fn patterns(supported_urls: &SupportedUrls) -> &[regex::Regex] {
    supported_urls.0.get("*").expect("a '*' entry")
}

mod google_provider {
    use super::*;

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should create a language model with default settings
    #[tokio::test]
    async fn should_create_a_language_model_with_default_settings() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = provider.call("gemini-pro");

        assert_eq!(model.model_id(), "gemini-pro");
        assert_eq!(model.provider(), "google.generative-ai");
        assert!(!model.supported_urls().await.is_empty());
        let request = language_request(model.as_ref(), &mock).await;
        assert_eq!(
            request.url,
            format!("{DEFAULT_BASE_URL}/models/gemini-pro:generateContent")
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should create an embedding model with correct settings
    #[tokio::test]
    async fn should_create_an_embedding_model_with_correct_settings() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = Provider::embedding_model(&provider, "embedding-001").unwrap();

        assert_eq!(model.model_id(), "embedding-001");
        assert_eq!(model.provider(), "google.generative-ai");
        let request = embedding_request(model.as_ref(), &mock).await;
        assert!(
            request
                .url
                .starts_with(&format!("{DEFAULT_BASE_URL}/models/embedding-001:")),
            "{}",
            request.url
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should pass custom headers to the model constructor
    #[tokio::test]
    async fn should_pass_custom_headers_to_the_model_constructor() {
        let (provider, mock) = create(GoogleProviderSettings {
            headers: Some(HashMap::from([(
                "Custom-Header".to_string(),
                Some("custom-value".to_string()),
            )])),
            ..Default::default()
        });
        let model = provider.call("gemini-pro");

        let request = language_request(model.as_ref(), &mock).await;
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
        assert_eq!(request.headers["custom-header"], "custom-value");
        assert_eq!(request.headers["user-agent"], PINNED_USER_AGENT);
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should pass custom generateId function to the model constructor
    #[tokio::test]
    async fn should_pass_custom_generate_id_function_to_the_model_constructor() {
        // The model calls `generateId` for a function call that has no id.
        let (provider, _mock) = create_answering(
            GoogleProviderSettings {
                generate_id: Some(Arc::new(|| "custom-id".to_string())),
                ..Default::default()
            },
            vec![Canned::json(&json!({
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{ "functionCall": { "name": "test-tool", "args": { "value": "x" } } }],
                        },
                        "finishReason": "STOP",
                    },
                ],
                "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 1 },
            }))],
        );
        let model = provider.call("gemini-pro");

        let result = model
            .do_generate(&CallOptions::new(vec![
                aimux_core::language_model_message::LanguageModelMessage::user_text("Hello"),
            ]))
            .await
            .unwrap();
        let tool_call_id = result.content.iter().find_map(|part| match part {
            GenerateContent::ToolCall(RawToolCall { tool_call_id, .. }) => {
                Some(tool_call_id.as_str())
            }
            _ => None,
        });
        assert_eq!(tool_call_id, Some("custom-id"));
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should use chat method to create a model
    #[test]
    fn should_use_chat_method_to_create_a_model() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let model = provider.chat("gemini-pro");

        assert_eq!(model.model_id(), "gemini-pro");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should use custom baseURL when provided
    #[tokio::test]
    async fn should_use_custom_base_url_when_provided() {
        let (provider, mock) = create(GoogleProviderSettings {
            base_url: Some("https://custom-endpoint.example.com".into()),
            ..Default::default()
        });
        let model = provider.call("gemini-pro");

        let request = language_request(model.as_ref(), &mock).await;
        assert_eq!(
            request.url,
            "https://custom-endpoint.example.com/models/gemini-pro:generateContent"
        );
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should create an image model with default settings
    #[tokio::test]
    async fn should_create_an_image_model_with_default_settings() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = provider.image("gemini-2.5-flash-image");

        assert_eq!(model.model_id(), "gemini-2.5-flash-image");
        assert_eq!(model.provider(), "google.generative-ai");
        let request = image_request(&model, &mock).await;
        assert!(
            request.url.starts_with(&format!("{DEFAULT_BASE_URL}/")),
            "{}",
            request.url
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should create an image model with custom maxImagesPerCall
    #[tokio::test]
    async fn should_create_an_image_model_with_custom_max_images_per_call() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = provider.image_with_settings(
            "gemini-2.5-flash-image",
            GoogleImageSettings {
                max_images_per_call: Some(3),
            },
        );

        assert_eq!(model.model_id(), "gemini-2.5-flash-image");
        assert_eq!(model.max_images_per_call(), Some(3));
        assert_eq!(model.provider(), "google.generative-ai");
        let request = image_request(&model, &mock).await;
        assert!(
            request.url.starts_with(&format!("{DEFAULT_BASE_URL}/")),
            "{}",
            request.url
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should support deprecated methods
    #[test]
    fn should_support_deprecated_methods() {
        let (provider, _) = create(GoogleProviderSettings::default());

        let language = provider.generative_ai("gemini-pro");
        let embedding = provider.embedding("embedding-001");
        let embedding_model = Provider::embedding_model(&provider, "embedding-001").unwrap();

        assert_eq!(language.model_id(), "gemini-pro");
        assert_eq!(embedding.model_id(), "embedding-001");
        assert_eq!(embedding_model.model_id(), "embedding-001");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should include YouTube URLs in supportedUrls
    #[tokio::test]
    async fn should_include_youtube_urls_in_supported_urls() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let supported_urls = provider.call("gemini-pro").supported_urls().await;
        let patterns = patterns(&supported_urls);

        let is_supported = |url: &str| patterns.iter().any(|pattern| pattern.is_match(url));
        let results = |urls: &[&str]| {
            urls.iter()
                .map(|url| json!({ "isSupported": is_supported(url), "url": url }))
                .collect::<Vec<_>>()
        };
        let test_results = json!({
            "supportedUrls": results(&[
                "https://generativelanguage.googleapis.com/v1beta/files/test123",
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                "https://youtube.com/watch?v=dQw4w9WgXcQ",
                "https://youtu.be/dQw4w9WgXcQ",
            ]),
            "unsupportedUrls": results(&[
                "https://example.com",
                "https://vimeo.com/123456789",
                "https://youtube.com/channel/UCdQw4w9WgXcQ",
            ]),
        });

        assert_eq!(
            test_results,
            json!({
                "supportedUrls": [
                    {
                        "isSupported": true,
                        "url": "https://generativelanguage.googleapis.com/v1beta/files/test123",
                    },
                    { "isSupported": true, "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ" },
                    { "isSupported": true, "url": "https://youtube.com/watch?v=dQw4w9WgXcQ" },
                    { "isSupported": true, "url": "https://youtu.be/dQw4w9WgXcQ" },
                ],
                "unsupportedUrls": [
                    { "isSupported": false, "url": "https://example.com" },
                    { "isSupported": false, "url": "https://vimeo.com/123456789" },
                    { "isSupported": false, "url": "https://youtube.com/channel/UCdQw4w9WgXcQ" },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should support default and configured Google file URLs with a custom baseURL
    #[tokio::test]
    async fn should_support_default_and_configured_google_file_urls_with_a_custom_base_url() {
        let (provider, _) = create(GoogleProviderSettings {
            base_url: Some("https://custom-endpoint.example.com/v1beta".into()),
            ..Default::default()
        });
        let supported_urls = provider.call("gemini-2.0-flash").supported_urls().await;

        for url in [
            "https://generativelanguage.googleapis.com/v1beta/files/google-file",
            "https://custom-endpoint.example.com/v1beta/files/custom-file",
        ] {
            assert!(supported_urls.is_supported(url, "text/markdown"), "{url}");
        }

        assert!(!supported_urls.is_supported(
            "https://example.com/files/unsupported-file",
            "text/markdown"
        ));
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should support documented external HTTPS URLs for Gemini models that accept external URLs
    #[tokio::test]
    async fn should_support_documented_external_https_urls_for_gemini_models() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let supported_urls = provider.call("gemini-3.5-flash").supported_urls().await;

        for media_type in [
            "text/html",
            "text/css",
            "text/plain",
            "text/xml",
            "text/csv",
            "text/rtf",
            "text/javascript",
            "application/json",
            "application/pdf",
            "image/bmp",
            "image/jpeg",
            "image/png",
            "image/webp",
            "video/mp4",
            "video/mpeg",
            "video/quicktime",
            "video/avi",
            "video/x-flv",
            "video/mpg",
            "video/webm",
            "video/wmv",
            "video/3gpp",
        ] {
            assert!(
                supported_urls.is_supported("https://example.com/file", media_type),
                "{media_type}"
            );
        }

        assert!(!supported_urls.is_supported("http://example.com/file.txt", "text/plain"));
        assert!(!supported_urls.is_supported("https://example.com/file.md", "text/markdown"));
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google-provider › should not support external HTTPS URLs for Gemini 2.0 models
    #[tokio::test]
    async fn should_not_support_external_https_urls_for_gemini_2_0_models() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let supported_urls = provider.call("gemini-2.0-flash").supported_urls().await;

        assert!(!supported_urls.is_supported("https://example.com/file.txt", "text/plain"));
    }
}

mod google_provider_custom_provider_name {
    use super::*;

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - custom provider name › should use custom provider name when specified
    #[test]
    fn should_use_custom_provider_name_when_specified() {
        let (provider, _) = create(GoogleProviderSettings {
            name: Some("my-gemini-proxy".into()),
            ..Default::default()
        });
        let model = provider.call("gemini-pro");

        assert_eq!(model.model_id(), "gemini-pro");
        assert_eq!(model.provider(), "my-gemini-proxy");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - custom provider name › should default to google.generative-ai when name not specified
    #[test]
    fn should_default_to_google_generative_ai_when_name_not_specified() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let model = provider.call("gemini-pro");

        assert_eq!(model.model_id(), "gemini-pro");
        assert_eq!(model.provider(), "google.generative-ai");
    }
}

mod google_provider_video {
    use super::*;

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - video › should create a video model with default settings
    #[tokio::test]
    async fn should_create_a_video_model_with_default_settings() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = provider.video("veo-3.1-generate-preview");

        assert_eq!(model.model_id(), "veo-3.1-generate-preview");
        assert_eq!(model.provider(), "google.generative-ai");
        let request = video_request(&model, &mock).await;
        assert!(
            request.url.starts_with(&format!(
                "{DEFAULT_BASE_URL}/models/veo-3.1-generate-preview:"
            )),
            "{}",
            request.url
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - video › should use custom baseURL for video model when provided
    #[tokio::test]
    async fn should_use_custom_base_url_for_video_model_when_provided() {
        let (provider, mock) = create(GoogleProviderSettings {
            base_url: Some("https://custom-endpoint.example.com".into()),
            ..Default::default()
        });
        let model = provider.video("veo-3.1-generate");

        assert_eq!(model.model_id(), "veo-3.1-generate");
        let request = video_request(&model, &mock).await;
        assert!(
            request
                .url
                .starts_with("https://custom-endpoint.example.com/models/veo-3.1-generate:"),
            "{}",
            request.url
        );
    }
}

mod google_provider_speech {
    use super::*;

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - speech › should create a speech model with default settings
    #[tokio::test]
    async fn should_create_a_speech_model_with_default_settings() {
        let (provider, mock) = create(GoogleProviderSettings::default());
        let model = provider.speech("gemini-2.5-flash-preview-tts");

        assert_eq!(model.model_id(), "gemini-2.5-flash-preview-tts");
        assert_eq!(model.provider(), "google.generative-ai.speech");
        let request = speech_request(&model, &mock).await;
        assert!(
            request.url.starts_with(&format!(
                "{DEFAULT_BASE_URL}/models/gemini-2.5-flash-preview-tts:"
            )),
            "{}",
            request.url
        );
        assert_eq!(request.headers["x-goog-api-key"], "test-api-key");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - speech › should create a speech model via speechModel()
    #[test]
    fn should_create_a_speech_model_via_speech_model() {
        let (provider, _) = create(GoogleProviderSettings::default());
        let model = Provider::speech_model(&provider, "gemini-2.5-pro-preview-tts")
            .expect("the provider has speech models")
            .unwrap();

        assert_eq!(model.model_id(), "gemini-2.5-pro-preview-tts");
        assert_eq!(model.provider(), "google.generative-ai.speech");
    }

    /// upstream: @ai-sdk/google src/google-provider.test.ts › google provider - speech › should use custom baseURL for speech model when provided
    #[tokio::test]
    async fn should_use_custom_base_url_for_speech_model_when_provided() {
        let (provider, mock) = create(GoogleProviderSettings {
            base_url: Some("https://custom-endpoint.example.com".into()),
            ..Default::default()
        });
        let model = provider.speech("gemini-2.5-flash-preview-tts");

        assert_eq!(model.model_id(), "gemini-2.5-flash-preview-tts");
        let request = speech_request(&model, &mock).await;
        assert!(
            request.url.starts_with(
                "https://custom-endpoint.example.com/models/gemini-2.5-flash-preview-tts:"
            ),
            "{}",
            request.url
        );
    }
}
