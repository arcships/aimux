//! `create_google_vertex`: modes, hosts and credential timing.
//!
//! `@ai-sdk/google-vertex` has no recorded fixtures, so these tests pin
//! `google-vertex-provider-base.ts` and `google-vertex-provider.ts` directly,
//! through an injected [`Fetch`] (the factory's `fetch` setting) that records
//! each request: the Express-mode and standard-mode paths, the three host
//! shapes (`global`, the `us`/`eu` multi-region hosts and regional ones), the
//! location check, and that nothing is read before a request is made.
//!
//! The standard-mode token is the one place aimux differs from the SDK: it has
//! no Application Default Credentials library, so the token is the
//! `access_token` setting (a `Resolvable`, refreshed by the host) or
//! `GOOGLE_VERTEX_ACCESS_TOKEN`.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use serial_test::serial;

use aimux_core::AiMuxError;
use aimux_core::content::ContentPart;
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::transcription_model::TranscriptionModel;
use aimux_core::video_model::VideoModel;
use aimux_provider_utils::{HeaderMapOpt, Resolvable};
use aimux_providers::vertex::{
    VertexProvider, VertexProviderSettings, create_google_vertex, google_vertex,
};

use mock_fetch::{Canned, EnvVar, MockFetch};

const ENV_VARS: [&str; 4] = [
    "GOOGLE_VERTEX_API_KEY",
    "GOOGLE_VERTEX_ACCESS_TOKEN",
    "GOOGLE_VERTEX_PROJECT",
    "GOOGLE_VERTEX_LOCATION",
];

/// Remove every Vertex environment variable for the length of a test.
fn clean_env() -> Vec<EnvVar> {
    ENV_VARS
        .iter()
        .map(|name| EnvVar::set(name, None))
        .collect()
}

fn user_prompt(text: &str) -> CallOptions {
    CallOptions::new(vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text(text)],
        ..Default::default()
    }])
}

fn gemini_response() -> Canned {
    Canned::json(&json!({
        "candidates": [{
            "content": { "parts": [{ "text": "ok" }], "role": "model" },
            "finishReason": "STOP", "index": 0
        }],
        "usageMetadata": { "promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2 }
    }))
}

fn claude_response() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }))
}

/// Standard mode with a token, a project and a location.
fn standard(mock: &Arc<MockFetch>, location: &str) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("tok".to_string())),
        project: Some("proj".to_string()),
        location: Some(location.to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// Express mode.
fn express(mock: &Arc<MockFetch>) -> VertexProvider {
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("express-key".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
}

// ── the two modes ────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn express_mode_uses_the_express_host_and_the_api_key_header() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);
    // No project, location or token anywhere: Express mode needs none.
    let model = express(&mock).chat("gemini-2.5-flash");
    assert_eq!(model.provider(), "google.vertex");

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "express-key");
    assert!(!seen[0].headers.contains_key("authorization"));
}

#[serial]
#[tokio::test]
async fn standard_mode_uses_the_scoped_host_and_a_bearer_token() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);
    let model = standard(&mock, "us-central1").chat("gemini-2.5-flash");
    assert_eq!(model.provider(), "google.vertex");

    model.do_generate(&user_prompt("hi")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer tok");
    assert!(!seen[0].headers.contains_key("x-goog-api-key"));
}

#[serial]
#[tokio::test]
async fn the_streaming_endpoint_keeps_the_sse_query() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: b"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"a\"}]},\"finishReason\":\"STOP\",\"index\":0}]}\n\n".to_vec(),
    }]);
    let result = standard(&mock, "global")
        .chat("gemini-2.5-flash")
        .do_stream(&user_prompt("hi"))
        .await
        .unwrap();
    drop(result);
    assert_eq!(
        mock.seen()[0].url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
}

#[serial]
#[tokio::test]
async fn hosts_follow_the_location() {
    let _env = clean_env();
    for (location, host) in [
        ("global", "aiplatform.googleapis.com"),
        ("us", "aiplatform.us.rep.googleapis.com"),
        ("eu", "aiplatform.eu.rep.googleapis.com"),
        ("europe-west4", "europe-west4-aiplatform.googleapis.com"),
    ] {
        let mock = MockFetch::new(vec![gemini_response()]);
        standard(&mock, location)
            .chat("gemini-2.5-flash")
            .do_generate(&user_prompt("hi"))
            .await
            .unwrap();
        assert_eq!(
            mock.seen()[0].url,
            format!(
                "https://{host}/v1beta1/projects/proj/locations/{location}/publishers/google/models/gemini-2.5-flash:generateContent"
            ),
            "{location}"
        );
    }
}

