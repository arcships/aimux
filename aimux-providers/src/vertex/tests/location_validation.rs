//! Port of `@ai-sdk/google-vertex` 5.0.100 `src/google-vertex-location-validation.test.ts`:
//! the cases the old `tests/vertex_location_validation_test.rs` covers only
//! in part (its other cases carry their markers there).
//!
//! Upstream throws when the model is created; aimux resolves the location per
//! request, so the request fails before anything is fetched. The upstream
//! `describe` is `${name} location validation` for `google`, `anthropic` and
//! `maas`; one test covers all three. Upstream's `argument: 'location'` has no
//! counterpart: aimux's `InvalidArgument` carries a message, which names the
//! location.

use std::sync::Arc;

use serial_test::serial;

use crate::test_support::mock_fetch::{EnvVar, MockFetch};
use crate::vertex::{
    VertexAnthropicProviderSettings, VertexPartnerProviderSettings, VertexProviderSettings,
    create_google_vertex, create_google_vertex_anthropic, create_google_vertex_maas,
};
use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::transcription_model::{
    AudioInput, InputAudioFormat, TranscriptionCallOptions, TranscriptionStreamOptions,
};
use aimux_provider_utils::Resolvable;

/// Whether `error` is the invalid-location error.
fn is_location_error(error: &AiMuxError) -> bool {
    matches!(
        error,
        AiMuxError::InvalidArgument(message) if message.starts_with("Invalid Google Vertex location.")
    )
}

mod name_location_validation {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum Name {
        Google,
        Anthropic,
        Maas,
    }

    /// `create({ baseURL })('test-model')` for each `name`, with a fixed
    /// token so that only the settings decide whether a request is sent.
    fn create(name: Name, base_url: &str, fetch: &Arc<MockFetch>) -> Arc<dyn LanguageModel> {
        let base_url = Some(base_url.to_string());
        let token = Some(Resolvable::Value(Some("test-token".to_string())));
        match name {
            Name::Google => create_google_vertex(VertexProviderSettings {
                base_url,
                access_token: Some("test-token".to_string().into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap()
            .call("test-model"),
            Name::Anthropic => create_google_vertex_anthropic(VertexAnthropicProviderSettings {
                base_url,
                fetch: Some(fetch.transport()),
                generate_auth_token: token,
                ..Default::default()
            })
            .unwrap()
            .call("test-model"),
            Name::Maas => Arc::new(
                create_google_vertex_maas(VertexPartnerProviderSettings {
                    base_url,
                    fetch: Some(fetch.transport()),
                    generate_auth_token: token,
                    ..Default::default()
                })
                .unwrap()
                .chat("test-model"),
            ),
        }
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-location-validation.test.ts › ${…} › allows a custom URL without a location
    #[tokio::test]
    #[serial]
    async fn allows_a_custom_url_without_a_location() {
        let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);
        for name in [Name::Google, Name::Anthropic, Name::Maas] {
            let fetch = MockFetch::new(vec![]);
            let model = create(name, "https://proxy.example/v1", &fetch);
            // The mock answers nothing, so the request fails after it is sent.
            let error = model
                .do_generate(&CallOptions::new(vec![LanguageModelMessage::user_text(
                    "Hello",
                )]))
                .await
                .expect_err("the mock answers nothing");
            assert!(!is_location_error(&error), "{name:?}: {error:?}");
            assert!(
                fetch
                    .seen()
                    .first()
                    .is_some_and(|seen| seen.url.starts_with("https://proxy.example/v1/")),
                "{name:?} sent no request: {error:?}"
            );
        }
    }
}

/// upstream: @ai-sdk/google-vertex src/google-vertex-location-validation.test.ts › validates location for the separate transcription host of %s
#[tokio::test]
async fn validates_location_for_the_separate_transcription_host_of_s() {
    for model_id in ["chirp_3", "gemini-3.5-transcribe-live"] {
        let fetch = MockFetch::new(vec![]);
        let model = create_google_vertex(VertexProviderSettings {
            project: Some("test".into()),
            location: Some("user@internal/#".into()),
            base_url: Some("https://proxy.example/v1".into()),
            access_token: Some("test-token".to_string().into()),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap()
        .transcription(model_id);

        // The live model only streams (over a WebSocket); the others are
        // unary.
        let error = if model_id.contains("-live") {
            let options = TranscriptionStreamOptions {
                audio: Box::pin(futures::stream::empty()),
                input_audio_format: InputAudioFormat {
                    format_type: "audio/pcm".into(),
                    rate: Some(16_000),
                },
                provider_options: None,
                abort_signal: None,
                headers: None,
                include_raw_chunks: false,
                timeout: None,
            };
            match model.do_stream(options).expect("a streaming model").await {
                Ok(_) => panic!("{model_id}: the stream started"),
                Err(error) => error,
            }
        } else {
            let options =
                TranscriptionCallOptions::new(AudioInput::Binary(vec![0, 1, 2]), "audio/wav");
            model.do_generate(&options).await.unwrap_err()
        };

        assert!(is_location_error(&error), "{model_id}: {error:?}");
        assert!(fetch.seen().is_empty(), "{model_id}");
    }
}
