//! Rust translation of the xAI speech (TTS) model tests.
//!
//! Source: `reference/aisdk-pinned/xai/src/xai-speech-model.test.ts`
//!
//! Each test starts a `wiremock` server, mounts a binary audio response (or
//! the `with_timestamps` JSON envelope) on `POST /tts`, calls `do_generate`
//! and asserts on the request the server saw and on the result.
//!
//! Not ported: the `specificationVersion` check (a TS type tag with no Rust
//! counterpart) and the `_internal.currentDate` injection case (the Rust model
//! has no clock seam; the real-date case covers the timestamp).

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel, SpeechResult};
use aimux_providers::{XAIProviderSettings, XaiSpeechModel, create_xai};

const AUDIO: [u8; 4] = [1, 2, 3, 4];

fn speech_model(
    server: &MockServer,
    headers: Option<HashMap<String, Option<String>>>,
) -> XaiSpeechModel {
    create_xai(XAIProviderSettings {
        api_key: Some("test-key".into()),
        base_url: Some(server.uri()),
        headers,
        ..Default::default()
    })
    .expect("valid settings")
    .speech()
}

/// Start a server answering `POST /tts` with `response`.
async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tts"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn audio_response(content_type: &str, headers: &[(&str, &str)]) -> ResponseTemplate {
    headers.iter().fold(
        ResponseTemplate::new(200)
            .insert_header("content-type", content_type)
            .set_body_bytes(AUDIO.to_vec()),
        |response, (name, value)| response.insert_header(*name, *value),
    )
}

/// The `with_timestamps` JSON envelope; 'AQIDBA==' is `AUDIO` in base64.
fn timestamps_response(headers: &[(&str, &str)]) -> ResponseTemplate {
    headers.iter().fold(
        ResponseTemplate::new(200).set_body_json(json!({
            "audio": "AQIDBA==",
            "content_type": "audio/mpeg",
            "duration": 1.19,
            "audio_timestamps": {
                "graph_chars": ["H", "i"],
                "graph_times": [[0.04, 0.06], [0.06, 0.1]],
            },
        })),
        |response, (name, value)| response.insert_header(*name, *value),
    )
}

fn options(text: &str) -> SpeechCallOptions {
    SpeechCallOptions::new(text)
}

/// `providerOptions: { xai: {...} }`.
fn with_xai(mut options: SpeechCallOptions, xai: Value) -> SpeechCallOptions {
    options.provider_options = Some(HashMap::from([(
        "xai".to_string(),
        xai.as_object().expect("an object").clone(),
    )]));
    options
}

/// The JSON body of the only request the server received.
async fn request_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.expect("requests recorded");
    assert_eq!(requests.len(), 1);
    serde_json::from_slice(&requests[0].body).unwrap()
}

fn xai_metadata(result: &SpeechResult) -> Value {
    Value::Object(result.provider_metadata.as_ref().expect("metadata")["xai"].clone())
}

fn is_unsupported(warnings: &[Warning], expected: &str) -> bool {
    warnings
        .iter()
        .any(|w| matches!(w, Warning::Unsupported { feature, .. } if feature == expected))
}

/// TS: "should expose correct provider and model information"
#[tokio::test]
async fn should_expose_correct_provider_and_model_information() {
    let model = speech_model(&MockServer::start().await, None);
    assert_eq!(model.provider(), "xai.speech");
    assert_eq!(model.model_id(), "");
}

/// TS: "should send text with xAI defaults"
#[tokio::test]
async fn should_send_text_with_xai_defaults() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();

    let expected = json!({
        "text": "Hello from the AI SDK!",
        "voice_id": "eve",
        "language": "auto",
        "output_format": { "codec": "mp3" },
    });
    assert_eq!(request_body(&server).await, expected);
    assert_eq!(result.request.unwrap().body, Some(expected));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(requests[0].url.path(), "/tts");
}

