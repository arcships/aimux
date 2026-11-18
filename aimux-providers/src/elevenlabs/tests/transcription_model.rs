//! Port of `@ai-sdk/elevenlabs@3.0.54`
//! `src/elevenlabs-transcription-model.test.ts`, `doGenerate` part.
//!
//! Harness: upstream sends `transcript-test.mp3` to a test server at
//! `https://api.elevenlabs.io/v1/speech-to-text`; here a [`MockFetch`]
//! answers with the same fixture, and a few stand-in bytes are the audio (no
//! case reads it back). `requestBodyMultipart` is read back from the
//! recorded multipart body: the fields by name, a later field of a name
//! replacing an earlier one as `Object.fromEntries` of the form does. The
//! `customModel` cases build the model from a config with a fixed endpoint,
//! as upstream builds `new ElevenLabsTranscriptionModel(...)`. Upstream
//! injects the date (`_internal.currentDate`); aimux reads the clock, so
//! timestamps are checked to fall within the call.
//!
//! The `doStream` cases are sub-features in layout.toml (realtime
//! transcription keeps the RFC-0034 D5 surface).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::transcription_model::{
    AudioInput, TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
};
use aimux_provider_utils::{HeaderMapOpt, Resolvable};

use crate::elevenlabs::{
    ElevenLabsProviderSettings, ElevenLabsTranscriptionModel, create_elevenlabs,
};
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

/// `prepareJsonFixtureResponse('elevenlabs-transcription', headers)`.
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/elevenlabs-transcription.json")).unwrap()
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

/// `createElevenLabs({ apiKey: 'test-api-key', headers }).transcription(model_id)`.
fn provider_model(
    fetch: &Arc<MockFetch>,
    headers: Option<HeaderMapOpt>,
    model_id: &str,
) -> ElevenLabsTranscriptionModel {
    create_elevenlabs(ElevenLabsProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        headers,
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription(model_id)
}

/// `provider.transcription('scribe_v1')`.
fn model(fetch: &Arc<MockFetch>) -> ElevenLabsTranscriptionModel {
    provider_model(fetch, None, "scribe_v1")
}

/// The base URL and `headers: () => ({})` of upstream's `customModel`.
struct StaticEndpoint;

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: "https://api.elevenlabs.io".to_string(),
            headers: HeaderMapOpt::new(),
        })
    }
}

