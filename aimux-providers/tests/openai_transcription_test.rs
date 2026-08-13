//! Rust translation of the OpenAI transcription (STT) model tests.
//!
//! Source: `reference/ai/packages/openai/src/transcription/openai-transcription-model.test.ts`
//!
//! Each test spins up a `wiremock` mock server, configures a JSON response,
//! creates an `OpenAITranscriptionModel` pointing at the mock, calls
//! `do_generate`, and asserts on the multipart request body / headers / result.
//!
//! The TS tests inject a custom `currentDate` via `_internal` for timestamp
//! assertions. The Rust model always uses `Utc::now()`; the timestamp tests
//! therefore verify that a timestamp is present and that `model_id` matches,
//! rather than asserting an exact timestamp value.
//!
//! The TS `doStream` tests use a mock WebSocket which is not practical to
//! translate to Rust (the Rust port does not implement realtime WebSocket
//! streaming). Those tests are omitted; the `do_stream` trait default returns
//! `AiMuxError::UnsupportedFunctionality`.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::{OpenAIConfig, OpenAIProvider};

// ── helpers ─────────────────────────────────────────────────────────────────

/// Mock audio data (same as TS `audioData` — just some bytes).
fn mock_audio_data() -> Vec<u8> {
    vec![1u8, 2, 3, 4, 5]
}

/// Build `TranscriptionCallOptions` with binary audio and a media type.
fn transcription_options(audio: Vec<u8>, media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), media_type)
}

/// Mount a mock JSON transcription response on the server at
/// `/audio/transcriptions`.
async fn mock_json_response(server: &MockServer, body: &Value) {
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body.clone()),
        )
        .mount(server)
        .await;
}

/// Mount a mock JSON transcription response with extra response headers.
async fn mock_json_response_with_headers(
    server: &MockServer,
    body: &Value,
    headers: &[(&str, &str)],
) {
    let mut template = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        template = template.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Parse a multipart/form-data body into a map of field name → value (string).
///
/// Non-file fields are decoded as UTF-8 strings. File fields are returned as
/// the raw bytes converted to a string (for assertion purposes).
fn parse_multipart_form(body: &[u8]) -> HashMap<String, String> {
    let body_str = String::from_utf8_lossy(body);
    let mut form = HashMap::new();

    // The first line is the boundary with -- prefix.
    // Find the boundary.
    let boundary = body_str
        .lines()
        .next()
        .unwrap_or("")
        .trim_start_matches('-');
    if boundary.is_empty() {
        return form;
    }

    // Split on the boundary.
    for part in body_str.split(boundary) {
        let part = part.trim_matches(|c| c == '\r' || c == '\n' || c == '-');
        if part.is_empty() || part == "--" {
            continue;
        }

        // Each part has headers and a body separated by \r\n\r\n.
        if let Some(header_end) = part.find("\r\n\r\n") {
            let headers = &part[..header_end];
            let value = &part[header_end + 4..];

            // Extract the field name from Content-Disposition.
            if let Some(name_start) = headers.find("name=\"") {
                let name_start = name_start + 6;
                if let Some(name_end) = headers[name_start..].find('"') {
                    let name = &headers[name_start..name_start + name_end];
                    form.insert(name.to_string(), value.trim_end_matches('\r').to_string());
                }
            }
        }
    }

    form
}

/// The full OpenAI transcription fixture response (matches the TS fixture).
fn fixture_response() -> Value {
    json!({
        "task": "transcribe",
        "language": "english",
        "duration": 36.709999084472656,
        "text": "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies.",
        "words": [
            {"word": "Galileo", "start": 0.0, "end": 0.6600000262260437},
            {"word": "was", "start": 0.6600000262260437, "end": 0.8999999761581421},
            {"word": "an", "start": 0.8999999761581421, "end": 1.1399999856948853},
            {"word": "American", "start": 1.1399999856948853, "end": 1.5},
            {"word": "robotic", "start": 1.5, "end": 2.0199999809265137}
        ],
        "usage": {"type": "duration", "seconds": 37}
    })
}

// ════════════════════════════════════════════════════════════════════════════
// doGenerate
// (openai-transcription-model.test.ts → describe('doGenerate'))
// ════════════════════════════════════════════════════════════════════════════

/// TS: "should reject gpt-realtime-whisper for non-streaming transcription"
#[tokio::test]
async fn should_reject_gpt_realtime_whisper_for_non_streaming() {
    let server = MockServer::start().await;
    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("gpt-realtime-whisper");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        AiMuxError::UnsupportedFunctionality(msg) => {
            assert!(msg.contains("gpt-realtime-whisper"));
        }
        e => panic!("expected Unsupported error, got: {e:?}"),
    }
}

