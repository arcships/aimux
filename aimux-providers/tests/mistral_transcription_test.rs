//! Rust translation of the Mistral transcription model tests.
//!
//! Source: `reference/aisdk-pinned/mistral/src/mistral-transcription-model.test.ts`
//!
//! The requests go through a scripted transport (`common/mock_fetch.rs`); the
//! TS `createTestServer` has no counterpart here, and the multipart body is
//! read back from the bytes the transport saw. Differences from the TS run:
//! the multipart boundary prefix is `----formdata-aimux-` (not `undici`), the
//! pinned `VERSION` mock has no counterpart (the user-agent check only looks at
//! the package prefix), and the response timestamp comes from the clock (no
//! `_internal.currentDate`), so it is only checked to be present. The fixture
//! `__fixtures__/mistral-transcription.json` is inlined.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{MistralProviderSettings, create_mistral};
use mock_fetch::{Canned, MockFetch, Seen};

const MODEL_ID: &str = "voxtral-mini-latest";
const URL: &str = "https://api.mistral.ai/v1/audio/transcriptions";

/// `__fixtures__/mistral-transcription.json`.
fn transcription_response() -> Value {
    json!({
      "model": "voxtral-mini-latest",
      "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies. Named after the Italian astronomer Galileo Galilei, the Galileo spacecraft consisted of an orbiter and an atmospheric entry probe. It was delivered into Earth orbit on October 18, 1989, by Space Shuttle Atlantis on the STS-34 mission, and arrived at Jupiter on December 7, 1995, after gravity assist flybys of Venus and Earth, and became the first spacecraft to orbit Jupiter.",
      "language": null,
      "segments": [
        {
          "type": "transcription_segment",
          "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies.",
          "start": 0.1,
          "end": 8.2,
          "speaker_id": "speaker_1"
        },
        {
          "type": "transcription_segment",
          "text": " Named after the Italian astronomer Galileo Galilei, the Galileo spacecraft consisted of an orbiter and an atmospheric entry probe.",
          "start": 9.1,
          "end": 17.3,
          "speaker_id": "speaker_1"
        },
        {
          "type": "transcription_segment",
          "text": " It was delivered into Earth orbit on October 18, 1989, by Space Shuttle Atlantis on the STS-34 mission, and arrived at Jupiter on December 7, 1995,",
          "start": 18.1,
          "end": 29.9,
          "speaker_id": "speaker_1"
        },
        {
          "type": "transcription_segment",
          "text": " after gravity assist flybys of Venus and Earth, and became the first spacecraft to orbit Jupiter.",
          "start": 30.2,
          "end": 35.9,
          "speaker_id": "speaker_1"
        }
      ],
      "usage": {
        "prompt_audio_seconds": 36,
        "prompt_tokens": 13,
        "completion_tokens": 151,
        "total_tokens": 164,
        "request_count": 1
      }
    })
}

fn provider_with(
    fetch: &Arc<MockFetch>,
    settings: MistralProviderSettings,
) -> aimux_providers::MistralProvider {
    create_mistral(MistralProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(fetch.transport()),
        ..settings
    })
    .unwrap()
}

fn options(audio: AudioInput, media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions {
        audio,
        media_type: media_type.into(),
        provider_options: None,
        abort_signal: None,
        max_retries: None,
        timeout: None,
        headers: None,
    }
}

fn wav(provider_options: Option<Value>) -> TranscriptionCallOptions {
    let mut options = options(AudioInput::Binary(vec![0, 1, 2, 3, 4]), "audio/wav");
    options.provider_options = provider_options
        .map(|value| aimux_core::shared::provider_namespace("mistral", value).unwrap());
    options
}

/// One multipart field as the transport saw it.
struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    body: String,
}

/// The multipart fields of a recorded request (ASCII payloads only).
fn multipart(seen: &Seen) -> Vec<Part> {
    let content_type = &seen.headers["content-type"];
    let boundary = content_type.split("boundary=").nth(1).unwrap();
    let body = String::from_utf8(seen.body.clone()).unwrap();
    body.split(&format!("--{boundary}"))
        .filter(|chunk| chunk.starts_with("\r\n"))
        .map(|chunk| {
            let (head, content) = chunk.trim_start().split_once("\r\n\r\n").unwrap();
            let disposition = |key: &str| {
                head.split(&format!("{key}=\""))
                    .nth(1)
                    .map(|rest| rest.split('"').next().unwrap().to_string())
            };
            Part {
                name: disposition("name").unwrap(),
                filename: disposition("filename"),
                content_type: head
                    .split("\r\n")
                    .find_map(|line| line.strip_prefix("Content-Type: "))
                    .map(str::to_string),
                body: content.strip_suffix("\r\n").unwrap().to_string(),
            }
        })
        .collect()
}

