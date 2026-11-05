//! Rust translation of the Deepgram transcription model tests.
//!
//! Source: `reference/ai/packages/deepgram/src/deepgram-transcription-model.test.ts`

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::Resolvable;
use aimux_providers::{DeepgramProviderSettings, create_deepgram};

fn mock_audio() -> Vec<u8> {
    vec![1u8, 2, 3, 4, 5]
}

fn options(audio: Vec<u8>, media_type: &str) -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(audio), media_type)
}

fn fixture_response() -> Value {
    json!({
        "metadata": {"duration": 36.744},
        "results": {
            "channels": [{
                "detected_language": "en",
                "alternatives": [{
                    "transcript": "galileo was an american robotic space program",
                    "words": [
                        {"word": "galileo", "start": 0.16, "end": 0.8},
                        {"word": "was", "start": 0.8, "end": 0.96}
                    ]
                }]
            }]
        }
    })
}

async fn mock_response(server: &MockServer, body: &Value) {
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body.clone()),
        )
        .mount(server)
        .await;
}

async fn mock_response_with_headers(server: &MockServer, body: &Value, headers: &[(&str, &str)]) {
    let mut t = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body.clone());
    for (k, v) in headers {
        t = t.insert_header(*k, *v);
    }
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .respond_with(t)
        .mount(server)
        .await;
}

/// partial: `should pass the model`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_model_in_query() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    // Model is passed as a query parameter.
    assert!(requests[0].url.as_str().contains("model=nova-3"));
}

/// `createDeepgram` adds `ai-sdk-deepgram/<version>` to the user-agent
/// (deepgram-provider.ts getHeaders, withUserAgentSuffix), and the
/// provider-utils part names the `@ai-sdk/provider-utils` that
/// `@ai-sdk/deepgram@3.1.24` depends on.
#[tokio::test]
async fn should_include_the_package_user_agent() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;
    let provider = create_deepgram(DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap();
    provider
        .transcription("nova-3")
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let user_agent = requests[0]
        .headers
        .get("user-agent")
        .expect("a user-agent")
        .to_str()
        .unwrap();
    assert!(
        user_agent.contains("ai-sdk-deepgram/3.1.24"),
        "{user_agent}"
    );
    assert!(
        user_agent.contains("ai-sdk-provider-utils/5.0.53"),
        "{user_agent}"
    );
}

/// partial: `should pass headers`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_headers() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let mut ph = HashMap::new();
    ph.insert(
        "Custom-Provider-Header".to_string(),
        "provider-header-value".to_string(),
    );
    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        headers: Some((ph).into_iter().map(|(k, v)| (k, Some(v))).collect()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

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
    assert_eq!(h.get("authorization").unwrap(), "Token test-api-key");
    assert_eq!(h.get("content-type").unwrap(), "audio/wav");
    assert_eq!(
        h.get("custom-provider-header").unwrap(),
        "provider-header-value"
    );
    assert_eq!(
        h.get("custom-request-header").unwrap(),
        "request-header-value"
    );
}

/// partial: `should extract the transcription text`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_extract_text() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.text, "galileo was an american robotic space program");
}

/// partial: `should pass detectLanguage as detect_language query parameter`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_pass_detect_language_query_param() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let po = aimux_core::shared::provider_namespace("deepgram", json!({"detectLanguage": true}))
        .unwrap();
    opts.provider_options = Some(po);

    model.do_generate(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].url.as_str().contains("detect_language=true"));
}

/// partial: `should return detected language from response`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_return_detected_language() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let po = aimux_core::shared::provider_namespace("deepgram", json!({"detectLanguage": true}))
        .unwrap();
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.language, Some("en".to_string()));
}

/// partial: `should include response data with timestamp, modelId and headers`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_include_response_data() {
    let server = MockServer::start().await;
    mock_response_with_headers(
        &server,
        &fixture_response(),
        &[
            ("x-request-id", "test-request-id"),
            ("x-ratelimit-remaining", "123"),
        ],
    )
    .await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("nova-3".to_string()));
    let headers = result.response.headers.as_ref().unwrap();
    assert_eq!(headers.get("x-request-id").unwrap(), "test-request-id");
    assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "123");
}

/// partial: `should use real date when no custom date provider is specified`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_use_real_date() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id, Some("nova-3".to_string()));
}

/// partial: `should return detected language from inline response`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_return_language_from_inline_response() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &json!({
            "metadata": {"duration": 1.0},
            "results": {
                "channels": [{
                    "detected_language": "sv",
                    "alternatives": [{"transcript": "hej", "words": []}]
                }]
            }
        }),
    )
    .await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let mut opts = options(mock_audio(), "audio/wav");
    let po = aimux_core::shared::provider_namespace("deepgram", json!({"detectLanguage": true}))
        .unwrap();
    opts.provider_options = Some(po);

    let result = model.do_generate(&opts).await.unwrap();

    assert_eq!(result.language, Some("sv".to_string()));
}

/// partial: `should return undefined language when not detected`; full port in src/deepgram/tests/transcription_model.rs.
#[tokio::test]
async fn should_return_none_language_when_not_detected() {
    let server = MockServer::start().await;
    mock_response(
        &server,
        &json!({
            "metadata": {"duration": 1.0},
            "results": {
                "channels": [{
                    "alternatives": [{"transcript": "hello", "words": []}]
                }]
            }
        }),
    )
    .await;

    let config = DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    };
    let provider = create_deepgram(config).unwrap();
    let model = provider.transcription("nova-3");

    let result = model
        .do_generate(&options(mock_audio(), "audio/wav"))
        .await
        .unwrap();

    assert_eq!(result.language, None);
}

