//! Rust translation of the AssemblyAI transcription model tests.
//! Source: `reference/ai/packages/assemblyai/src/assemblyai-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{AssemblyAIConfig, AssemblyAIProvider};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

async fn mock_all(server: &MockServer, transcript: &Value, headers: &[(&str, &str)]) {
    // Upload
    Mock::given(method("POST"))
        .and(path("/v2/upload"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"upload_url": "https://upload.assemblyai.com/test"})),
        )
        .mount(server)
        .await;
    // Submit
    Mock::given(method("POST"))
        .and(path("/v2/transcript"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": "test-id", "status": "queued"})),
        )
        .mount(server)
        .await;
    // Poll result (completed on first poll)
    let mut t = ResponseTemplate::new(200).set_body_json(transcript.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/v2/transcript/test-id"))
        .respond_with(t)
        .mount(server)
        .await;
}

fn fixture_transcript() -> Value {
    json!({
        "id": "test-id",
        "status": "completed",
        "text": "Hello from AssemblyAI.",
        "language_code": "en",
        "audio_duration": 5.0,
        "words": [
            {"text": "Hello", "start": 0.0, "end": 500.0},
            {"text": " from AssemblyAI.", "start": 500.0, "end": 2000.0}
        ]
    })
}

#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_transcript(), &[]).await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from AssemblyAI.");
    assert_eq!(result.language, Some("en".to_string()));
    assert_eq!(result.duration_in_seconds, Some(5.0));
    assert_eq!(result.segments.len(), 2);
    // Words are in milliseconds, segments in seconds
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 0.5);
}

#[tokio::test]
async fn should_pass_model_as_speech_models() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_transcript(), &[]).await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    // Submit request is the second one (index 1)
    let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(body["speech_models"], json!(["universal-2"]));
}

#[tokio::test]
async fn should_use_speech_model_for_best() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_transcript(), &[]).await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("best");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(body["speech_model"], "best");
    assert!(!result.warnings.is_empty());
}

#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_transcript(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = AssemblyAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_headers(ph);
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let mut opts = options(mock_audio(), "audio/wav");
    let mut rh = HashMap::new();
    rh.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    opts.headers = Some(rh);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "test-api-key");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_all(
        &server,
        &fixture_transcript(),
        &[("x-request-id", "test-req")],
    )
    .await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("universal-2".to_string()));
}

#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_all(&server, &fixture_transcript(), &[]).await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("universal-2".to_string()));
}

#[tokio::test]
async fn should_include_provider_metadata_with_utterances() {
    let server = MockServer::start().await;
    let transcript = json!({
        "id": "test-id",
        "status": "completed",
        "text": "Hello",
        "language_code": "en",
        "words": [{"text": "Hello", "start": 0.0, "end": 500.0}],
        "utterances": [{"text": "Hello", "start": 0, "end": 500, "speaker": "A"}]
    });
    mock_all(&server, &transcript, &[]).await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.provider_metadata.is_some());
    let md = result.provider_metadata.as_ref().unwrap();
    let aai = md.get("assemblyai").unwrap();
    assert!(aai.get("utterances").is_some());
}

/// A transient 503 on the transcript-submit stage, followed by 200, must
/// succeed without re-uploading the audio: the upload exchange is retried
/// independently of the submit exchange, so Core's outer per-`do_generate`
/// retry never needs to (and must not) replay the upload.
#[tokio::test]
async fn transient_submit_failure_is_retried_without_re_uploading() {
    let server = MockServer::start().await;

    let upload_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upload_observed = std::sync::Arc::clone(&upload_attempts);
    Mock::given(method("POST"))
        .and(path("/v2/upload"))
        .respond_with(move |_: &wiremock::Request| {
            upload_observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200)
                .set_body_json(json!({"upload_url": "https://upload.assemblyai.com/test"}))
        })
        .mount(&server)
        .await;

    let submit_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let submit_observed = std::sync::Arc::clone(&submit_attempts);
    Mock::given(method("POST"))
        .and(path("/v2/transcript"))
        .respond_with(move |_: &wiremock::Request| {
            if submit_observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({"error": "try again"}))
            } else {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "test-id", "status": "queued"}))
            }
        })
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/transcript/test-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture_transcript()))
        .mount(&server)
        .await;

    let config = AssemblyAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = AssemblyAIProvider::new(config);
    let model = provider.transcription("universal-2");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from AssemblyAI.");
    assert_eq!(
        upload_attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the upload stage must not be replayed by a submit-stage retry"
    );
    assert_eq!(submit_attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}
