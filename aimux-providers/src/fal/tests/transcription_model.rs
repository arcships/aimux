//! Port of `@ai-sdk/fal@3.0.54` `src/fal-transcription-model.test.ts`
//! (fixtures `__fixtures__/fal-transcription-queue.json` and
//! `__fixtures__/fal-transcription-result.json`, copied to `fixtures/`).
//!
//! Upstream answers the hard-coded `https://queue.fal.run`; here the provider's
//! `base_url` points at a mock server. The upstream `_internal.currentDate`
//! hook has no Rust counterpart, so the two response cases check that the
//! timestamp is the clock's time during the call instead of `new Date(0)`.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::Timestamp;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};

use crate::fal::{FalProvider, FalProviderSettings, create_fal};

/// Stand-in for `transcript-test.mp3`.
const AUDIO: [u8; 4] = [0x49, 0x44, 0x33, 0x04];

/// The TS `{ audio: audioData, mediaType: 'audio/wav' }`.
fn call() -> TranscriptionCallOptions {
    TranscriptionCallOptions::new(AudioInput::Binary(AUDIO.to_vec()), "audio/wav")
}

/// `prepareJsonFixtureResponse(headers)`: the submit URL answers the queue
/// fixture and the status URL the result fixture, both with `headers`.
async fn serve(headers: &[(&str, &str)]) -> MockServer {
    let respond = |fixture: &str| {
        let body: Value = serde_json::from_str(fixture).unwrap();
        headers.iter().fold(
            ResponseTemplate::new(200).set_body_json(body),
            |template, (name, value)| template.insert_header(*name, *value),
        )
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal-ai/wizper"))
        .respond_with(respond(include_str!(
            "fixtures/fal-transcription-queue.json"
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fal-ai/wizper/requests/test-id"))
        .respond_with(respond(include_str!(
            "fixtures/fal-transcription-result.json"
        )))
        .mount(&server)
        .await;
    server
}

/// The TS `createFal({ apiKey: 'test-api-key', headers })`, on the mock server.
fn provider(server: &MockServer, headers: Option<HashMap<String, Option<String>>>) -> FalProvider {
    create_fal(FalProviderSettings {
        api_key: Some("test-api-key".to_string().into()),
        base_url: Some(server.uri()),
        headers,
        ..Default::default()
    })
    .unwrap()
}

mod do_generate {
    use super::*;

    mod transcription {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-transcription-model.test.ts › doGenerate › transcription › should pass the model
        #[tokio::test]
        async fn should_pass_the_model() {
            let server = serve(&[]).await;

            provider(&server, None)
                .transcription("wizper")
                .do_generate(&call())
                .await
                .unwrap();

            let requests = server.received_requests().await.unwrap();
            let body: Value = requests[0].body_json().unwrap();
            assert!(
                body["audio_url"]
                    .as_str()
                    .unwrap()
                    .starts_with("data:audio/")
            );
            assert_eq!(body["task"], json!("transcribe"));
            assert_eq!(body["diarize"], json!(true));
            assert_eq!(body["chunk_level"], json!("word"));
        }

        /// upstream: @ai-sdk/fal src/fal-transcription-model.test.ts › doGenerate › transcription › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let server = serve(&[]).await;
            let mut options = call();
            options.headers = Some(HashMap::from([(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]));

            provider(
                &server,
                Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
            )
            .transcription("wizper")
            .do_generate(&options)
            .await
            .unwrap();

            let requests = server.received_requests().await.unwrap();
            let headers = &requests[0].headers;
            assert_eq!(headers.get("authorization").unwrap(), "Key test-api-key");
            assert_eq!(headers.get("content-type").unwrap(), "application/json");
            assert_eq!(
                headers.get("custom-provider-header").unwrap(),
                "provider-header-value"
            );
            assert_eq!(
                headers.get("custom-request-header").unwrap(),
                "request-header-value"
            );
        }

        /// upstream: @ai-sdk/fal src/fal-transcription-model.test.ts › doGenerate › transcription › should extract the transcription text
        #[tokio::test]
        async fn should_extract_the_transcription_text() {
            let server = serve(&[]).await;

            let result = provider(&server, None)
                .transcription("wizper")
                .do_generate(&call())
                .await
                .unwrap();

            assert_eq!(result.text, "Hello from the Versal AISDK.");
        }
    }

    mod response_headers {
        use super::*;

        /// partial: `doGenerate › response headers › should include response data with timestamp, modelId and headers`; upstream injects `currentDate: () => new Date(0)`, which the Rust model has no hook for, so the test checks the timestamp is the clock's time during the call. The mock server adds headers of its own (`date`), so the snapshot's four headers are checked one by one.
        #[tokio::test]
        async fn should_include_response_data_with_timestamp_model_id_and_headers() {
            let server = serve(&[
                ("x-request-id", "test-request-id"),
                ("x-ratelimit-remaining", "123"),
            ])
            .await;

            let before = Timestamp::now();
            let result = provider(&server, None)
                .transcription("wizper")
                .do_generate(&call())
                .await
                .unwrap();
            let after = Timestamp::now();

            let response = result.response;
            let timestamp = response.timestamp.unwrap();
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(response.model_id.as_deref(), Some("wizper"));
            assert_eq!(
                response.body,
                Some(json!({
                    "chunks": [{
                        "text": "Hello from the Versal AISDK.",
                        "timestamp": [0, 2.508],
                    }],
                    "languages": ["en"],
                    "text": "Hello from the Versal AISDK.",
                }))
            );
            let headers = response.headers.unwrap();
            assert_eq!(headers["content-length"], "131");
            assert_eq!(headers["content-type"], "application/json");
            assert_eq!(headers["x-ratelimit-remaining"], "123");
            assert_eq!(headers["x-request-id"], "test-request-id");
        }
    }

    mod response_metadata {
        use super::*;

        /// partial: `doGenerate › response metadata › should use real date when no custom date provider is specified`; upstream injects `currentDate: () => new Date(0)`, which the Rust model has no hook for, so the test checks the timestamp is the clock's time during the call.
        #[tokio::test]
        async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
            let server = serve(&[]).await;

            let before = Timestamp::now();
            let result = provider(&server, None)
                .transcription("wizper")
                .do_generate(&call())
                .await
                .unwrap();
            let after = Timestamp::now();

            let timestamp = result.response.timestamp.unwrap();
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(result.response.model_id.as_deref(), Some("wizper"));
        }
    }
}
