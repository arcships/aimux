//! Rust translation of the Fal transcription model tests.
//! Source: `reference/ai/packages/fal/src/fal-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::Resolvable;
use aimux_providers::{FalProviderSettings, create_fal};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3]
}
fn options(audio: Vec<u8>, mt: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), mt)
}

async fn mock_queue_and_result(server: &MockServer, result_body: &Value, headers: &[(&str, &str)]) {
    // Submit endpoint
    let mut submit_t = ResponseTemplate::new(200).set_body_json(json!({"request_id": "test-id"}));
    for (k, v) in headers {
        submit_t = submit_t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/fal-ai/wizper"))
        .respond_with(submit_t)
        .mount(server)
        .await;

    // Poll endpoint — return result on first try
    let mut result_t = ResponseTemplate::new(200).set_body_json(result_body.clone());
    for (k, v) in headers {
        result_t = result_t.insert_header(*k, *v);
    }
    Mock::given(method("GET"))
        .and(path("/fal-ai/wizper/requests/test-id"))
        .respond_with(result_t)
        .mount(server)
        .await;
}

fn fixture_result() -> Value {
    json!({
        "text": "Hello from the Versal AISDK.",
        "chunks": [
            {"text": "Hello", "timestamp": [0.0, 1.0]},
            {"text": " from the Versal AISDK.", "timestamp": [1.0, 3.5]}
        ],
        "inferred_languages": ["en"]
    })
}

/// partial: `should pass the model`; full port in src/fal/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_model_and_audio() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.transcription("wizper");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        body["audio_url"]
            .as_str()
            .unwrap()
            .starts_with("data:audio/")
    );
    assert_eq!(body["task"], "transcribe");
    assert_eq!(body["diarize"], true);
    assert_eq!(body["chunk_level"], "word");
}

/// partial: `should pass headers`; full port in src/fal/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        headers: Some((ph).into_iter().map(|(k, v)| (k, Some(v))).collect()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.transcription("wizper");

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
    assert_eq!(h.get("authorization").unwrap(), "Key test-api-key");
    assert_eq!(h.get("content-type").unwrap(), "application/json");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// partial: `should extract the transcription text`; full port in src/fal/tests/transcription_model.rs.
#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "Hello from the Versal AISDK.");
}

/// partial: `should include response data with timestamp, modelId and headers`; full port in src/fal/tests/transcription_model.rs.
#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_queue_and_result(
        &server,
        &fixture_result(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("wizper".to_string()));
}

/// partial: `should use real date when no custom date provider is specified`; full port in src/fal/tests/transcription_model.rs.
#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;

    let config = FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_fal(config).unwrap();
    let model = provider.transcription("wizper");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("wizper".to_string()));
}

fn wizper(base_url: &str) -> aimux_providers::FalTranscriptionModel {
    create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(base_url.to_string()),
        ..Default::default()
    })
    .unwrap()
    .transcription("wizper")
}

fn with_fal(fal: Value) -> TranscriptionCallOptions {
    let mut call = options(mock_audio(), "audio/wav");
    call.provider_options = Some(serde_json::from_value(json!({ "fal": fal })).unwrap());
    call
}

/// `providerOptions.fal` is parsed against `falTranscriptionModelOptionsSchema`
/// (fal-transcription-model.ts getArgs). Its `.default()`s fill a missing key
/// and keep a `null`; the body then takes `language` as is, drops a nullish
/// `version`/`batchSize`/`numSpeakers`, and keeps the base `diarize: true` /
/// `chunk_level: "word"` unless the parsed value is set.
#[tokio::test]
async fn should_parse_the_options_with_the_upstream_defaults() {
    let server = MockServer::start().await;
    mock_queue_and_result(&server, &fixture_result(), &[]).await;
    let model = wizper(&server.uri());

    for fal in [
        json!({}),
        json!({
            "language": null, "diarize": null, "chunkLevel": null,
            "version": null, "batchSize": null, "numSpeakers": null,
        }),
        json!({
            "language": "de", "diarize": false, "chunkLevel": "word",
            "version": "3", "batchSize": 8, "numSpeakers": 2, "unknown": 1,
        }),
    ] {
        model.do_generate(&with_fal(fal)).await.unwrap();
    }

    let requests = server.received_requests().await.unwrap();
    let bodies: Vec<Value> = requests
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| {
            let mut body: Value = serde_json::from_slice(&request.body).unwrap();
            body.as_object_mut().unwrap().remove("audio_url");
            body
        })
        .collect();
    assert_eq!(
        bodies,
        [
            json!({
                "task": "transcribe", "diarize": true, "chunk_level": "segment",
                "language": "en", "version": "3", "batch_size": 64,
            }),
            json!({
                "task": "transcribe", "diarize": true, "chunk_level": "word",
                "language": null,
            }),
            json!({
                "task": "transcribe", "diarize": false, "chunk_level": "word",
                "language": "de", "version": "3", "batch_size": 8, "num_speakers": 2,
            }),
        ]
    );
}

/// A value outside the schema fails with the upstream message before
/// anything is sent.
#[tokio::test]
async fn should_reject_options_outside_the_schema() {
    let model = wizper("http://127.0.0.1:9");
    for fal in [
        json!({ "language": 1 }),
        json!({ "diarize": "yes" }),
        json!({ "chunkLevel": "sentence" }),
        json!({ "version": "2" }),
        json!({ "batchSize": "64" }),
        json!({ "numSpeakers": "2" }),
    ] {
        let error = model.do_generate(&with_fal(fal.clone())).await.unwrap_err();
        let aimux_core::error::AiMuxError::InvalidArgument { message, cause } = &error else {
            panic!("expected InvalidArgument for {fal}, got {error:?}");
        };
        assert_eq!(
            message, "Invalid argument for parameter providerOptions: invalid fal provider options",
            "{fal}"
        );
        assert!(cause.is_some(), "{fal}");
    }
}

/// A poll answered with `detail: "Request is still in progress"` is pending,
/// whatever its status; any other error response ends the call
/// (fal-transcription-model.ts doGenerate, the status `failedResponseHandler`).
#[tokio::test]
async fn should_poll_only_while_the_request_is_in_progress() {
    let in_progress = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fal-ai/wizper/requests/test-id"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({ "detail": "Request is still in progress" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&in_progress)
        .await;
    mock_queue_and_result(&in_progress, &fixture_result(), &[]).await;
    let result = wizper(&in_progress.uri())
        .do_generate(&with_fal(json!({ "pollIntervalMs": 0 })))
        .await
        .unwrap();
    assert_eq!(result.text, "Hello from the Versal AISDK.");

    let failed = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/fal-ai/wizper/requests/test-id"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({ "detail": "Invalid audio file" })),
        )
        .with_priority(1)
        .mount(&failed)
        .await;
    mock_queue_and_result(&failed, &fixture_result(), &[]).await;
    let error = wizper(&failed.uri())
        .do_generate(&with_fal(json!({ "pollIntervalMs": 0 })))
        .await
        .unwrap_err();
    let aimux_core::error::AiMuxError::ApiCall(detail) = &error else {
        panic!("expected ApiCall, got {error:?}");
    };
    assert_eq!(detail.status_code, Some(400));
    let polls = failed
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "GET")
        .count();
    assert_eq!(polls, 1);
}
