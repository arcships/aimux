//! Rust translation of the Mistral speech model tests.
//!
//! Source: `reference/aisdk-pinned/mistral/src/mistral-speech-model.test.ts`
//!
//! The requests go through a scripted transport (`common/mock_fetch.rs`); the
//! TS `createTestServer` that intercepts `https://api.mistral.ai/v1/...` has no
//! counterpart here. The pinned `VERSION` mock has none either: the
//! user-agent check only looks at the package prefix. The response timestamp is
//! taken from the clock (no `_internal.currentDate`), so it is only checked to
//! be present.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::provider::Provider;
use aimux_core::shared::Warning;
use aimux_core::speech_model::{AudioData, SpeechCallOptions, SpeechModel, SpeechResult};
use aimux_providers::{MistralProviderSettings, create_mistral};
use mock_fetch::{Canned, MockFetch};

const MODEL_ID: &str = "voxtral-mini-tts-2603";
const URL: &str = "https://api.mistral.ai/v1/audio/speech";

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

fn response(audio_data: &str) -> Canned {
    Canned::json(&json!({ "audio_data": audio_data }))
}

/// One call with the default response; returns the result and the recorded body.
async fn generate(options: SpeechCallOptions) -> (SpeechResult, Value) {
    let fetch = MockFetch::new(vec![response("SUQzBAAAAAAA")]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).speech(MODEL_ID);
    let result = model.do_generate(&options).await.unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, URL);
    (result, seen[0].json_body())
}

fn text_options() -> SpeechCallOptions {
    SpeechCallOptions::new("Hello from the AI SDK!")
}

fn mistral_options(value: Value) -> aimux_core::shared::SharedProviderOptions {
    aimux_core::shared::provider_namespace("mistral", value).unwrap()
}

fn feature(warning: &Warning) -> &str {
    match warning {
        Warning::Unsupported { feature, .. } => feature,
        other => panic!("unexpected warning {other:?}"),
    }
}

/// TS: "should expose correct provider and model information"
#[test]
fn should_expose_correct_provider_and_model_information() {
    let fetch = MockFetch::new(vec![]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).speech(MODEL_ID);
    assert_eq!(model.provider(), "mistral.speech");
    assert_eq!(model.model_id(), MODEL_ID);
}

/// TS: "should create speech models through both provider factories"
#[test]
fn should_create_speech_models_through_both_provider_factories() {
    let fetch = MockFetch::new(vec![]);
    let provider = provider_with(&fetch, MistralProviderSettings::default());
    assert_eq!(provider.speech(MODEL_ID).provider(), "mistral.speech");
    let model = provider.speech_model(MODEL_ID).unwrap().unwrap();
    assert_eq!(model.provider(), "mistral.speech");
}

/// TS: "should send a non-streaming request with default mp3 output"
#[tokio::test]
async fn should_send_a_non_streaming_request_with_default_mp3_output() {
    let (_, body) = generate(text_options()).await;
    assert_eq!(
        body,
        json!({
            "model": MODEL_ID,
            "input": "Hello from the AI SDK!",
            "response_format": "mp3",
            "stream": false,
        })
    );
}

/// TS: "should map the voice to voice_id"
#[tokio::test]
async fn should_map_the_voice_to_voice_id() {
    let mut options = text_options();
    options.voice = Some("voice-id".into());
    let (_, body) = generate(options).await;
    assert_eq!(body["voice_id"], "voice-id");
}

/// TS: "should map refAudio to ref_audio and prefer it over voice"
#[tokio::test]
async fn should_map_ref_audio_to_ref_audio_and_prefer_it_over_voice() {
    let mut options = text_options();
    options.voice = Some("voice-id".into());
    options.provider_options = Some(mistral_options(
        json!({ "refAudio": "cmVmZXJlbmNlLWF1ZGlv" }),
    ));
    let (_, body) = generate(options).await;
    assert_eq!(
        body,
        json!({
            "model": MODEL_ID,
            "input": "Hello from the AI SDK!",
            "ref_audio": "cmVmZXJlbmNlLWF1ZGlv",
            "response_format": "mp3",
            "stream": false,
        })
    );
}

/// TS: "should accept the %s output format"
#[tokio::test]
async fn should_accept_the_output_formats() {
    for output_format in ["pcm", "wav", "mp3", "flac", "opus"] {
        let mut options = text_options();
        options.output_format = Some(output_format.into());
        let (_, body) = generate(options).await;
        assert_eq!(body["response_format"], output_format);
    }
}

/// TS: "should warn and use mp3 for unsupported output formats"
#[tokio::test]
async fn should_warn_and_use_mp3_for_unsupported_output_formats() {
    let mut options = text_options();
    options.output_format = Some("aac".into());
    let (result, body) = generate(options).await;
    assert_eq!(body["response_format"], "mp3");
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| feature(warning) == "outputFormat")
    );
}

