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

// ── Older ports (previously `tests/google_factory_test.rs`) ──
//
// `create_google` against the recorded AI SDK behavior.
//
// `fixtures/aisdk/google/*.json` hold what `@ai-sdk/google`'s `createGoogle`
// sent over a mocked fetch for a fixed input: URL, headers, body and the
// model's `provider` string. Each fixture is replayed here through an
// injected [`Fetch`] (the factory's `fetch` setting), so the test also proves
// the transport is the one the settings name. The expected request comes from
// the fixture; the inputs are rebuilt from the fixture's recorded `sdk.input`.
//
// The recorded `x-goog-api-key` is redacted, so the key the case used is
// substituted back. The user-agent aimux sends is checked against the
// recorded one (package and provider-utils parts).

mod older_factory {
    use aimux_core::tool::RawToolCall;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use futures::StreamExt;
    use serde_json::{Value, json};

    use crate::google::{GoogleProviderSettings, create_google};
    use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
    use aimux_core::options::CallOptions;
    use aimux_core::result::{GenerateContent, GenerateResult};
    use aimux_core::stream_part::StreamPart;
    use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, Tool};
    use aimux_core::types::FinishReasonUnified;

    use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

    const KEY: &str = "sk-test-fixture";

    fn settings(mock: &Arc<MockFetch>) -> GoogleProviderSettings {
        GoogleProviderSettings {
            api_key: Some(KEY.to_string()),
            fetch: Some(mock.transport()),
            ..Default::default()
        }
    }

    // ── fixtures ─────────────────────────────────────────────────────────────────

    /// Every fixture under `fixtures/aisdk/google/`; the directory listing is
    /// compared to this so a new fixture cannot go unreplayed.
    const FIXTURES: [&str; 5] = [
        "embedding-basic",
        "generate-basic",
        "generate-provider-options",
        "generate-stream-basic",
        "generate-tools",
    ];

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/aisdk/google")
    }

    struct Fixture {
        name: &'static str,
        json: Value,
    }

    impl Fixture {
        fn load(name: &'static str) -> Self {
            assert!(FIXTURES.contains(&name), "unlisted fixture {name}");
            let path = fixtures_dir().join(format!("{name}.json"));
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            Self {
                name,
                json: serde_json::from_str(&text).expect("fixture is JSON"),
            }
        }

        fn provider_string(&self) -> &str {
            self.json["model"]["provider"].as_str().unwrap()
        }

        fn model_id(&self) -> &str {
            self.json["model"]["modelId"].as_str().unwrap()
        }

        fn sdk_input(&self) -> &Value {
            &self.json["sdk"]["input"]
        }

        /// The transport answering with the recorded response.
        fn mock(&self) -> Arc<MockFetch> {
            let response = &self.json["response"];
            let body = match &response["body"] {
                Value::String(text) => text.clone().into_bytes(),
                other => serde_json::to_vec(other).unwrap(),
            };
            MockFetch::new(vec![Canned {
                status: response["status"].as_u64().unwrap() as u16,
                headers: response["headers"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect(),
                body,
            }])
        }

        /// The recorded request must equal what went out: method, URL, headers
        /// (names case-insensitively; user-agent checked against the recording) and body.
        fn assert_request(&self, seen: &Seen) {
            let recorded = &self.json["request"];
            assert_eq!(
                seen.method,
                recorded["method"].as_str().unwrap(),
                "{}",
                self.name
            );
            assert_eq!(seen.url, recorded["url"].as_str().unwrap(), "{}", self.name);

            let mut expected: BTreeMap<String, String> = recorded["headers"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.as_str().unwrap().to_string()))
                .collect();
            // The user-agent is checked against the recording, not overwritten:
            // aimux sends `ai-sdk-google/<v> ai-sdk-provider-utils/<v>`; every
            // recording carries the same provider-utils part, and a provider-layer
            // recording (not made through `ai`) starts with the same package part.
            let recorded_ua = expected.remove("user-agent").expect("recorded user-agent");
            let mut sent_headers = seen.headers.clone();
            let sent_ua = sent_headers.remove("user-agent").expect("sent user-agent");
            let (package_ua, utils_ua) = sent_ua
                .split_once(' ')
                .expect("package and provider-utils user-agent parts");
            assert!(
                package_ua.starts_with("ai-sdk-google/")
                    && utils_ua.starts_with("ai-sdk-provider-utils/"),
                "{}: sent user-agent {sent_ua:?}",
                self.name
            );
            assert!(
                recorded_ua.contains(&format!(" {utils_ua} ")),
                "{}: recorded user-agent {recorded_ua:?} lacks {utils_ua:?}",
                self.name
            );
            if recorded_ua.starts_with("ai-sdk-google/") {
                assert!(
                    recorded_ua.starts_with(&format!("{package_ua} ")),
                    "{}: recorded user-agent {recorded_ua:?} does not start with {package_ua:?}",
                    self.name
                );
            }
            assert_eq!(
                expected.get("x-goog-api-key").map(String::as_str),
                Some("<redacted>"),
                "{}: the recording redacts the key",
                self.name
            );
            expected.insert("x-goog-api-key".to_string(), KEY.to_string());
            assert_eq!(sent_headers, expected, "{}: headers", self.name);

            assert_eq!(seen.json_body(), recorded["body"], "{}: body", self.name);
        }

        /// The call options the recording was made with.
        fn call_options(&self) -> CallOptions {
            call_options_from(self.sdk_input())
        }
    }

    /// Rebuild aimux call options from a recorded `sdk.input` (the subset the
    /// Google fixtures use).
    fn call_options_from(input: &Value) -> CallOptions {
        let mut prompt: LanguageModelPrompt = Vec::new();
        if let Some(system) = input.get("system").and_then(Value::as_str) {
            prompt.push(LanguageModelMessage::System {
                content: system.to_string(),
                provider_options: None,
            });
        }
        if let Some(text) = input.get("prompt").and_then(Value::as_str) {
            prompt.push(LanguageModelMessage::user_text(text));
        }
        let mut options = CallOptions::new(prompt);
        if let Some(tools) = input.get("tools").and_then(Value::as_object) {
            options.tools = Some(
                tools
                    .iter()
                    .map(|(name, tool)| {
                        let mut function =
                            FunctionTool::new(name.clone(), tool["inputSchema"].clone());
                        if let Some(description) = tool["description"].as_str() {
                            function = function.with_description(description);
                        }
                        Tool::Function(function)
                    })
                    .collect(),
            );
        }
        options.tool_choice = match input.get("toolChoice").and_then(Value::as_str) {
            Some("required") => Some(LanguageModelV4ToolChoice::Required),
            Some("none") => Some(LanguageModelV4ToolChoice::None),
            Some("auto") => Some(LanguageModelV4ToolChoice::Auto),
            None => options
                .tools
                .as_ref()
                .filter(|tools| !tools.is_empty())
                .map(|_| LanguageModelV4ToolChoice::Auto),
            Some(other) => panic!("unmapped toolChoice {other}"),
        };
        options.provider_options = input
            .get("providerOptions")
            .map(|value| serde_json::from_value(value.clone()).unwrap());
        options
    }

    fn generated_text(result: &GenerateResult) -> String {
        result
            .content
            .iter()
            .filter_map(|part| match part {
                GenerateContent::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn unified_finish(recorded: &str) -> FinishReasonUnified {
        match recorded {
            "stop" => FinishReasonUnified::Stop,
            "tool-calls" => FinishReasonUnified::ToolCalls,
            other => panic!("unmapped finish reason {other}"),
        }
    }

    /// Result fields every recorded `generateText` case pins.
    fn assert_generate_result(fixture: &Fixture, result: &GenerateResult) {
        let recorded = &fixture.json["result"];
        assert_eq!(
            generated_text(result),
            recorded["text"].as_str().unwrap(),
            "{}: text",
            fixture.name
        );
        assert_eq!(
            result.response.as_ref().and_then(|r| r.id.as_deref()),
            recorded["response"]["id"].as_str(),
            "{}: response id",
            fixture.name
        );
        assert_eq!(
            result.usage.input_tokens.total,
            recorded["usage"]["inputTokens"].as_u64().map(|v| v as _),
            "{}: input tokens",
            fixture.name
        );
        assert_eq!(
            result.usage.output_tokens.total,
            recorded["usage"]["outputTokens"].as_u64().map(|v| v as _),
            "{}: output tokens",
            fixture.name
        );
        assert_eq!(
            result.finish_reason.unified,
            unified_finish(recorded["finishReason"].as_str().unwrap()),
            "{}: finish",
            fixture.name
        );
        assert_eq!(
            result.finish_reason.raw.as_deref(),
            recorded["rawFinishReason"].as_str(),
            "{}: raw finish",
            fixture.name
        );
        // The usage metadata comes back under the Gemini namespace.
        assert_eq!(
            result.provider_metadata.as_ref().unwrap()["google"]["usageMetadata"],
            recorded["providerMetadata"]["google"]["usageMetadata"],
            "{}: usage metadata",
            fixture.name
        );
    }

    // ── the fixtures ─────────────────────────────────────────────────────────────

    #[test]
    fn every_fixture_is_replayed() {
        let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
            .expect("fixtures/aisdk/google exists")
            .map(|entry| {
                let name = entry.unwrap().file_name().into_string().unwrap();
                name.strip_suffix(".json")
                    .unwrap_or_else(|| panic!("unexpected file {name}"))
                    .to_string()
            })
            .collect();
        on_disk.sort();
        assert_eq!(
            on_disk, FIXTURES,
            "add a test for the new fixture and list it"
        );
    }

    /// Replay a fixture whose request is a plain `generateText`.
    async fn replay_generate(name: &'static str) -> (Fixture, Arc<MockFetch>, GenerateResult) {
        let fixture = Fixture::load(name);
        let mock = fixture.mock();
        let provider = create_google(settings(&mock)).unwrap();
        let model = provider.chat(fixture.model_id());
        assert_eq!(
            model.provider(),
            fixture.provider_string(),
            "{name}: provider()"
        );
        assert_eq!(model.model_id(), fixture.model_id());

        let result = model.do_generate(&fixture.call_options()).await.unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1, "{name}: exactly one request");
        fixture.assert_request(&seen[0]);
        assert_generate_result(&fixture, &result);
        (fixture, mock, result)
    }

    #[tokio::test]
    async fn generate_basic() {
        replay_generate("generate-basic").await;
    }

    #[tokio::test]
    async fn generate_provider_options_are_read_from_the_google_namespace() {
        let (fixture, _, _) = replay_generate("generate-provider-options").await;
        // The recorded body carries both option groups the case set.
        let body = &fixture.json["request"]["body"];
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            json!(1024)
        );
        assert_eq!(
            body["safetySettings"][0]["threshold"],
            json!("BLOCK_ONLY_HIGH")
        );
    }

    #[tokio::test]
    async fn generate_tools() {
        let (fixture, _, result) = replay_generate("generate-tools").await;
        let recorded = &fixture.json["result"]["content"][0];
        let call = result
            .content
            .iter()
            .find_map(|part| match part {
                GenerateContent::ToolCall(RawToolCall {
                    tool_name, input, ..
                }) => Some((tool_name.clone(), input.clone())),
                _ => None,
            })
            .expect("a tool call");
        assert_eq!(call.0, recorded["toolName"].as_str().unwrap());
        assert_eq!(
            serde_json::from_str::<Value>(&call.1).unwrap(),
            recorded["input"]
        );
    }

    #[tokio::test]
    async fn generate_stream_basic() {
        let fixture = Fixture::load("generate-stream-basic");
        let mock = fixture.mock();
        let model = create_google(settings(&mock))
            .unwrap()
            .chat(fixture.model_id());
        assert_eq!(model.provider(), fixture.provider_string());

        let result = model.do_stream(&fixture.call_options()).await.unwrap();
        let parts: Vec<StreamPart> = result
            .stream
            .map(|part| part.expect("no stream error"))
            .collect()
            .await;

        // The recording's user-agent names the provider package; the stream case
        // is checked like the others.
        assert!(
            fixture.json["request"]["headers"]["user-agent"]
                .as_str()
                .unwrap()
                .starts_with("ai-sdk-google/"),
            "the stream case is a provider-layer recording"
        );
        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        fixture.assert_request(&seen[0]);

        let text: String = parts
            .iter()
            .filter_map(|part| match part {
                StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Hello world");
        let recorded = &fixture.json["result"];
        let finish = parts.iter().find_map(|part| match part {
            StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata,
            } => Some((finish_reason, usage, provider_metadata)),
            _ => None,
        });
        let (reason, usage, metadata) = finish.expect("a finish part");
        assert_eq!(
            reason.unified,
            unified_finish(recorded["finishReason"].as_str().unwrap())
        );
        assert_eq!(reason.raw.as_deref(), Some("STOP"));
        assert_eq!(
            usage.input_tokens.total,
            recorded["usage"]["inputTokens"].as_u64().map(|v| v as _)
        );
        assert_eq!(
            usage.output_tokens.total,
            recorded["usage"]["outputTokens"].as_u64().map(|v| v as _)
        );
        assert_eq!(
            metadata.as_ref().unwrap()["google"]["usageMetadata"],
            recorded["parts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|part| part["type"] == "finish-step")
                .unwrap()["providerMetadata"]["google"]["usageMetadata"]
        );
    }

    #[tokio::test]
    async fn embedding_basic() {
        let fixture = Fixture::load("embedding-basic");
        let mock = fixture.mock();
        let model = create_google(settings(&mock))
            .unwrap()
            .embedding(fixture.model_id());
        assert_eq!(model.provider(), fixture.provider_string());

        let values: Vec<String> = fixture.sdk_input()["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let result = model
            .do_embed(&EmbeddingCallOptions {
                values,
                abort_signal: None,
                max_retries: None,
                timeout: None,
                provider_options: None,
                headers: None,
            })
            .await
            .unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        fixture.assert_request(&seen[0]);
        let recorded: Vec<Vec<f32>> = fixture.json["result"]["embeddings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap() as f32)
                    .collect()
            })
            .collect();
        assert_eq!(result.embeddings, recorded);
    }
}