/// TS: "should pass the model"
#[tokio::test]
async fn should_pass_the_model() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let form = parse_multipart_form(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "whisper-1");
}

/// TS: "should default whisper-1 to verbose_json response format"
#[tokio::test]
async fn should_default_whisper_1_to_verbose_json() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let form = parse_multipart_form(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "whisper-1");
    assert_eq!(form.get("response_format").unwrap(), "verbose_json");
    assert_eq!(result.duration_in_seconds, Some(36.709999084472656));
}

/// TS: "should pass headers"
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let mut provider_headers = HashMap::new();
    provider_headers.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = OpenAIConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_org_id("test-organization")
        .with_project("test-project")
        .with_headers(provider_headers);
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut request_headers = HashMap::new();
    request_headers.insert(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    );
    options.headers = Some(request_headers);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(h.get("authorization").unwrap(), "Bearer test-api-key");
    assert_eq!(h.get("openai-organization").unwrap(), "test-organization");
    assert_eq!(h.get("openai-project").unwrap(), "test-project");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// TS: "should extract the transcription text"
#[tokio::test]
async fn should_extract_the_transcription_text() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(
        result.text,
        "Galileo was an American robotic space program that studied the planet Jupiter and its moons, as well as several other solar system bodies."
    );
}

/// TS: "should include response data with timestamp, modelId and headers"
#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_json_response_with_headers(
        &server,
        &fixture_response(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert_eq!(headers.get("x-request-id").unwrap(), "test-request-id");
    assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "123");
}

/// TS: "should use real date when no custom date provider is specified"
#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
}

/// TS: "should pass response_format when `providerOptions.openai.timestampGranularities` is set"
#[tokio::test]
async fn should_pass_response_format_with_timestamp_granularities_word() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({"timestampGranularities": ["word"]}),
    );
    options.provider_options = Some(po);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let form = parse_multipart_form(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "whisper-1");
    assert_eq!(form.get("response_format").unwrap(), "verbose_json");
    assert_eq!(form.get("temperature").unwrap(), "0");
    assert_eq!(form.get("timestamp_granularities[]").unwrap(), "word");
}

/// TS: "should not set pass response_format to "verbose_json" when model is "gpt-4o-transcribe""
#[tokio::test]
async fn should_use_json_for_gpt4o_transcribe() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("gpt-4o-transcribe");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({"timestampGranularities": ["word"]}),
    );
    options.provider_options = Some(po);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let form = parse_multipart_form(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "gpt-4o-transcribe");
    assert_eq!(form.get("response_format").unwrap(), "json");
    assert_eq!(form.get("temperature").unwrap(), "0");
    assert_eq!(form.get("timestamp_granularities[]").unwrap(), "word");
}

/// TS: "should pass timestamp_granularities when specified"
#[tokio::test]
async fn should_pass_timestamp_granularities_segment() {
    let server = MockServer::start().await;
    mock_json_response(&server, &fixture_response()).await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({"timestampGranularities": ["segment"]}),
    );
    options.provider_options = Some(po);

    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    let form = parse_multipart_form(&requests[0].body);
    assert_eq!(form.get("model").unwrap(), "whisper-1");
    assert_eq!(form.get("response_format").unwrap(), "verbose_json");
    assert_eq!(form.get("temperature").unwrap(), "0");
    assert_eq!(form.get("timestamp_granularities[]").unwrap(), "segment");
}

/// TS: "should work when no words, language, or duration are returned"
#[tokio::test]
async fn should_work_without_optional_fields() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        &json!({
            "task": "transcribe",
            "text": "Hello from the Vercel AI SDK!",
            "_request_id": "req_1234"
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.duration_in_seconds, None);
    assert_eq!(result.language, None);
    assert!(result.segments.is_empty());
    assert_eq!(result.text, "Hello from the Vercel AI SDK!");
    assert!(result.warnings.is_empty());
    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("whisper-1".to_string()));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
}

