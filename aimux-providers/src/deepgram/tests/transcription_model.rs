//! Port of `@ai-sdk/deepgram@3.1.24` `src/deepgram-transcription-model.test.ts`.
//!
//! Harness: upstream sends `transcript-test.mp3` to a test server at
//! `https://api.deepgram.com/v1/listen`; here a [`MockFetch`] answers with the
//! same fixture, and a few stand-in bytes are the audio (no case reads it
//! back). The `customModel` cases build the model from a config with a fixed
//! endpoint, as upstream builds `new DeepgramTranscriptionModel(...)`.
//! Upstream injects the date (`_internal.currentDate`); aimux reads the
//! clock, so timestamps are checked to fall within the call.

use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

use crate::deepgram::{DeepgramProviderSettings, DeepgramTranscriptionModel, create_deepgram};
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

const LISTEN_URL: &str = "https://api.deepgram.com/v1/listen";

/// `prepareJsonFixtureResponse('deepgram-transcription', headers)`.
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/deepgram-transcription.json")).unwrap()
}

fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let mut all = vec![("content-type".to_string(), "application/json".to_string())];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    Canned {
        status: 200,
        headers: all,
        body: serde_json::to_vec(body).unwrap(),
    }
}

fn fixture_fetch() -> Arc<MockFetch> {
    MockFetch::new(vec![json_response(&fixture(), &[])])
}

/// `createDeepgram({ apiKey: 'test-api-key', headers })`.
fn provider_model(
    fetch: &Arc<MockFetch>,
    headers: Option<HeaderMapOpt>,
) -> DeepgramTranscriptionModel {
    create_deepgram(DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        headers,
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("nova-3")
}

/// `provider.transcription('nova-3')`.
fn model(fetch: &Arc<MockFetch>) -> DeepgramTranscriptionModel {
    provider_model(fetch, None)
}

/// The base URL and `headers: () => ({})` of upstream's `customModel`.
struct StaticEndpoint;

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: "https://api.deepgram.com".to_string(),
            headers: HeaderMapOpt::new(),
        })
    }
}

/// `new DeepgramTranscriptionModel('nova-3', { provider: 'test-provider',
/// url: () => 'https://api.deepgram.com/v1/listen', headers: () => ({}) })`.
fn custom_model(fetch: &Arc<MockFetch>) -> DeepgramTranscriptionModel {
    DeepgramTranscriptionModel::from_config(
        "nova-3".to_string(),
        EndpointConfig::new(
            "test-provider".to_string(),
            StaticEndpoint,
            Some(fetch.transport()),
        ),
    )
}

/// `{ audio: audioData, mediaType: 'audio/wav', providerOptions }`.
fn call(provider_options: Option<Value>) -> TranscriptionCallOptions {
    let mut call =
        TranscriptionCallOptions::new(AudioInput::Binary(b"audio".to_vec()), "audio/wav");
    call.provider_options = provider_options.map(|value| serde_json::from_value(value).unwrap());
    call
}

async fn generate(
    model: &DeepgramTranscriptionModel,
    call: &TranscriptionCallOptions,
) -> TranscriptionResult {
    model.do_generate(call).await.unwrap()
}