/// TS: "should pass standard speech options"
#[tokio::test]
async fn should_pass_standard_speech_options() {
    let server = server_with(audio_response("audio/wav", &[])).await;
    let mut opts = options("Hello from the AI SDK!");
    opts.voice = Some("ara".into());
    opts.language = Some("en".into());
    opts.output_format = Some("wav".into());
    opts.speed = Some(1.2);
    speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    assert_eq!(
        request_body(&server).await,
        json!({
            "text": "Hello from the AI SDK!",
            "voice_id": "ara",
            "language": "en",
            "output_format": { "codec": "wav" },
            "speed": 1.2,
        })
    );
}

/// TS: "should accept the %s output format"
#[tokio::test]
async fn should_accept_each_output_format() {
    for format in ["mp3", "wav", "pcm", "mulaw", "alaw"] {
        let server = server_with(audio_response("audio/mpeg", &[])).await;
        let mut opts = options("Hello from the AI SDK!");
        opts.output_format = Some(format.into());
        let result = speech_model(&server, None)
            .do_generate(&opts)
            .await
            .unwrap();

        assert_eq!(
            request_body(&server).await["output_format"]["codec"],
            format
        );
        assert!(result.warnings.is_empty(), "{format}");
    }
}

/// TS: "should map provider options onto xAI request fields"
#[tokio::test]
async fn should_map_provider_options_onto_xai_request_fields() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let opts = with_xai(
        options("Hello from the AI SDK!"),
        json!({
            "sampleRate": 44100,
            "bitRate": 192000,
            "optimizeStreamingLatency": 1,
            "textNormalization": true,
        }),
    );
    speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    let body = request_body(&server).await;
    assert_eq!(
        body["output_format"],
        json!({ "codec": "mp3", "sample_rate": 44100, "bit_rate": 192000 })
    );
    assert_eq!(body["optimize_streaming_latency"], 1);
    assert_eq!(body["text_normalization"], true);
}

/// TS: "should pass withTimestamps and replace provider options"
#[tokio::test]
async fn should_pass_with_timestamps_and_replace_provider_options() {
    let server = server_with(timestamps_response(&[])).await;
    let opts = with_xai(
        options("Hello from the AI SDK!"),
        json!({ "withTimestamps": true, "replace": { "nginx": "/ˈɛndʒɪn ˈɛks/" } }),
    );
    speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    let body = request_body(&server).await;
    assert_eq!(body["with_timestamps"], true);
    assert_eq!(body["replace"], json!({ "nginx": "/ˈɛndʒɪn ˈɛks/" }));
}

/// TS: "should decode the with_timestamps envelope and extract provider metadata"
#[tokio::test]
async fn should_decode_the_with_timestamps_envelope_and_extract_provider_metadata() {
    let trace = "993675dc-8ea6-4f54-b4ad-a59ac2615026";
    let server = server_with(timestamps_response(&[("x-trace-id", trace)])).await;
    let opts = with_xai(options("Hi"), json!({ "withTimestamps": true }));
    let result = speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    assert!(matches!(&result.audio, AudioData::Binary(audio) if audio == &AUDIO));
    assert_eq!(
        xai_metadata(&result),
        json!({
            "traceId": trace,
            "duration": 1.19,
            "contentType": "audio/mpeg",
            "audioTimestamps": {
                "graphChars": ["H", "i"],
                "graphTimes": [[0.04, 0.06], [0.06, 0.1]],
            },
        })
    );
    assert_eq!(result.response.body.unwrap()["audio"], "AQIDBA==");
}

/// TS: "should extract the trace id from binary responses"
#[tokio::test]
async fn should_extract_the_trace_id_from_binary_responses() {
    let trace = "06e3dab5-e3ba-4c6b-83a6-1e9ea11d78af";
    let server = server_with(audio_response("audio/mpeg", &[("x-trace-id", trace)])).await;
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert_eq!(xai_metadata(&result), json!({ "traceId": trace }));
}

/// TS: "should return empty provider metadata when xAI headers are absent"
#[tokio::test]
async fn should_return_empty_provider_metadata_when_xai_headers_are_absent() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert_eq!(xai_metadata(&result), json!({}));
}

/// TS: "should warn and use mp3 for unsupported output formats"
#[tokio::test]
async fn should_warn_and_use_mp3_for_unsupported_output_formats() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let mut opts = options("Hello from the AI SDK!");
    opts.output_format = Some("flac".into());
    let result = speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    assert_eq!(request_body(&server).await["output_format"]["codec"], "mp3");
    assert!(is_unsupported(&result.warnings, "outputFormat"));
}