/// TS: "should parse segments when provided in response"
#[tokio::test]
async fn should_parse_segments() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        &json!({
            "task": "transcribe",
            "text": "Hello world. How are you?",
            "segments": [
                {
                    "id": 0, "seek": 0, "start": 0.0, "end": 2.5,
                    "text": "Hello world.", "tokens": [1234, 5678],
                    "temperature": 0.0, "avg_logprob": -0.5,
                    "compression_ratio": 1.2, "no_speech_prob": 0.1
                },
                {
                    "id": 1, "seek": 250, "start": 2.5, "end": 5.0,
                    "text": " How are you?", "tokens": [9012, 3456],
                    "temperature": 0.0, "avg_logprob": -0.6,
                    "compression_ratio": 1.1, "no_speech_prob": 0.05
                }
            ],
            "language": "english",
            "duration": 5.0,
            "_request_id": "req_1234"
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({"timestampGranularities": ["segment"]}),
    );
    options.provider_options = Some(po);

    let result = model.do_generate(&options).await.unwrap();

    assert_eq!(result.segments.len(), 2);
    assert_eq!(result.segments[0].text, "Hello world.");
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 2.5);
    assert_eq!(result.segments[1].text, " How are you?");
    assert_eq!(result.segments[1].start_second, 2.5);
    assert_eq!(result.segments[1].end_second, 5.0);
    assert_eq!(result.text, "Hello world. How are you?");
    assert_eq!(result.duration_in_seconds, Some(5.0));
    assert_eq!(result.language, Some("en".to_string()));
}

/// TS: "should fallback to words when segments are not available"
#[tokio::test]
async fn should_fallback_to_words() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        &json!({
            "task": "transcribe",
            "text": "Hello world",
            "words": [
                {"word": "Hello", "start": 0.0, "end": 1.0},
                {"word": "world", "start": 1.0, "end": 2.0}
            ],
            "language": "english",
            "duration": 2.0,
            "_request_id": "req_1234"
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let mut options = transcription_options(mock_audio_data(), "audio/wav");
    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({"timestampGranularities": ["word"]}),
    );
    options.provider_options = Some(po);

    let result = model.do_generate(&options).await.unwrap();

    assert_eq!(result.segments.len(), 2);
    assert_eq!(result.segments[0].text, "Hello");
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 1.0);
    assert_eq!(result.segments[1].text, "world");
    assert_eq!(result.segments[1].start_second, 1.0);
    assert_eq!(result.segments[1].end_second, 2.0);
}

/// TS: "should handle empty segments array"
#[tokio::test]
async fn should_handle_empty_segments() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        &json!({
            "task": "transcribe",
            "text": "Hello world",
            "segments": [],
            "language": "english",
            "duration": 2.0,
            "_request_id": "req_1234"
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.segments.is_empty());
    assert_eq!(result.text, "Hello world");
}

/// TS: "should handle segments with missing optional fields"
#[tokio::test]
async fn should_handle_segments_with_missing_optional_fields() {
    let server = MockServer::start().await;
    mock_json_response(
        &server,
        &json!({
            "task": "transcribe",
            "text": "Test",
            "segments": [
                {
                    "id": 0, "seek": 0, "start": 0.0, "end": 1.0,
                    "text": "Test", "tokens": [1234],
                    "temperature": 0.0, "avg_logprob": -0.5,
                    "compression_ratio": 1.0, "no_speech_prob": 0.1
                }
            ],
            "_request_id": "req_1234"
        }),
    )
    .await;

    let config = OpenAIConfig::new("test-api-key").with_base_url(server.uri());
    let provider = OpenAIProvider::new(config);
    let model = provider.transcription("whisper-1");

    let result = model
        .do_generate(&transcription_options(mock_audio_data(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].text, "Test");
    assert_eq!(result.segments[0].start_second, 0.0);
    assert_eq!(result.segments[0].end_second, 1.0);
    assert_eq!(result.language, None);
    assert_eq!(result.duration_in_seconds, None);
}
