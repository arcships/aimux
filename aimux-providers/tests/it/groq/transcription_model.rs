//! Port of the Groq transcription model tests.
//! Upstream: `packages/groq/src/groq-transcription-model.test.ts`
//! (fixture `__fixtures__/groq-transcription-text.json`).
//!
//! The upstream `_internal.currentDate` hook has no Rust counterpart, so the
//! two date cases check that the timestamp is the clock's time during the call
//! instead of a fixed date.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Timestamp;
use aimux_core::shared::SharedProviderOptions;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_providers::groq::{GroqProviderSettings, create_groq};

/// Stand-in for `transcript-test.mp3`.
const AUDIO: [u8; 4] = [0x49, 0x44, 0x33, 0x04];

fn call(media_type: &str, groq: Option<Value>) -> TranscriptionCallOptions {
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(AUDIO.to_vec()), media_type);
    if let Some(Value::Object(groq)) = groq {
        options.provider_options = Some(SharedProviderOptions::from([("groq".to_string(), groq)]));
    }
    options
}

fn json_body() -> Value {
    json!({
        "task": "transcribe",
        "language": "English",
        "duration": 2.5,
        "text": "Hello world!",
        "segments": [{
            "id": 0, "seek": 0, "start": 0, "end": 2.48, "text": "Hello world!",
            "tokens": [50365, 2425, 490, 264], "temperature": 0,
            "avg_logprob": -0.29010406, "compression_ratio": 0.7777778,
            "no_speech_prob": 0.032802984
        }],
        "x_groq": { "id": "req_01jrh9nn61f24rydqq1r4b3yg5" }
    })
}

async fn serve(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn model_on(
    server: &MockServer,
    headers: Option<HashMap<String, Option<String>>>,
    id: &str,
) -> impl TranscriptionModel {
    create_groq(GroqProviderSettings {
        api_key: Some("test-api-key".to_string()),
        base_url: Some(server.uri()),
        headers,
        ..Default::default()
    })
    .unwrap()
    .transcription(id)
}

/// The value of a multipart text field of the first request.
async fn field(server: &MockServer, name: &str) -> Option<String> {
    let body = server.received_requests().await.unwrap()[0].body.clone();
    let body = String::from_utf8_lossy(&body).into_owned();
    let marker = format!("name=\"{name}\"\r\n\r\n");
    let start = body.find(&marker)? + marker.len();
    Some(body[start..body[start..].find("\r\n")? + start].to_string())
}

mod do_generate {
    use super::*;

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should pass the model
    #[tokio::test]
    async fn should_pass_the_model() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        model.do_generate(&call("audio/wav", None)).await.unwrap();

        assert_eq!(
            field(&server, "model").await.as_deref(),
            Some("whisper-large-v3-turbo")
        );
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
        let model = model_on(
            &server,
            Some(HashMap::from([(
                "Custom-Provider-Header".to_string(),
                Some("provider-header-value".to_string()),
            )])),
            "whisper-large-v3-turbo",
        );
        let mut options = call("audio/wav", None);
        options.headers = Some(HashMap::from([(
            "Custom-Request-Header".to_string(),
            "request-header-value".to_string(),
        )]));

        model.do_generate(&options).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let headers = &requests[0].headers;
        assert_eq!(headers.get("authorization").unwrap(), "Bearer test-api-key");
        assert!(
            headers
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("multipart/form-data; boundary=")
        );
        assert_eq!(
            headers.get("custom-provider-header").unwrap(),
            "provider-header-value"
        );
        assert_eq!(
            headers.get("custom-request-header").unwrap(),
            "request-header-value"
        );
        assert!(
            headers
                .get("user-agent")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("ai-sdk-groq/")
        );
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should extract the transcription text
    #[tokio::test]
    async fn should_extract_the_transcription_text() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        let result = model.do_generate(&call("audio/wav", None)).await.unwrap();

        assert_eq!(result.text, "Hello world!");
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should extract a plain-text transcription response
    #[tokio::test]
    async fn should_extract_a_plain_text_transcription_response() {
        let body = " Hello from the Versal AISDK.";
        let server = serve(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain; charset=UTF-8")
                .set_body_bytes(body.as_bytes()),
        )
        .await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        let result = model
            .do_generate(&call(
                "audio/wav",
                Some(json!({ "responseFormat": "text" })),
            ))
            .await
            .unwrap();

        assert_eq!(
            field(&server, "response_format").await.as_deref(),
            Some("text")
        );
        assert_eq!(result.text, body);
        assert_eq!(result.response.body, Some(Value::String(body.to_string())));
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
    #[tokio::test]
    async fn should_include_response_data_with_timestamp_model_id_and_headers() {
        let server = serve(
            ResponseTemplate::new(200)
                .insert_header("x-request-id", "test-request-id")
                .insert_header("x-ratelimit-remaining", "123")
                .set_body_json(json_body()),
        )
        .await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        let before = Timestamp::now();
        let result = model.do_generate(&call("audio/wav", None)).await.unwrap();
        let after = Timestamp::now();

        let timestamp = result.response.timestamp.unwrap();
        assert!(before <= timestamp && timestamp <= after);
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("whisper-large-v3-turbo")
        );
        let headers = result.response.headers.unwrap();
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["x-request-id"], "test-request-id");
        assert_eq!(headers["x-ratelimit-remaining"], "123");
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should use real date when no custom date provider is specified
    #[tokio::test]
    async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        let before = Timestamp::now();
        let result = model.do_generate(&call("audio/wav", None)).await.unwrap();
        let after = Timestamp::now();

        let timestamp = result.response.timestamp.unwrap();
        assert!(before <= timestamp && timestamp <= after);
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("whisper-large-v3-turbo")
        );
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should correctly pass provider options when they are an array
    #[tokio::test]
    async fn should_correctly_pass_provider_options_when_they_are_an_array() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
        let model = model_on(&server, None, "whisper-large-v3-turbo");

        model
            .do_generate(&call(
                "audio/wav",
                Some(json!({
                    "timestampGranularities": ["segment"],
                    "responseFormat": "verbose_json"
                })),
            ))
            .await
            .unwrap();

        assert_eq!(
            field(&server, "timestamp_granularities[]").await.as_deref(),
            Some("segment")
        );
        assert_eq!(
            field(&server, "response_format").await.as_deref(),
            Some("verbose_json")
        );
    }

    /// upstream: @ai-sdk/groq src/groq-transcription-model.test.ts › doGenerate › should fallback to words when segments are not available
    #[tokio::test]
    async fn should_fallback_to_words_when_segments_are_not_available() {
        let server = serve(ResponseTemplate::new(200).set_body_json(json!({
            "task": "transcribe",
            "language": "English",
            "duration": 2,
            "text": "Hello world",
            "segments": null,
            "words": [
                { "word": "Hello", "start": 0, "end": 1 },
                { "word": "world", "start": 1, "end": 2 }
            ],
            "x_groq": { "id": "req_01jrh9nn61f24rydqq1r4b3yg5" }
        })))
        .await;
        let model = model_on(&server, None, "whisper-large-v3");

        let result = model
            .do_generate(&call(
                "audio/wav",
                Some(json!({
                    "language": "en",
                    "responseFormat": "verbose_json",
                    "timestampGranularities": ["word"]
                })),
            ))
            .await
            .unwrap();

        let segments: Vec<_> = result
            .segments
            .iter()
            .map(|s| (s.text.as_str(), s.start_second, s.end_second))
            .collect();
        assert_eq!(segments, [("Hello", 0.0, 1.0), ("world", 1.0, 2.0)]);
    }
}