/// `diarize` is sent only when the caller sets it
/// (deepgram-transcription-model.ts getArgs).
#[tokio::test]
async fn should_send_diarize_only_when_set() {
    for (deepgram, expected) in [
        (None, None),
        (Some(json!({ "diarize": true })), Some("true")),
        (Some(json!({ "diarize": false })), Some("false")),
    ] {
        let server = MockServer::start().await;
        mock_response(&server, &fixture_response()).await;
        let model = create_deepgram(DeepgramProviderSettings {
            api_key: Some(Resolvable::Value("test-api-key".to_string())),
            base_url: Some(server.uri().to_string()),
            ..Default::default()
        })
        .unwrap()
        .transcription("nova-3");
        let mut call = options(mock_audio(), "audio/wav");
        call.provider_options = deepgram
            .map(|deepgram| serde_json::from_value(json!({ "deepgram": deepgram })).unwrap());
        model.do_generate(&call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let diarize = requests[0]
            .url
            .query_pairs()
            .find(|(key, _)| key == "diarize")
            .map(|(_, value)| value.into_owned());
        assert_eq!(diarize.as_deref(), expected);
    }
}

/// The query parameters a transcription sends with `deepgram` options.
async fn query_with(deepgram: Value) -> Vec<(String, String)> {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;
    let model = create_deepgram(DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .transcription("nova-3");
    let mut call = options(mock_audio(), "audio/wav");
    call.provider_options = Some(serde_json::from_value(json!({ "deepgram": deepgram })).unwrap());
    model.do_generate(&call).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    requests[0]
        .url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// `providerOptions.deepgram` is parsed against
/// `deepgramTranscriptionModelOptionsSchema` and every key of it is sent, as
/// `String(value)`, in the order upstream builds its body
/// (deepgram-transcription-model.ts getArgs).
#[tokio::test]
async fn should_send_every_schema_option_as_upstream_does() {
    let query = query_with(json!({
        "language": "en",
        "smartFormat": true,
        "punctuate": true,
        "paragraphs": true,
        "summarize": "v2",
        "topics": true,
        "intents": true,
        "sentiment": true,
        "detectEntities": true,
        "redact": ["pci", "numbers"],
        "replace": "[redacted]",
        "search": "jupiter",
        "keyterm": "galileo",
        "diarize": true,
        "utterances": true,
        "uttSplit": 0.8,
        "fillerWords": true,
        "detectLanguage": true,
    }))
    .await;
    assert_eq!(
        query,
        [
            ("model", "nova-3"),
            ("detect_entities", "true"),
            ("detect_language", "true"),
            ("diarize", "true"),
            ("filler_words", "true"),
            ("intents", "true"),
            ("keyterm", "galileo"),
            ("language", "en"),
            ("paragraphs", "true"),
            ("punctuate", "true"),
            ("redact", "pci,numbers"),
            ("replace", "[redacted]"),
            ("search", "jupiter"),
            ("sentiment", "true"),
            ("smart_format", "true"),
            ("summarize", "v2"),
            ("topics", "true"),
            ("utterances", "true"),
            ("utt_split", "0.8"),
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .to_vec()
    );

    let query =
        query_with(json!({ "redact": "numbers", "summarize": false, "language": null })).await;
    assert_eq!(
        query,
        [
            ("model", "nova-3"),
            ("redact", "numbers"),
            ("summarize", "false")
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .to_vec()
    );
}

/// A value outside the schema fails with the upstream message before
/// anything is sent.
#[tokio::test]
async fn should_reject_options_outside_the_schema() {
    let model = create_deepgram(DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some("http://127.0.0.1:9".to_string()),
        ..Default::default()
    })
    .unwrap()
    .transcription("nova-3");
    for deepgram in [
        json!({ "summarize": true }),
        json!({ "summarize": "v1" }),
        json!({ "topics": ["space"] }),
        json!({ "redact": 1 }),
        json!({ "search": ["jupiter"] }),
        json!({ "uttSplit": "0.8" }),
    ] {
        let mut call = options(mock_audio(), "audio/wav");
        call.provider_options =
            Some(serde_json::from_value(json!({ "deepgram": deepgram })).unwrap());
        let error = model.do_generate(&call).await.unwrap_err();
        let aimux_core::error::AiMuxError::InvalidArgument { message, cause } = &error else {
            panic!("expected InvalidArgument for {deepgram}, got {error:?}");
        };
        assert_eq!(
            message,
            "Invalid argument for parameter providerOptions: invalid deepgram provider options",
            "{deepgram}"
        );
        assert!(cause.is_some(), "{deepgram}");
    }
}

/// The query is form-encoded like `URLSearchParams.toString()`
/// (deepgram-transcription-model.ts): a space is `+`, `&` and `=` inside a
/// value are escaped, so a value cannot add or split parameters.
#[tokio::test]
async fn should_form_encode_the_query() {
    let server = MockServer::start().await;
    mock_response(&server, &fixture_response()).await;
    let model = create_deepgram(DeepgramProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri().to_string()),
        ..Default::default()
    })
    .unwrap()
    .transcription("nova-3");
    let mut call = options(mock_audio(), "audio/wav");
    call.provider_options = Some(
        serde_json::from_value(json!({ "deepgram": {
            "keyterm": "space probe",
            "replace": "a&model=other",
        } }))
        .unwrap(),
    );
    model.do_generate(&call).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("model=nova-3&keyterm=space+probe&replace=a%26model%3Dother")
    );
}