/// `new URL(server.calls[0].requestUrl).searchParams.get(key)`.
fn query_param(seen: &Seen, key: &str) -> Option<String> {
    url::Url::parse(&seen.url)
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// A body with one channel: `detected_language` when given.
fn inline_response(transcript: &str, detected_language: Option<&str>) -> Value {
    let mut channel = json!({ "alternatives": [{ "transcript": transcript, "words": [] }] });
    if let Some(language) = detected_language {
        channel["detected_language"] = json!(language);
    }
    json!({ "metadata": { "duration": 1.0 }, "results": { "channels": [channel] } })
}

mod do_generate {
    use super::*;

    mod transcription {
        use super::*;

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should pass the model
        ///
        /// Upstream checks only that the multipart body matches `{}`; the
        /// model is a query parameter.
        #[tokio::test]
        async fn should_pass_the_model() {
            let fetch = fixture_fetch();
            generate(&model(&fetch), &call(None)).await;

            let seen = &fetch.seen()[0];
            assert!(seen.url.starts_with(LISTEN_URL), "{}", seen.url);
            assert_eq!(query_param(seen, "model").as_deref(), Some("nova-3"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should pass headers
        ///
        /// The user-agent part (`ai-sdk-deepgram/<version>`) is not sent by
        /// aimux yet (layout.toml), so only the headers are compared.
        #[tokio::test]
        async fn should_pass_headers() {
            let fetch = fixture_fetch();
            let model = provider_model(
                &fetch,
                Some(
                    [(
                        "Custom-Provider-Header".to_string(),
                        Some("provider-header-value".to_string()),
                    )]
                    .into(),
                ),
            );
            let mut call = call(None);
            call.headers = Some(
                [(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )]
                .into(),
            );
            generate(&model, &call).await;

            let headers = &fetch.seen()[0].headers;
            assert_eq!(headers["authorization"], "Token test-api-key");
            assert_eq!(headers["content-type"], "audio/wav");
            assert_eq!(headers["custom-provider-header"], "provider-header-value");
            assert_eq!(headers["custom-request-header"], "request-header-value");
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should extract the transcription text
        #[tokio::test]
        async fn should_extract_the_transcription_text() {
            let fetch = fixture_fetch();
            let result = generate(&model(&fetch), &call(None)).await;

            assert_eq!(
                result.text,
                "galileo was an american robotic space program that studied the planet jupiter and its moons as well as several other solar system bodies named after the italian astronomer galileo galilei the galileo spacecraft consisted of an orbiter and an atmospheric entry probe it was delivered into earth orbit on october eighteen nineteen eighty nine by space shuttle atlantis on the sts-thirty four mission and arrived at jupiter on december seven nineteen ninety five after gravity assist flybys of venus and earth and became the first spacecraft to orbit jupiter"
            );
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should pass detectLanguage as detect_language query parameter
        #[tokio::test]
        async fn should_pass_detect_language_as_detect_language_query_parameter() {
            let fetch = fixture_fetch();
            generate(
                &model(&fetch),
                &call(Some(json!({ "deepgram": { "detectLanguage": true } }))),
            )
            .await;

            assert!(fetch.seen()[0].url.contains("detect_language=true"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should return detected language from response
        #[tokio::test]
        async fn should_return_detected_language_from_response() {
            let fetch = fixture_fetch();
            let result = generate(
                &model(&fetch),
                &call(Some(json!({ "deepgram": { "detectLanguage": true } }))),
            )
            .await;

            assert_eq!(result.language.as_deref(), Some("en"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should not send diarize by default
        #[tokio::test]
        async fn should_not_send_diarize_by_default() {
            let fetch = fixture_fetch();
            generate(&model(&fetch), &call(None)).await;

            assert!(!fetch.seen()[0].url.contains("diarize"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should pass diarize when explicitly enabled
        #[tokio::test]
        async fn should_pass_diarize_when_explicitly_enabled() {
            let fetch = fixture_fetch();
            generate(
                &model(&fetch),
                &call(Some(json!({ "deepgram": { "diarize": true } }))),
            )
            .await;

            assert!(fetch.seen()[0].url.contains("diarize=true"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › transcription › should pass keyterm, paragraphs, intents, sentiment, and replace as query parameters
        #[tokio::test]
        async fn should_pass_keyterm_paragraphs_intents_sentiment_and_replace_as_query_parameters()
        {
            let fetch = fixture_fetch();
            generate(
                &model(&fetch),
                &call(Some(json!({ "deepgram": {
                    "keyterm": "galileo",
                    "paragraphs": true,
                    "intents": true,
                    "sentiment": true,
                    "redact": "numbers",
                    "replace": "[redacted]",
                } }))),
            )
            .await;

            let seen = &fetch.seen()[0];
            assert_eq!(query_param(seen, "keyterm").as_deref(), Some("galileo"));
            assert_eq!(query_param(seen, "paragraphs").as_deref(), Some("true"));
            assert_eq!(query_param(seen, "intents").as_deref(), Some("true"));
            assert_eq!(query_param(seen, "sentiment").as_deref(), Some("true"));
            assert_eq!(query_param(seen, "redact").as_deref(), Some("numbers"));
            assert_eq!(query_param(seen, "replace").as_deref(), Some("[redacted]"));
        }
    }

    mod response_headers {
        use super::*;

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › response headers › should include response data with timestamp, modelId and headers
        ///
        /// Upstream's snapshot is `{ body, headers, modelId, timestamp }`:
        /// the body is the fixture, the headers include the test's two.
        #[tokio::test]
        async fn should_include_response_data_with_timestamp_model_id_and_headers() {
            let fetch = MockFetch::new(vec![json_response(
                &fixture(),
                &[
                    ("x-request-id", "test-request-id"),
                    ("x-ratelimit-remaining", "123"),
                ],
            )]);
            let before = aimux_core::Timestamp::now();
            let result = generate(&custom_model(&fetch), &call(None)).await;
            let after = aimux_core::Timestamp::now();

            let response = result.response;
            assert_eq!(response.body, Some(fixture()));
            let headers = response.headers.expect("response headers");
            assert_eq!(headers["x-request-id"], "test-request-id");
            assert_eq!(headers["x-ratelimit-remaining"], "123");
            assert_eq!(response.model_id.as_deref(), Some("nova-3"));
            let timestamp = response.timestamp.expect("a timestamp");
            assert!(before <= timestamp && timestamp <= after);
        }
    }

    mod response_metadata {
        use super::*;

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › response metadata › should use real date when no custom date provider is specified
        #[tokio::test]
        async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
            let fetch = fixture_fetch();
            let before = aimux_core::Timestamp::now();
            let result = generate(&custom_model(&fetch), &call(None)).await;
            let after = aimux_core::Timestamp::now();

            let timestamp = result.response.timestamp.expect("a timestamp");
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(result.response.model_id.as_deref(), Some("nova-3"));
        }
    }

    mod language_detection {
        use super::*;

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › language detection › should return detected language from inline response
        #[tokio::test]
        async fn should_return_detected_language_from_inline_response() {
            let fetch = MockFetch::new(vec![json_response(
                &inline_response("hej", Some("sv")),
                &[],
            )]);
            let result = generate(
                &model(&fetch),
                &call(Some(json!({ "deepgram": { "detectLanguage": true } }))),
            )
            .await;

            assert_eq!(result.language.as_deref(), Some("sv"));
        }

        /// upstream: @ai-sdk/deepgram src/deepgram-transcription-model.test.ts › doGenerate › language detection › should return undefined language when not detected
        #[tokio::test]
        async fn should_return_undefined_language_when_not_detected() {
            let fetch = MockFetch::new(vec![json_response(&inline_response("hello", None), &[])]);
            let result = generate(&model(&fetch), &call(None)).await;

            assert_eq!(result.language, None);
        }
    }
}
