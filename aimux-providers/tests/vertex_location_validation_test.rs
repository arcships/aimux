//! Port of @ai-sdk/google-vertex `src/google-vertex-location-validation.test.ts`
//! (tag `@ai-sdk/google-vertex@5.0.100`, the locked version): a location that goes into a host name is one DNS
//! label. Upstream throws when the model is created; aimux resolves the
//! location per request, so the request fails before anything is fetched.
//! Tests that set `GOOGLE_VERTEX_LOCATION` are serial.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions};
use aimux_provider_utils::Resolvable;
use aimux_providers::{
    VertexAnthropicProviderSettings, VertexPartnerProviderSettings, VertexProviderSettings,
    create_google_vertex, create_google_vertex_anthropic, create_google_vertex_maas,
};
use mock_fetch::{EnvVar, MockFetch};
use serial_test::serial;

const INVALID: [&str; 6] = [
    "evil.example.com/#",
    "user@internal:8080/#",
    "us-east-1/../..",
    "us east 1",
    "",
    "region\n",
];
const VALID: [&str; 5] = ["global", "eu", "us", "us-central1", "us-east5"];

#[derive(Clone, Copy, Debug)]
enum Kind {
    Google,
    Anthropic,
    Maas,
}

const KINDS: [Kind; 3] = [Kind::Google, Kind::Anthropic, Kind::Maas];

/// A model of `kind` whose requests go to `fetch`, authenticated with a fixed
/// token so that only the location decides whether a request is sent.
fn model(
    kind: Kind,
    location: Option<&str>,
    base_url: Option<&str>,
    fetch: &Arc<MockFetch>,
) -> Arc<dyn LanguageModel> {
    let project = Some("test".to_string());
    let location = location.map(String::from);
    let base_url = base_url.map(String::from);
    let token = Some(Resolvable::Value(Some("test-token".to_string())));
    match kind {
        Kind::Google => create_google_vertex(VertexProviderSettings {
            project,
            location,
            base_url,
            access_token: Some("test-token".to_string().into()),
            fetch: Some(fetch.transport()),
            ..Default::default()
        })
        .unwrap()
        .call("test-model"),
        Kind::Anthropic => create_google_vertex_anthropic(VertexAnthropicProviderSettings {
            project,
            location,
            base_url,
            fetch: Some(fetch.transport()),
            generate_auth_token: token,
            ..Default::default()
        })
        .unwrap()
        .call("test-model"),
        Kind::Maas => Arc::new(
            create_google_vertex_maas(VertexPartnerProviderSettings {
                project,
                location,
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

fn prompt() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

/// Whether a request with this location is refused as an invalid location.
/// The mock has no canned response, so an accepted request fails later, with
/// some other error.
async fn rejects(kind: Kind, location: Option<&str>, base_url: Option<&str>) -> bool {
    let fetch = MockFetch::new(vec![]);
    let error = model(kind, location, base_url, &fetch)
        .do_generate(&prompt())
        .await
        .expect_err("the mock answers nothing");
    let rejected = matches!(
        &error,
        AiMuxError::InvalidArgument { message, .. } if message.starts_with("Invalid Google Vertex location.")
    );
    if rejected {
        assert!(
            fetch.seen().is_empty(),
            "{kind:?} fetched with {location:?}"
        );
    }
    rejected
}

/// TS: "rejects %j before fetching"
#[tokio::test]
async fn rejects_a_location_that_is_not_one_dns_label() {
    for kind in KINDS {
        for location in INVALID {
            assert!(
                rejects(kind, Some(location), None).await,
                "{kind:?} {location:?}"
            );
        }
    }
}

/// TS: "accepts %j"
#[tokio::test]
async fn accepts_regional_and_multi_region_locations() {
    for kind in KINDS {
        for location in VALID {
            assert!(
                !rejects(kind, Some(location), None).await,
                "{kind:?} {location:?}"
            );
        }
    }
}

/// TS: "uses the environment location"
#[tokio::test]
#[serial]
async fn validates_the_environment_location() {
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", Some("user@internal/#"));
    for kind in KINDS {
        assert!(rejects(kind, None, None).await, "{kind:?}");
    }
}

/// TS: "allows a custom URL with an unused invalid location"
#[tokio::test]
async fn a_custom_url_ignores_an_invalid_location() {
    for kind in KINDS {
        assert!(
            !rejects(kind, Some("not a region"), Some("https://proxy.example/v1")).await,
            "{kind:?}"
        );
    }
}

/// TS: "allows a custom URL without a location"
#[tokio::test]
#[serial]
async fn a_custom_url_needs_no_location() {
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", None);
    for kind in KINDS {
        assert!(
            !rejects(kind, None, Some("https://proxy.example/v1")).await,
            "{kind:?}"
        );
    }
}

/// TS: "does not validate location in express mode"
#[tokio::test]
async fn express_mode_ignores_the_location() {
    let fetch = MockFetch::new(vec![]);
    let error = create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("test-key".into())),
        location: Some("not a region".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .call("test-model")
    .do_generate(&prompt())
    .await
    .expect_err("the mock answers nothing");
    assert!(
        !matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.starts_with("Invalid Google Vertex location.")),
        "{error:?}"
    );
    assert_eq!(fetch.seen().len(), 1);
}

/// TS: "validates location for the separate transcription host of %s".
/// Upstream's Gemini case is the live model; aimux's Gemini transcription
/// models, unary and live, resolve the project and location the same way, and
/// the unary one runs without a socket.
#[tokio::test]
async fn transcription_validates_the_location_with_a_custom_url() {
    for model_id in ["chirp_3", "gemini-3.5-transcribe"] {
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
        let options = TranscriptionCallOptions::new(AudioInput::Binary(vec![0, 1, 2]), "audio/wav");
        let error = model.do_generate(&options).await.unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.starts_with("Invalid Google Vertex location.")),
            "{model_id}: {error:?}"
        );
        assert!(fetch.seen().is_empty(), "{model_id}");
    }
}

/// google-vertex-transcription-model.ts:113-120: a `region` provider option
/// replaces the location and is checked the same way.
#[tokio::test]
async fn transcription_validates_the_region_option() {
    let fetch = MockFetch::new(vec![]);
    let model = create_google_vertex(VertexProviderSettings {
        project: Some("test".into()),
        location: Some("us-central1".into()),
        access_token: Some("test-token".to_string().into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("chirp_3");
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(vec![0, 1, 2]), "audio/wav");
    options.provider_options = Some(
        serde_json::from_value(serde_json::json!({"googleVertex": {"region": "evil.example/#"}}))
            .unwrap(),
    );
    let error = model.do_generate(&options).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, .. } if message.starts_with("Invalid Google Cloud Speech-to-Text region.")),
        "{error:?}"
    );
    assert!(fetch.seen().is_empty());
}