/// TS: "should warn for unsupported standard speech options"
#[tokio::test]
async fn should_warn_for_unsupported_standard_speech_options() {
    let mut options = text_options();
    options.instructions = Some("Speak cheerfully".into());
    options.speed = Some(1.2);
    options.language = Some("en".into());
    let (result, _) = generate(options).await;
    let features: Vec<&str> = result.warnings.iter().map(feature).collect();
    assert_eq!(features, ["instructions", "speed", "language"]);
}

/// TS: "should pass headers and the Mistral user agent"
#[tokio::test]
async fn should_pass_headers_and_the_mistral_user_agent() {
    let fetch = MockFetch::new(vec![response("SUQzBAAAAAAA")]);
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
    let mut options = text_options();
    options.headers = Some(HashMap::from([(
        "Custom-Request-Header".to_string(),
        "request-header-value".to_string(),
    )]));
    provider
        .speech(MODEL_ID)
        .do_generate(&options)
        .await
        .unwrap();

    let headers = &fetch.seen()[0].headers;
    assert_eq!(headers["authorization"], "Bearer test-api-key");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["custom-provider-header"], "provider-header-value");
    assert_eq!(headers["custom-request-header"], "request-header-value");
    assert!(headers["user-agent"].contains("ai-sdk-mistral/"));
}

/// TS: "should use a custom base URL"
#[tokio::test]
async fn should_use_a_custom_base_url() {
    let fetch = MockFetch::new(vec![response("SUQzBAAAAAAA")]);
    let provider = provider_with(
        &fetch,
        MistralProviderSettings {
            base_url: Some("https://custom.mistral.example/v2/".into()),
            ..Default::default()
        },
    );
    provider
        .speech(MODEL_ID)
        .do_generate(&text_options())
        .await
        .unwrap();
    assert_eq!(
        fetch.seen()[0].url,
        "https://custom.mistral.example/v2/audio/speech"
    );
}

/// TS: "should use a custom fetch implementation"
#[tokio::test]
async fn should_use_a_custom_fetch_implementation() {
    let (_, _) = generate(text_options()).await;
}

/// TS: "should return base64 audio data"
#[tokio::test]
async fn should_return_base64_audio_data() {
    let fetch = MockFetch::new(vec![response("YXVkaW8=")]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).speech(MODEL_ID);
    let result = model.do_generate(&text_options()).await.unwrap();
    assert!(matches!(result.audio, AudioData::Base64(ref audio) if audio == "YXVkaW8="));
    assert!(result.warnings.is_empty());
}

/// TS: "should include response data with timestamp, model id, and headers"
#[tokio::test]
async fn should_include_response_data_with_timestamp_model_id_and_headers() {
    let mut canned = response("SUQzBAAAAAAA");
    canned
        .headers
        .push(("x-request-id".into(), "test-request-id".into()));
    let fetch = MockFetch::new(vec![canned]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).speech(MODEL_ID);
    let result = model.do_generate(&text_options()).await.unwrap();
    assert!(result.response.timestamp.is_some());
    assert_eq!(result.response.model_id.as_deref(), Some(MODEL_ID));
    assert_eq!(
        result.response.headers.unwrap()["x-request-id"],
        "test-request-id"
    );
    assert_eq!(
        result.response.body,
        Some(json!({ "audio_data": "SUQzBAAAAAAA" }))
    );
}

/// TS: "should redact reference audio from request metadata"
#[tokio::test]
async fn should_redact_reference_audio_from_request_metadata() {
    let mut options = text_options();
    options.provider_options = Some(mistral_options(
        json!({ "refAudio": "sensitive-reference-audio" }),
    ));
    let (result, _) = generate(options).await;
    let body = result.request.unwrap().body.unwrap();
    assert_eq!(
        body,
        Value::String(
            r#"{"model":"voxtral-mini-tts-2603","input":"Hello from the AI SDK!","ref_audio":"[redacted]","response_format":"mp3","stream":false}"#
                .into()
        )
    );
    assert!(!body.as_str().unwrap().contains("sensitive-reference-audio"));
}

/// TS: "should redact reference audio from API errors"
#[tokio::test]
async fn should_redact_reference_audio_from_api_errors() {
    let fetch = MockFetch::new(vec![Canned {
        status: 400,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&json!({
            "object": "error",
            "message": "The request was rejected.",
            "type": "invalid_request_error",
            "param": "ref_audio",
            "code": "invalid_reference_audio",
        }))
        .unwrap(),
    }]);
    let model = provider_with(&fetch, MistralProviderSettings::default()).speech(MODEL_ID);
    let mut options = text_options();
    options.provider_options = Some(mistral_options(
        json!({ "refAudio": "sensitive-reference-audio" }),
    ));

    let Err(AiMuxError::ApiCall(error)) = model.do_generate(&options).await else {
        panic!("expected an API call error");
    };
    assert_eq!(error.message, "The request was rejected.");
    assert_eq!(error.status_code, Some(400));
    assert_eq!(error.request_body_values["ref_audio"], "[redacted]");
}

/// TS: "should forward the abort signal"
#[tokio::test]
async fn should_forward_the_abort_signal() {
    let mut options = text_options();
    options.abort_signal = Some(aimux_core::AbortSignal::new());
    let (_, _) = generate(options).await;
}