fn values<'a>(parts: &'a [Part], name: &str) -> Vec<&'a str> {
    parts
        .iter()
        .filter(|part| part.name == name)
        .map(|part| part.body.as_str())
        .collect()
}

async fn run(options: TranscriptionCallOptions) -> (Seen, Vec<Part>) {
    let fetch = MockFetch::new(vec![Canned::json(&transcription_response())]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    model.do_generate(&options).await.unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    let parts = multipart(&seen[0]);
    (seen[0].clone(), parts)
}

/// TS: "should expose correct provider and model information"
#[test]
fn should_expose_correct_provider_and_model_information() {
    let fetch = MockFetch::new(vec![]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    assert_eq!(model.provider(), "mistral.transcription");
    assert_eq!(model.model_id(), MODEL_ID);
}

/// TS: "should create transcription models through both provider factories"
#[test]
fn should_create_transcription_models_through_both_provider_factories() {
    let fetch = MockFetch::new(vec![]);
    let provider = provider_with(&fetch, MistralProviderSettings::default());
    assert_eq!(
        provider.transcription(MODEL_ID).provider(),
        "mistral.transcription"
    );
    let model = provider.transcription_model(MODEL_ID).unwrap().unwrap();
    assert_eq!(model.provider(), "mistral.transcription");
}

/// TS: "should send Uint8Array audio as a multipart file"
#[tokio::test]
async fn should_send_uint8array_audio_as_a_multipart_file() {
    let (seen, parts) = run(wav(None)).await;
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.url, URL);
    assert_eq!(values(&parts, "model"), [MODEL_ID]);
    let file = parts.iter().find(|part| part.name == "file").unwrap();
    assert_eq!(file.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(file.filename.as_deref(), Some("audio.wav"));
    assert_eq!(file.body.len(), 5);
}

/// TS: "should send base64 audio with a media-type-derived filename"
#[tokio::test]
async fn should_send_base64_audio_with_a_media_type_derived_filename() {
    let (_, parts) = run(options(AudioInput::Base64("aGVsbG8=".into()), "audio/mp4")).await;
    let file = parts.iter().find(|part| part.name == "file").unwrap();
    assert_eq!(file.content_type.as_deref(), Some("audio/mp4"));
    assert_eq!(file.filename.as_deref(), Some("audio.m4a"));
    assert_eq!(file.body.len(), 5);
}

/// TS: "should pass provider options as Mistral multipart fields"
#[tokio::test]
async fn should_pass_provider_options_as_mistral_multipart_fields() {
    let (_, parts) = run(wav(Some(json!({
        "temperature": 0.2,
        "timestampGranularities": ["segment", "word"],
        "diarize": true,
        "contextBias": ["Vercel", "AI_SDK"],
    }))))
    .await;
    assert_eq!(values(&parts, "temperature"), ["0.2"]);
    assert_eq!(
        values(&parts, "timestamp_granularities"),
        ["segment", "word"]
    );
    assert_eq!(values(&parts, "diarize"), ["true"]);
    assert_eq!(values(&parts, "context_bias"), ["Vercel", "AI_SDK"]);
}

/// TS: "should reject context bias items with commas or whitespace"
#[tokio::test]
async fn should_reject_context_bias_items_with_commas_or_whitespace() {
    let fetch = MockFetch::new(vec![]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    let Err(AiMuxError::InvalidArgument(message)) = model
        .do_generate(&wav(Some(json!({ "contextBias": ["AI SDK"] }))))
        .await
    else {
        panic!("expected an invalid argument error");
    };
    assert!(message.contains("invalid mistral provider options"));
    assert!(fetch.seen().is_empty());
}

/// TS: "should pass language when timestamp granularities are not set"
#[tokio::test]
async fn should_pass_language_when_timestamp_granularities_are_not_set() {
    let (_, parts) = run(wav(Some(json!({ "language": "en" })))).await;
    assert_eq!(values(&parts, "language"), ["en"]);
}

/// TS: "should reject language combined with timestamp granularities"
#[tokio::test]
async fn should_reject_language_combined_with_timestamp_granularities() {
    let fetch = MockFetch::new(vec![]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    let result = model
        .do_generate(&wav(Some(json!({
            "language": "en",
            "timestampGranularities": ["segment"],
        }))))
        .await;
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
    assert!(fetch.seen().is_empty());
}

/// TS: "should pass headers, abort signal, and the Mistral user agent"
#[tokio::test]
async fn should_pass_headers_abort_signal_and_the_mistral_user_agent() {
    let fetch = MockFetch::new(vec![Canned::json(&transcription_response())]);
    let provider = provider_with(
        &fetch,
        MistralProviderSettings {
            headers: Some(HashMap::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
            ..Default::default()
        },
    );
    let mut call = wav(None);
    call.abort_signal = Some(aimux_core::AbortSignal::new());
    call.headers = Some(HashMap::from([(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    )]));
    provider
        .transcription(MODEL_ID)
        .do_generate(&call)
        .await
        .unwrap();

    let headers = &fetch.seen()[0].headers;
    assert_eq!(headers["authorization"], "Bearer test-api-key");
    assert!(headers["content-type"].starts_with("multipart/form-data; boundary=----formdata-"));
    assert_eq!(headers["custom-provider-header"], "provider-header-value");
    assert_eq!(headers["custom-request-header"], "request-header-value");
    assert!(headers["user-agent"].contains("ai-sdk-mistral/"));
}

/// TS: "should use a custom base URL and fetch implementation"
#[tokio::test]
async fn should_use_a_custom_base_url_and_fetch_implementation() {
    let fetch = MockFetch::new(vec![Canned::json(&transcription_response())]);
    let provider = provider_with(
        &fetch,
        MistralProviderSettings {
            base_url: Some("https://custom.mistral.example/v2/".into()),
            ..Default::default()
        },
    );
    provider
        .transcription(MODEL_ID)
        .do_generate(&wav(None))
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://custom.mistral.example/v2/audio/transcriptions"
    );
}

/// TS: "should map transcript fields, usage, diarization, and response metadata"
#[tokio::test]
async fn should_map_transcript_fields_usage_diarization_and_response_metadata() {
    let mut canned = Canned::json(&transcription_response());
    canned
        .headers
        .push(("x-request-id".into(), "test-request-id".into()));
    let fetch = MockFetch::new(vec![canned]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    let result = model.do_generate(&wav(None)).await.unwrap();

    let fixture = transcription_response();
    let fixture_segments = fixture["segments"].as_array().unwrap();
    assert_eq!(result.text, fixture["text"].as_str().unwrap());
    assert_eq!(result.language, None);
    assert_eq!(result.segments.len(), fixture_segments.len());
    for (segment, expected) in result.segments.iter().zip(fixture_segments) {
        assert_eq!(segment.text, expected["text"].as_str().unwrap());
        assert_eq!(segment.start_second, expected["start"].as_f64().unwrap());
        assert_eq!(segment.end_second, expected["end"].as_f64().unwrap());
    }
    assert_eq!(result.duration_in_seconds, Some(36.0));
    assert!(result.warnings.is_empty());
    assert!(result.response.timestamp.is_some());
    assert_eq!(
        result.response.model_id.as_deref(),
        Some("voxtral-mini-latest")
    );
    let headers = result.response.headers.unwrap();
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["x-request-id"], "test-request-id");
    assert_eq!(result.response.body, Some(fixture.clone()));

    let expected_segments: Vec<Value> = fixture_segments
        .iter()
        .map(|segment| {
            json!({
                "type": segment["type"],
                "text": segment["text"],
                "startSecond": segment["start"],
                "endSecond": segment["end"],
                "speakerId": segment["speaker_id"],
            })
        })
        .collect();
    let metadata = result.provider_metadata.unwrap();
    assert_eq!(metadata.len(), 1);
    assert_eq!(
        Value::Object(metadata["mistral"].clone()),
        json!({
            "usage": {
                "promptTokens": 13,
                "completionTokens": 151,
                "totalTokens": 164,
                "promptAudioSeconds": 36,
                "requestCount": 1,
            },
            "segments": expected_segments,
        })
    );
}

/// TS: "should handle nullable response fields and use the last segment for duration"
#[tokio::test]
async fn should_handle_nullable_response_fields_and_use_the_last_segment_for_duration() {
    let fetch = MockFetch::new(vec![Canned::json(&json!({
        "model": "voxtral-mini-transcribe-2602",
        "text": "Hello.",
        "language": null,
        "segments": [{
            "text": "Hello.",
            "start": 0,
            "end": 2.5,
            "score": null,
            "speaker_id": null,
        }],
        "usage": null,
    }))]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).transcription(MODEL_ID);
    let result = model.do_generate(&wav(None)).await.unwrap();

    assert_eq!(result.language, None);
    assert_eq!(result.duration_in_seconds, Some(2.5));
    assert!(result.provider_metadata.is_none());
}