/// `new ElevenLabsTranscriptionModel('scribe_v1', { provider: 'test-provider',
/// url: () => 'https://api.elevenlabs.io/v1/speech-to-text', headers: () => ({}) })`.
fn custom_model(fetch: &Arc<MockFetch>) -> ElevenLabsTranscriptionModel {
    ElevenLabsTranscriptionModel::from_config(
        "scribe_v1".to_string(),
        EndpointConfig::new(
            "test-provider".to_string(),
            StaticEndpoint,
            Some(fetch.transport()),
        ),
        #[cfg(feature = "realtime")]
        None,
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
    model: &ElevenLabsTranscriptionModel,
    call: &TranscriptionCallOptions,
) -> TranscriptionResult {
    model.do_generate(call).await.unwrap()
}

/// `await server.calls[0].requestBodyMultipart`.
#[derive(Debug, Default)]
struct Multipart {
    /// The text fields; a later field of a name replaces an earlier one.
    fields: BTreeMap<String, String>,
    /// The file parts: name to filename.
    files: BTreeMap<String, String>,
}

fn multipart(seen: &Seen) -> Multipart {
    let content_type = &seen.headers["content-type"];
    let boundary = content_type
        .split_once("boundary=")
        .map(|(_, boundary)| boundary)
        .expect("a multipart boundary");
    let body = String::from_utf8_lossy(&seen.body).into_owned();
    let mut form = Multipart::default();
    for part in body.split(&format!("--{boundary}")) {
        let Some((head, value)) = part.split_once("\r\n\r\n") else {
            continue;
        };
        let attribute = |key: &str| {
            head.split_once(&format!("; {key}=\""))
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(value, _)| value.to_string())
        };
        let name = attribute("name").expect("a part name");
        match attribute("filename") {
            Some(filename) => {
                form.files.insert(name, filename);
            }
            None => {
                let value = value.strip_suffix("\r\n").unwrap_or(value);
                form.fields.insert(name, value.to_string());
            }
        }
    }
    form
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › should reject scribe_v2_realtime for non-streaming transcription
    #[tokio::test]
    async fn should_reject_scribe_v2_realtime_for_non_streaming_transcription() {
        let fetch = MockFetch::new(vec![]);
        let error = provider_model(&fetch, None, "scribe_v2_realtime")
            .do_generate(&call(None))
            .await
            .unwrap_err();

        assert!(
            matches!(error, AiMuxError::UnsupportedFunctionality(_)),
            "{error:?}"
        );
        assert!(fetch.seen().is_empty());
    }

    /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › warns when streaming options are passed to batch transcription
    #[tokio::test]
    async fn warns_when_streaming_options_are_passed_to_batch_transcription() {
        let fetch = fixture_fetch();
        let result = generate(
            &model(&fetch),
            &call(Some(json!({
                "elevenlabs": { "streaming": { "includeTimestamps": true } },
            }))),
        )
        .await;

        assert_eq!(
            result.warnings,
            vec![Warning::Unsupported {
                feature: "providerOptions.elevenlabs.streaming".to_string(),
                details: Some(
                    "ElevenLabs batch transcription does not support streaming options."
                        .to_string()
                ),
            }]
        );
    }

    mod transcription {
        use super::*;

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › transcription › should pass the model
        #[tokio::test]
        async fn should_pass_the_model() {
            let fetch = fixture_fetch();
            generate(&model(&fetch), &call(None)).await;

            let form = multipart(&fetch.seen()[0]);
            assert_eq!(form.fields["model_id"], "scribe_v1");
        }

        /// partial: `doGenerate › transcription › should pass headers`; upstream also asserts that `requestUserAgent` contains `ai-sdk-elevenlabs/0.0.0-test`; that user-agent part is not sent on this branch (#444 adds it with its own test). Only the headers are compared here, the boundary being aimux's rather than undici's.
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
                "scribe_v1",
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
            assert_eq!(headers["xi-api-key"], "test-api-key");
            assert!(
                headers["content-type"].starts_with("multipart/form-data; boundary="),
                "{}",
                headers["content-type"]
            );
            assert_eq!(headers["custom-provider-header"], "provider-header-value");
            assert_eq!(headers["custom-request-header"], "request-header-value");
        }

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › transcription › should extract the transcription text
        #[tokio::test]
        async fn should_extract_the_transcription_text() {
            let fetch = fixture_fetch();
            let result = generate(&model(&fetch), &call(None)).await;

            assert_eq!(result.text, "Hello from the Vercel AI SDK.");
        }

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › transcription › should pass provider options correctly
        #[tokio::test]
        async fn should_pass_provider_options_correctly() {
            let fetch = fixture_fetch();
            generate(
                &model(&fetch),
                &call(Some(json!({ "elevenlabs": {
                    "languageCode": "en",
                    "fileFormat": "pcm_s16le_16",
                    "tagAudioEvents": false,
                    "numSpeakers": 2,
                    "timestampsGranularity": "character",
                    "diarize": true,
                } }))),
            )
            .await;

            let form = multipart(&fetch.seen()[0]);
            assert!(form.files.contains_key("file"), "{form:?}");
            assert_eq!(
                form.fields,
                [
                    ("diarize", "true"),
                    ("file_format", "pcm_s16le_16"),
                    ("language_code", "en"),
                    ("model_id", "scribe_v1"),
                    ("num_speakers", "2"),
                    ("tag_audio_events", "false"),
                    ("timestamps_granularity", "character"),
                ]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>()
            );
        }
    }

    mod response_headers {
        use super::*;

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › response headers › should include response data with timestamp, modelId and headers
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
            assert_eq!(headers["content-type"], "application/json");
            assert_eq!(headers["x-request-id"], "test-request-id");
            assert_eq!(headers["x-ratelimit-remaining"], "123");
            assert_eq!(response.model_id.as_deref(), Some("scribe_v1"));
            let timestamp = response.timestamp.expect("a timestamp");
            assert!(before <= timestamp && timestamp <= after);
        }
    }

    mod response_metadata {
        use super::*;

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › response metadata › should use real date when no custom date provider is specified
        #[tokio::test]
        async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
            let fetch = fixture_fetch();
            let before = aimux_core::Timestamp::now();
            let result = generate(&custom_model(&fetch), &call(None)).await;
            let after = aimux_core::Timestamp::now();

            let timestamp = result.response.timestamp.expect("a timestamp");
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(result.response.model_id.as_deref(), Some("scribe_v1"));
        }
    }

    mod no_additional_formats {
        use super::*;

        /// upstream: @ai-sdk/elevenlabs src/elevenlabs-transcription-model.test.ts › doGenerate › no additional formats › should work when no additional formats are returned
        ///
        /// Upstream snapshots the whole result: the text, one segment per
        /// word or spacing entry, the language, the duration (the last
        /// entry's end), no warnings and the response.
        #[tokio::test]
        async fn should_work_when_no_additional_formats_are_returned() {
            let fetch = fixture_fetch();
            let result = generate(&custom_model(&fetch), &call(None)).await;

            assert_eq!(result.text, "Hello from the Vercel AI SDK.");
            assert_eq!(
                result
                    .segments
                    .iter()
                    .map(|segment| (
                        segment.text.as_str(),
                        segment.start_second,
                        segment.end_second
                    ))
                    .collect::<Vec<_>>(),
                vec![
                    ("Hello", 0.199, 0.479),
                    (" ", 0.479, 0.499),
                    ("from", 0.5, 0.639),
                    (" ", 0.639, 0.66),
                    ("the", 0.66, 0.759),
                    (" ", 0.759, 0.759),
                    ("Vercel", 0.759, 1.12),
                    (" ", 1.12, 1.2),
                    ("AI", 1.2, 1.519),
                    (" ", 1.519, 1.58),
                    ("SDK.", 1.58, 2.479),
                ]
            );
            assert_eq!(result.language.as_deref(), Some("eng"));
            assert_eq!(result.duration_in_seconds, Some(2.479));
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert_eq!(result.response.body, Some(fixture()));
            assert_eq!(result.response.model_id.as_deref(), Some("scribe_v1"));
        }
    }
}