/// Upstream's `GroqTranscriptionModel` has `doGenerate` only
/// (`groq-transcription-model.ts:30`, `:123`), and `doStream` is optional on
/// `TranscriptionModelV4` (`@ai-sdk/provider@4.0.22
/// src/transcription-model/v4/transcription-model-v4.ts:44-46`). Streaming is
/// therefore unsupported before any request is sent.
#[tokio::test]
async fn the_model_has_no_streaming_method() {
    use aimux_core::error::AiMuxError;
    use aimux_core::transcription_model::{
        InputAudioFormat, TranscriptionStreamOptions, stream_transcribe,
    };

    let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
    let model = model_on(&server, None, "whisper-large-v3");
    let options = || TranscriptionStreamOptions {
        audio: Box::pin(futures::stream::empty()),
        input_audio_format: InputAudioFormat {
            format_type: "audio/pcm".to_string(),
            rate: Some(16_000),
        },
        provider_options: None,
        abort_signal: None,
        headers: None,
        include_raw_chunks: false,
        timeout: None,
    };

    assert!(model.do_stream(options()).is_none());
    let error = stream_transcribe(&model, options()).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::UnsupportedFunctionality(message)
            if message == "The groq.transcription model \"whisper-large-v3\" does not support streaming transcription."),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// Only `transcribe` downloads an audio URL (layer 5's `AudioInput::Url`);
/// the model itself refuses one before any request, as every transcription
/// model does.
#[tokio::test]
async fn url_audio_is_refused_before_any_request() {
    let server = serve(ResponseTemplate::new(200).set_body_json(json_body())).await;
    let options = TranscriptionCallOptions::new(
        AudioInput::Url("https://cdn.test/a.wav".into()),
        "audio/wav",
    );
    let error = model_on(&server, None, "whisper-large-v3-turbo")
        .do_generate(&options)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), AudioInput::url_error().to_string());
    assert!(server.received_requests().await.unwrap().is_empty());
}