/// TS: "should warn and ignore bitRate for non-mp3 output"
#[tokio::test]
async fn should_warn_and_ignore_bit_rate_for_non_mp3_output() {
    let server = server_with(audio_response("audio/wav", &[])).await;
    let mut opts = with_xai(
        options("Hello from the AI SDK!"),
        json!({ "bitRate": 192000 }),
    );
    opts.output_format = Some("wav".into());
    let result = speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    assert_eq!(
        request_body(&server).await["output_format"],
        json!({ "codec": "wav" })
    );
    assert!(is_unsupported(&result.warnings, "providerOptions"));
}

/// TS: "should warn when instructions are provided"
#[tokio::test]
async fn should_warn_when_instructions_are_provided() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let mut opts = options("Hello from the AI SDK!");
    opts.instructions = Some("Speak cheerfully".into());
    let result = speech_model(&server, None)
        .do_generate(&opts)
        .await
        .unwrap();

    assert!(is_unsupported(&result.warnings, "instructions"));
}

/// TS: "should pass headers and the xAI user agent"
#[tokio::test]
async fn should_pass_headers_and_the_xai_user_agent() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let provider_headers = HashMap::from([(
        "Custom-Provider-Header".to_string(),
        Some("provider-header-value".to_string()),
    )]);
    let mut opts = options("Hello from the AI SDK!");
    opts.headers = Some(HashMap::from([(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    )]));
    speech_model(&server, Some(provider_headers))
        .do_generate(&opts)
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let header = |name: &str| requests[0].headers.get(name).unwrap().to_str().unwrap();
    assert_eq!(header("authorization"), "Bearer test-key");
    assert_eq!(header("content-type"), "application/json");
    assert_eq!(header("custom-provider-header"), "provider-header-value");
    assert_eq!(header("custom-request-header"), "request-header-value");
    assert!(header("user-agent").contains("ai-sdk-xai/5.0.12"));
}

/// TS: "should return binary audio data"
#[tokio::test]
async fn should_return_binary_audio_data() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert!(matches!(&result.audio, AudioData::Binary(audio) if audio == &AUDIO));
    assert!(result.warnings.is_empty());
}

/// TS: "should include response timestamp, model id, and headers"
#[tokio::test]
async fn should_include_response_timestamp_model_id_and_headers() {
    let server = server_with(audio_response(
        "audio/mpeg",
        &[("x-request-id", "test-request-id")],
    ))
    .await;
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();

    assert_eq!(result.response.model_id.as_deref(), Some(""));
    assert_eq!(
        result
            .response
            .headers
            .unwrap()
            .get("x-request-id")
            .map(String::as_str),
        Some("test-request-id")
    );
    assert!(result.response.timestamp.is_some());
}

/// TS: "should handle API errors"
#[tokio::test]
async fn should_handle_api_errors() {
    let server = server_with(
        ResponseTemplate::new(400).set_body_string(
            json!({ "error": { "message": "Invalid text", "type": "invalid_request_error" } })
                .to_string(),
        ),
    )
    .await;
    let error = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap_err();

    let AiMuxError::ApiCall(error) = error else {
        panic!("expected an ApiCall error, got {error:?}");
    };
    assert_eq!(error.message, "Invalid text");
    assert_eq!(error.status_code, Some(400));
}

/// TS: "should use the real date when no custom date provider is specified"
#[tokio::test]
async fn should_use_the_real_date_when_no_custom_date_provider_is_specified() {
    let server = server_with(audio_response("audio/mpeg", &[])).await;
    let before = chrono::Utc::now();
    let result = speech_model(&server, None)
        .do_generate(&options("Hello from the AI SDK!"))
        .await
        .unwrap();
    let after = chrono::Utc::now();

    let timestamp = chrono::DateTime::parse_from_rfc3339(&result.response.timestamp.unwrap())
        .expect("an RFC 3339 timestamp");
    assert!(before <= timestamp && timestamp <= after);
    assert_eq!(result.response.model_id.as_deref(), Some(""));
}