#[serial]
#[tokio::test]
async fn base_url_replaces_the_host_in_both_modes() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response(), gemini_response()]);
    let provider = create_google_vertex(VertexProviderSettings {
        base_url: Some("https://gateway.example/vertex/".to_string()),
        access_token: Some(Resolvable::Value("tok".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    // No project or location needed either.
    provider
        .chat("gemini-2.5-flash")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();

    let express = create_google_vertex(VertexProviderSettings {
        base_url: Some("https://gateway.example/vertex".to_string()),
        api_key: Some(Resolvable::Value("k".to_string())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    express
        .chat("gemini-2.5-flash")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();

    let seen = mock.seen();
    for request in &seen {
        assert_eq!(
            request.url,
            "https://gateway.example/vertex/models/gemini-2.5-flash:generateContent"
        );
    }
    assert_eq!(seen[1].headers["x-goog-api-key"], "k");
}

#[serial]
#[tokio::test]
async fn tuned_models_are_served_from_the_endpoint_without_the_publisher() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);
    standard(&mock, "us-central1")
        .chat("endpoints/123")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();
    assert_eq!(
        mock.seen()[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/123:generateContent"
    );
}

// ── locations ────────────────────────────────────────────────────────────────

#[test]
fn an_explicit_location_must_be_one_dns_label() {
    for bad in ["evil.example", "a/b", "us:443", "x@y", "", "-a", "a-"] {
        let result = create_google_vertex(VertexProviderSettings {
            location: Some(bad.to_string()),
            ..Default::default()
        });
        match result {
            Err(AiMuxError::InvalidArgument(message)) => {
                assert!(
                    message.contains("base_url"),
                    "points at base_url: {message}"
                );
            }
            Err(other) => panic!("{bad:?}: expected InvalidArgument, got {other:?}"),
            Ok(_) => panic!("{bad:?}: expected an error"),
        }
    }
    assert!(
        create_google_vertex(VertexProviderSettings {
            location: Some("us-central1".to_string()),
            ..Default::default()
        })
        .is_ok()
    );
}

#[serial]
#[tokio::test]
async fn a_location_from_the_environment_is_checked_when_the_request_is_made() {
    let _env = clean_env();
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", Some("evil.example/x"));
    let mock = MockFetch::new(vec![gemini_response()]);
    let provider = create_google_vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("tok".to_string())),
        project: Some("proj".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("creation reads nothing");
    let error = provider
        .chat("gemini-2.5-flash")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap_err();
    assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
    assert!(mock.seen().is_empty());
}

#[test]
fn a_bad_base_url_fails_the_factory() {
    let result = create_google_vertex(VertexProviderSettings {
        base_url: Some("ftp://nope".to_string()),
        ..Default::default()
    });
    assert!(matches!(result, Err(AiMuxError::InvalidArgument(_))));
}

// ── nothing is read until a request ──────────────────────────────────────────

#[serial]
#[test]
fn the_default_instance_reads_nothing_and_cannot_fail() {
    let _env = clean_env();
    assert!(std::ptr::eq(google_vertex(), google_vertex()));
    // Models of every modality are made without a project, token or location.
    let provider = google_vertex();
    assert_eq!(provider.chat("m").provider(), "google.vertex");
    assert_eq!(provider.embedding("m").provider(), "google.vertex");
    assert_eq!(provider.image("m").provider(), "google.vertex");
    assert_eq!(provider.video("m").provider(), "google.vertex.video");
    assert_eq!(
        provider.transcription("m").provider(),
        "google.vertex.transcription"
    );
    assert_eq!(
        provider.anthropic_model("m").provider(),
        "googleVertex.anthropic.messages"
    );
}

#[serial]
#[tokio::test]
async fn missing_settings_fail_the_call_with_typed_errors_naming_the_variable() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);

    // Nothing at all: the location is looked for first.
    let provider = create_google_vertex(VertexProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    match provider
        .chat("m")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap_err()
    {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_LOCATION");
            assert_eq!(name, "location");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }

    // Location and project known, no token.
    let provider = create_google_vertex(VertexProviderSettings {
        location: Some("us-central1".to_string()),
        project: Some("proj".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap();
    match provider
        .chat("m")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap_err()
    {
        AiMuxError::LoadApiKey { env_var, .. } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_ACCESS_TOKEN");
        }
        other => panic!("expected LoadApiKey, got {other:?}"),
    }
    assert!(mock.seen().is_empty(), "nothing was sent");
}

#[serial]
#[tokio::test]
async fn the_environment_is_read_on_each_call() {
    let _env = clean_env();
    let _token = EnvVar::set("GOOGLE_VERTEX_ACCESS_TOKEN", Some("env-token"));
    let _project = EnvVar::set("GOOGLE_VERTEX_PROJECT", Some("env-project"));
    let _location = EnvVar::set("GOOGLE_VERTEX_LOCATION", Some("us"));
    let mock = MockFetch::new(vec![gemini_response(), gemini_response()]);
    let model = create_google_vertex(VertexProviderSettings {
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");

    model.do_generate(&user_prompt("one")).await.unwrap();
    unsafe { std::env::set_var("GOOGLE_VERTEX_ACCESS_TOKEN", "rotated") };
    model.do_generate(&user_prompt("two")).await.unwrap();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://aiplatform.us.rep.googleapis.com/v1beta1/projects/env-project/locations/us/publishers/google/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer env-token");
    assert_eq!(seen[1].headers["authorization"], "Bearer rotated");
}

#[serial]
#[tokio::test]
async fn an_access_token_producer_runs_on_every_request() {
    let _env = clean_env();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mock = MockFetch::new(vec![gemini_response(), gemini_response()]);
    let model = create_google_vertex(VertexProviderSettings {
        project: Some("proj".to_string()),
        location: Some("us-central1".to_string()),
        access_token: Some(Resolvable::from_async_fn(move || {
            let counted = counted.clone();
            async move { Ok(format!("token-{}", counted.fetch_add(1, Ordering::SeqCst))) }
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "creation evaluates nothing"
    );
    model.do_generate(&user_prompt("one")).await.unwrap();
    model.do_generate(&user_prompt("two")).await.unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer token-0");
    assert_eq!(seen[1].headers["authorization"], "Bearer token-1");
}

// ── headers ──────────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn headers_layer_over_the_token_and_the_key_wins_in_express_mode() {
    let _env = clean_env();
    let mut headers = HeaderMapOpt::new();
    headers.insert("X-Team".to_string(), Some("blue".to_string()));
    headers.insert("Authorization".to_string(), Some("Bearer user".to_string()));

    // Standard mode: the user's Authorization replaces the token's, and no
    // token is needed to be able to send it.
    let mock = MockFetch::new(vec![gemini_response()]);
    create_google_vertex(VertexProviderSettings {
        project: Some("proj".to_string()),
        location: Some("us-central1".to_string()),
        headers: Some(Resolvable::Value(headers.clone())),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    let seen = mock.seen();
    assert_eq!(seen[0].headers["authorization"], "Bearer user");
    assert_eq!(seen[0].headers["x-team"], "blue");

    // Express mode: x-goog-api-key is set last, so it always wins.
    headers.insert("x-goog-api-key".to_string(), Some("user-key".to_string()));
    let mock = MockFetch::new(vec![gemini_response()]);
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("express-key".to_string())),
        headers: Some(Resolvable::Value(headers)),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_eq!(mock.seen()[0].headers["x-goog-api-key"], "express-key");
}

// ── namespaces ───────────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn response_metadata_is_written_under_the_vertex_namespace() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);
    let result = standard(&mock, "us-central1")
        .chat("gemini-2.5-flash")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();
    let metadata = result.provider_metadata.unwrap();
    assert_eq!(
        metadata["googleVertex"]["usageMetadata"]["totalTokenCount"],
        2
    );
    assert!(metadata.get("google").is_none());
    assert!(
        metadata.get("vertex").is_none(),
        "the legacy `vertex` key is not written"
    );
}

#[serial]
#[tokio::test]
async fn provider_options_are_read_from_googlevertex_then_google() {
    let _env = clean_env();
    let cached = |options: Value| {
        let mut call = user_prompt("hi");
        call.provider_options = Some(
            options
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        call
    };
    for (options, expected) in [
        (
            json!({ "googleVertex": { "cachedContent": "c1" } }),
            Some("c1"),
        ),
        (json!({ "google": { "cachedContent": "c3" } }), Some("c3")),
        (
            json!({
                "googleVertex": { "cachedContent": "c1" },
                "google": { "cachedContent": "c3" },
            }),
            Some("c1"),
        ),
        // The historical `vertex` alias is neither read nor forwarded.
        (json!({ "vertex": { "cachedContent": "legacy" } }), None),
    ] {
        let mock = MockFetch::new(vec![gemini_response()]);
        standard(&mock, "us-central1")
            .chat("gemini-2.5-flash")
            .do_generate(&cached(options.clone()))
            .await
            .unwrap();
        let body = mock.seen()[0].json_body();
        assert_eq!(
            body.get("cachedContent").and_then(Value::as_str),
            expected,
            "{options}"
        );
    }
}

#[serial]
#[tokio::test]
async fn the_legacy_vertex_key_is_neither_read_nor_written_by_embeddings_and_images() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned::json(&json!({
        "predictions": [{ "embeddings": { "values": [0.5], "statistics": { "token_count": 2 } } }]
    }))]);
    standard(&mock, "us-central1")
        .embedding("textembedding-gecko@001")
        .do_embed(&EmbeddingCallOptions {
            values: vec!["a".to_string()],
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: Some(
                [(
                    "vertex".to_string(),
                    json!({ "taskType": "RETRIEVAL_QUERY" }),
                )]
                .into_iter()
                .collect(),
            ),
            headers: None,
        })
        .await
        .unwrap();
    assert!(
        mock.seen()[0].json_body()["instances"][0]
            .get("task_type")
            .is_none(),
        "options under `vertex` are ignored"
    );

    let mock = MockFetch::new(vec![Canned::json(
        &json!({ "predictions": [{ "bytesBase64Encoded": "AAAA" }] }),
    )]);
    let mut image = aimux_core::image_model::ImageCallOptions::new("a cat".to_string());
    image.provider_options = [(
        "vertex".to_string(),
        json!({ "personGeneration": "allow_all" }),
    )]
    .into_iter()
    .collect();
    let result = standard(&mock, "us-central1")
        .image("imagen-4.0-generate-001")
        .do_generate(&image)
        .await
        .unwrap();
    assert!(
        mock.seen()[0].json_body()["parameters"]
            .get("personGeneration")
            .is_none()
    );
    let metadata = result.provider_metadata.unwrap();
    assert!(metadata.contains_key("googleVertex"));
    assert!(!metadata.contains_key("vertex"));
}

// ── other modalities ─────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn embedding_and_image_go_through_the_same_endpoint_rules() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![
        Canned::json(&json!({
            "predictions": [{ "embeddings": { "values": [0.5], "statistics": { "token_count": 2 } } }]
        })),
        Canned::json(&json!({ "predictions": [{ "bytesBase64Encoded": "AAAA" }] })),
    ]);
    let provider = standard(&mock, "us-central1");
    provider
        .embedding("textembedding-gecko@001")
        .do_embed(&EmbeddingCallOptions {
            values: vec!["a".to_string()],
            abort_signal: None,
            max_retries: None,
            timeout: None,
            provider_options: None,
            headers: None,
        })
        .await
        .unwrap();
    provider
        .image("imagen-4.0-generate-001")
        .do_generate(&aimux_core::image_model::ImageCallOptions::new(
            "a cat".to_string(),
        ))
        .await
        .unwrap();
    let seen = mock.seen();
    let base = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/publishers/google/models";
    assert_eq!(
        seen[0].url,
        format!("{base}/textembedding-gecko@001:predict")
    );
    assert_eq!(
        seen[1].url,
        format!("{base}/imagen-4.0-generate-001:predict")
    );
    for request in &seen {
        assert_eq!(request.headers["authorization"], "Bearer tok");
    }
}

#[serial]
#[tokio::test]
async fn speech_to_text_needs_standard_mode() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![]);
    let error = express(&mock)
        .transcription("chirp_2")
        .do_generate(
            &aimux_core::transcription_model::TranscriptionCallOptions::new(
                aimux_core::transcription_model::AudioInput::Binary(vec![0, 1]),
                "audio/wav".to_string(),
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
    assert!(mock.seen().is_empty());
}

#[serial]
#[tokio::test]
async fn transform_request_body_rewrites_every_json_body() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![gemini_response()]);
    create_google_vertex(VertexProviderSettings {
        api_key: Some(Resolvable::Value("k".to_string())),
        transform_request_body: Some(Arc::new(|mut body: Value| {
            body["labels"] = json!({ "team": "blue" });
            body
        })),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .chat("gemini-2.5-flash")
    .do_generate(&user_prompt("hi"))
    .await
    .unwrap();
    assert_eq!(
        mock.seen()[0].json_body()["labels"],
        json!({ "team": "blue" })
    );
}

#[serial]
#[tokio::test]
async fn list_models_is_one_exchange_through_the_same_transport() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![Canned::json(&json!({
        "models": [{ "name": "models/gemini-2.5-flash" }]
    }))]);
    let models = express(&mock).list_models().await.unwrap();
    assert_eq!(models[0].id, "gemini-2.5-flash");
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://aiplatform.googleapis.com/v1/publishers/google/models"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "express-key");
}

#[test]
fn the_provider_offers_language_embedding_image_transcription_and_video() {
    let provider = create_google_vertex(VertexProviderSettings::default()).unwrap();
    assert!(provider.language_model("m").is_ok());
    assert!(provider.embedding_model("m").is_ok());
    assert!(provider.image_model("m").is_ok());
    assert!(provider.transcription_model("m").unwrap().is_ok());
    assert!(provider.video_model("m").unwrap().is_ok());
    assert!(provider.speech_model("s").is_none());
    assert!(Provider::files(&provider).is_none());
}

// ── Claude on Vertex ─────────────────────────────────────────────────────────

#[serial]
#[tokio::test]
async fn anthropic_models_use_the_publisher_path_and_the_same_credentials() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![claude_response(), claude_response()]);
    let provider = standard(&mock, "us-east5");
    let model = provider.anthropic_model("claude-sonnet-4-5");
    assert_eq!(model.provider(), "googleVertex.anthropic.messages");

    model.do_generate(&user_prompt("hi")).await.unwrap();
    model.do_stream(&user_prompt("hi")).await.ok();

    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/anthropic/models/claude-sonnet-4-5:rawPredict"
    );
    assert_eq!(seen[0].headers["authorization"], "Bearer tok");
    assert_eq!(
        seen[0].json_body()["anthropic_version"],
        "vertex-2023-10-16"
    );
    assert!(seen[0].json_body().get("model").is_none());
    assert!(seen[1].url.ends_with(":streamRawPredict"));
}

#[serial]
#[tokio::test]
async fn anthropic_in_express_mode_uses_the_express_root() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![claude_response()]);
    express(&mock)
        .anthropic_model("claude-sonnet-4-5")
        .do_generate(&user_prompt("hi"))
        .await
        .unwrap();
    let seen = mock.seen();
    assert_eq!(
        seen[0].url,
        "https://aiplatform.googleapis.com/v1/publishers/anthropic/models/claude-sonnet-4-5:rawPredict"
    );
    assert_eq!(seen[0].headers["x-goog-api-key"], "express-key");
}

#[serial]
#[tokio::test]
async fn anthropic_resolves_its_project_when_the_call_is_made() {
    let _env = clean_env();
    let mock = MockFetch::new(vec![claude_response()]);
    // Creating the provider and the model needs no project.
    let model = create_google_vertex(VertexProviderSettings {
        access_token: Some(Resolvable::Value("tok".to_string())),
        location: Some("us-east5".to_string()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .anthropic_model("claude-sonnet-4-5");
    match model.do_generate(&user_prompt("hi")).await.unwrap_err() {
        AiMuxError::LoadSetting { env_var, name } => {
            assert_eq!(env_var, "GOOGLE_VERTEX_PROJECT");
            assert_eq!(name, "project");
        }
        other => panic!("expected LoadSetting, got {other:?}"),
    }
    assert!(mock.seen().is_empty());
}
