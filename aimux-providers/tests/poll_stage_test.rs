//! The poll and download stages of the asynchronous-job vendors.
//!
//! Each package fixes its attempt budget in a constant; `providerOptions.<ns>.
//! pollIntervalMs` shortens or lengthens the wait between attempts and nothing
//! else. Whatever happens in a later stage, the request that creates the job is
//! sent exactly once: an exhausted poll or download reports its error and never
//! submits a second generation.
//!
//! Requests that stay on the provider transport run through a scripted
//! [`RouteFetch`]; the SSRF-guarded ones (Gladia and Black Forest Labs polls,
//! and every download of a provider-supplied URL) go to a local `wiremock`
//! server, whose origin is the configured base URL and therefore trusted.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::AbortSignal;
use aimux_core::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};
use aimux_provider_utils::Resolvable;
use aimux_providers::assemblyai::{AssemblyAIProviderSettings, create_assemblyai};
use aimux_providers::black_forest_labs::{
    BlackForestLabsProviderSettings, create_black_forest_labs,
};
use aimux_providers::fal::{FalProviderSettings, create_fal};
use aimux_providers::gladia::{GladiaProviderSettings, create_gladia};
use aimux_providers::luma::{LumaProviderSettings, create_luma};
use aimux_providers::recraft::{RecraftProviderSettings, create_recraft};
use aimux_providers::replicate::{ReplicateProviderSettings, create_replicate};
use aimux_providers::revai::{RevaiProviderSettings, create_revai};

use mock_fetch::{Canned, RouteFetch, Seen};

/// Attempt budgets the packages fix. Kept equal to the packages' constants on
/// purpose: a change of a budget is a decision, and it shows up here.
const LUMA_MAX_POLL_ATTEMPTS: usize = 120;
const BFL_MAX_POLL_ATTEMPTS: usize = 120;
const STT_MAX_POLL_ATTEMPTS: usize = 6_000;
const DOWNLOAD_ATTEMPTS: usize = 3;

fn key() -> Option<Resolvable<String>> {
    Some(Resolvable::Value("test-key".to_string()))
}

fn unavailable() -> Canned {
    Canned {
        status: 503,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("retry-after-ms".into(), "0".into()),
        ],
        body: serde_json::to_vec(&json!({ "error": "unavailable", "detail": "unavailable" }))
            .unwrap(),
    }
}

/// The providerOptions key each package reads for the poll interval: the
/// upstream name where `@ai-sdk/*` defines one (Luma, Black Forest Labs), else
/// `pollIntervalMs`.
fn interval_key(namespace: &str) -> &'static str {
    match namespace {
        "luma" | "blackForestLabs" => "pollIntervalMillis",
        _ => "pollIntervalMs",
    }
}

fn image_options(namespace: &str, poll_interval_ms: u64) -> ImageCallOptions {
    let mut options = ImageCallOptions::new("a cat".to_string());
    options.provider_options.insert(
        namespace.to_string(),
        json!({ interval_key(namespace): poll_interval_ms }),
    );
    options
}

fn transcription_options(namespace: &str, poll_interval_ms: u64) -> TranscriptionCallOptions {
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
    options.provider_options = Some(HashMap::from([(
        namespace.to_string(),
        json!({ interval_key(namespace): poll_interval_ms }),
    )]));
    options
}

fn is_status(error: &AiMuxError, status: u16) -> bool {
    matches!(error, AiMuxError::ApiCall(detail) if detail.status_code == Some(status))
}

/// Gaps between consecutive requests whose path matches.
fn gaps(seen: &[Seen], path: &str) -> Vec<Duration> {
    let times: Vec<_> = seen
        .iter()
        .filter(|s| url::Url::parse(&s.url).is_ok_and(|u| u.path() == path))
        .map(|s| s.at)
        .collect();
    times.windows(2).map(|w| w[1] - w[0]).collect()
}

// ── Luma ─────────────────────────────────────────────────────────────────────

fn luma_route(
    poll: impl Fn(usize) -> Canned + Send + Sync + 'static,
) -> std::sync::Arc<RouteFetch> {
    let polls = std::sync::atomic::AtomicUsize::new(0);
    RouteFetch::new(move |seen| {
        if seen.method == "POST" {
            Canned::json(&json!({ "id": "gen-1", "state": "queued" }))
        } else {
            poll(polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
        }
    })
}

fn luma_model(route: &std::sync::Arc<RouteFetch>) -> impl ImageModel {
    create_luma(LumaProviderSettings {
        api_key: key(),
        base_url: Some("http://luma.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .image("ray2")
}

#[tokio::test]
async fn luma_poll_exhaustion_does_not_submit_a_second_generation() {
    let route = luma_route(|_| unavailable());
    let error = luma_model(&route)
        .do_generate(&image_options("luma", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(
        route.count("POST", "/dream-machine/v1/generations/image"),
        1
    );
    assert_eq!(
        route.count("GET", "/dream-machine/v1/generations/gen-1"),
        LUMA_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn luma_still_pending_after_the_last_poll_is_a_timeout() {
    let route = luma_route(|_| Canned::json(&json!({ "id": "gen-1", "state": "queued" })));
    let error = luma_model(&route)
        .do_generate(&image_options("luma", 0))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiMuxError::Timeout(ref m) if m.contains("gen-1")),
        "{error:?}"
    );
    assert_eq!(
        route.count("POST", "/dream-machine/v1/generations/image"),
        1
    );
    assert_eq!(
        route.count("GET", "/dream-machine/v1/generations/gen-1"),
        LUMA_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn luma_poll_recovers_from_a_transient_error_without_resubmitting() {
    // 503, queued, then completed (the image URL is never fetched here: the
    // download goes to a path of the same fake host and fails, which is fine —
    // what is under test is that the poll got that far).
    let route = luma_route(|n| match n {
        0 => unavailable(),
        1 => Canned::json(&json!({ "id": "gen-1", "state": "queued" })),
        _ => Canned::json(&json!({
            "id": "gen-1", "state": "completed", "assets": { "image": "http://127.0.0.1:1/x.png" }
        })),
    });
    let _ = luma_model(&route)
        .do_generate(&image_options("luma", 0))
        .await;
    assert_eq!(
        route.count("POST", "/dream-machine/v1/generations/image"),
        1
    );
    assert_eq!(route.count("GET", "/dream-machine/v1/generations/gen-1"), 3);
}

#[tokio::test]
async fn luma_poll_interval_override_sets_the_wait_between_polls() {
    // Default is 500 ms. 80 ms must be what the loop waits.
    let route = luma_route(|_| Canned::json(&json!({ "id": "gen-1", "state": "queued" })));
    let model = luma_model(&route);
    let mut options = image_options("luma", 80);
    // Stop after a few polls: abort once the third poll has been seen.
    let signal = AbortSignal::new();
    options.abort_signal = Some(signal.clone());
    let watcher = {
        let route = route.clone();
        tokio::spawn(async move {
            while route.count("GET", "/dream-machine/v1/generations/gen-1") < 4 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            signal.abort();
        })
    };
    let error = model.do_generate(&options).await.unwrap_err();
    watcher.await.unwrap();
    assert!(matches!(error, AiMuxError::Aborted(_)), "{error:?}");
    let gaps = gaps(&route.seen(), "/dream-machine/v1/generations/gen-1");
    assert!(gaps.len() >= 3);
    for gap in gaps {
        assert!(gap >= Duration::from_millis(70), "{gap:?}");
        assert!(gap < Duration::from_millis(400), "{gap:?}");
    }
}

#[tokio::test]
async fn luma_poll_stops_on_abort_before_the_next_wait() {
    let signal = AbortSignal::new();
    let route = {
        let signal = signal.clone();
        luma_route(move |_| {
            // The first poll aborts the call.
            signal.abort();
            Canned::json(&json!({ "id": "gen-1", "state": "queued" }))
        })
    };
    let mut options = image_options("luma", 0);
    options.abort_signal = Some(signal);
    let error = luma_model(&route).do_generate(&options).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Aborted(_)), "{error:?}");
    assert_eq!(route.count("GET", "/dream-machine/v1/generations/gen-1"), 1);
    assert_eq!(
        route.count("POST", "/dream-machine/v1/generations/image"),
        1
    );
}

// ── Speech-to-text vendors on the provider transport ─────────────────────────

fn counting_route(
    submit_paths: &'static [&'static str],
    submit: Canned,
    poll_prefix: &'static str,
    poll: Canned,
) -> std::sync::Arc<RouteFetch> {
    RouteFetch::new(move |seen| {
        let url_path = url::Url::parse(&seen.url).unwrap().path().to_string();
        if seen.method == "POST" && submit_paths.contains(&url_path.as_str()) {
            if url_path.ends_with("/upload") {
                return Canned::json(&json!({ "upload_url": "https://upload.example/test" }));
            }
            return submit.clone();
        }
        if seen.method == "GET" && url_path.starts_with(poll_prefix) {
            return poll.clone();
        }
        Canned::json_status(
            404,
            &json!({ "error": "unexpected request", "path": url_path }),
        )
    })
}

#[tokio::test]
async fn assemblyai_poll_exhaustion_does_not_resubmit_or_reupload() {
    let route = counting_route(
        &["/v2/upload", "/v2/transcript"],
        Canned::json(&json!({ "id": "t1", "status": "queued" })),
        "/v2/transcript/t1",
        unavailable(),
    );
    let model = create_assemblyai(AssemblyAIProviderSettings {
        api_key: key(),
        base_url: Some("http://assemblyai.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("universal-2");
    let error = model
        .do_generate(&transcription_options("assemblyai", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(route.count("POST", "/v2/upload"), 1);
    assert_eq!(route.count("POST", "/v2/transcript"), 1);
    assert_eq!(
        route.count("GET", "/v2/transcript/t1"),
        STT_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn assemblyai_poll_interval_override_sets_the_wait_between_polls() {
    // 100 ms instead of the default; the third poll completes the job.
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let route = RouteFetch::new(move |seen| {
        let url_path = url::Url::parse(&seen.url).unwrap().path().to_string();
        match (seen.method.as_str(), url_path.as_str()) {
            ("POST", "/v2/upload") => Canned::json(&json!({ "upload_url": "https://u.example/t" })),
            ("POST", "/v2/transcript") => Canned::json(&json!({ "id": "t1", "status": "queued" })),
            _ if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 => {
                Canned::json(&json!({ "id": "t1", "status": "processing" }))
            }
            _ => Canned::json(&json!({ "id": "t1", "status": "completed", "text": "done" })),
        }
    });
    let model = create_assemblyai(AssemblyAIProviderSettings {
        api_key: key(),
        base_url: Some("http://assemblyai.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("universal-2");
    let result = model
        .do_generate(&transcription_options("assemblyai", 100))
        .await
        .unwrap();
    assert_eq!(result.text, "done");
    let gaps = gaps(&route.seen(), "/v2/transcript/t1");
    assert_eq!(gaps.len(), 2);
    for gap in gaps {
        // The old fixed wait was 100 ms as well; the override below the old
        // value is covered by the Luma test, this one pins the shape.
        assert!(gap >= Duration::from_millis(90), "{gap:?}");
        assert!(gap < Duration::from_millis(400), "{gap:?}");
    }
}

#[tokio::test]
async fn revai_poll_exhaustion_does_not_resubmit() {
    let route = counting_route(
        &["/speechtotext/v1/jobs"],
        Canned::json(&json!({ "id": "j1", "status": "in_progress", "language": "en" })),
        "/speechtotext/v1/jobs/j1",
        unavailable(),
    );
    let model = create_revai(RevaiProviderSettings {
        api_key: key(),
        base_url: Some("http://revai.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("machine");
    let error = model
        .do_generate(&transcription_options("revai", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(route.count("POST", "/speechtotext/v1/jobs"), 1);
    assert_eq!(
        route.count("GET", "/speechtotext/v1/jobs/j1"),
        STT_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn fal_transcription_poll_exhaustion_does_not_resubmit() {
    let route = counting_route(
        &["/fal-ai/wizper"],
        Canned::json(&json!({ "request_id": "r1" })),
        "/fal-ai/wizper/requests/r1",
        unavailable(),
    );
    let model = create_fal(FalProviderSettings {
        api_key: key(),
        base_url: Some("http://fal.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("wizper");
    let error = model
        .do_generate(&transcription_options("fal", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(route.count("POST", "/fal-ai/wizper"), 1);
    assert_eq!(
        route.count("GET", "/fal-ai/wizper/requests/r1"),
        STT_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn fal_queue_404_is_pending_and_spends_attempts() {
    // 404 while the request registers is "not yet", never a failure.
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let route = RouteFetch::new(move |seen| {
        if seen.method == "POST" {
            return Canned::json(&json!({ "request_id": "r1" }));
        }
        if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 3 {
            Canned::json_status(404, &json!({ "detail": "not found" }))
        } else {
            Canned::json(&json!({ "text": "hello" }))
        }
    });
    let model = create_fal(FalProviderSettings {
        api_key: key(),
        base_url: Some("http://fal.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("wizper");
    let result = model
        .do_generate(&transcription_options("fal", 0))
        .await
        .unwrap();
    assert_eq!(result.text, "hello");
    assert_eq!(route.count("POST", "/fal-ai/wizper"), 1);
    assert_eq!(route.count("GET", "/fal-ai/wizper/requests/r1"), 4);
}

// ── SSRF-guarded polls (Gladia, Black Forest Labs) ───────────────────────────

async fn request_count(server: &MockServer, method_name: &str, request_path: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == method_name && r.url.path() == request_path)
        .count()
}

#[tokio::test]
async fn gladia_poll_exhaustion_does_not_resubmit_or_reupload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/upload"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "audio_url": "https://u.example/a" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/pre-recorded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "result_url": format!("{}/v2/pre-recorded/job-1", server.uri()) }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/pre-recorded/job-1"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after-ms", "0")
                .set_body_json(json!({ "error": { "message": "unavailable" } })),
        )
        .mount(&server)
        .await;
    let model = create_gladia(GladiaProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .transcription("default");
    let error = model
        .do_generate(&transcription_options("gladia", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(request_count(&server, "POST", "/v2/upload").await, 1);
    assert_eq!(request_count(&server, "POST", "/v2/pre-recorded").await, 1);
    assert_eq!(
        request_count(&server, "GET", "/v2/pre-recorded/job-1").await,
        STT_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn black_forest_labs_poll_exhaustion_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/flux-pro-1.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "img-1",
            "polling_url": format!("{}/v1/get_result", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/get_result"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after-ms", "0")
                .set_body_json(json!({ "detail": "unavailable" })),
        )
        .mount(&server)
        .await;
    let model = create_black_forest_labs(BlackForestLabsProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("flux-pro-1.1");
    let error = model
        .do_generate(&image_options("blackForestLabs", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(request_count(&server, "POST", "/flux-pro-1.1").await, 1);
    assert_eq!(
        request_count(&server, "GET", "/v1/get_result").await,
        BFL_MAX_POLL_ATTEMPTS
    );
}

#[tokio::test]
async fn black_forest_labs_poll_interval_override_sets_the_wait_between_polls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/flux-pro-1.1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "img-1",
            "polling_url": format!("{}/v1/get_result", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/get_result"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "Pending" })))
        .mount(&server)
        .await;
    let signal = AbortSignal::new();
    let mut options = image_options("blackForestLabs", 60);
    options.abort_signal = Some(signal.clone());
    let watcher = {
        let server_uri = server.uri();
        let signal = signal.clone();
        let started = std::time::Instant::now();
        tokio::spawn(async move {
            // Four polls at 60 ms (the default is 500 ms) fit well inside a
            // second; abort after that to end the call.
            let _ = server_uri;
            tokio::time::sleep(Duration::from_millis(600)).await;
            signal.abort();
            started.elapsed()
        })
    };
    let model = create_black_forest_labs(BlackForestLabsProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("flux-pro-1.1");
    let error = model.do_generate(&options).await.unwrap_err();
    watcher.await.unwrap();
    assert!(matches!(error, AiMuxError::Aborted(_)), "{error:?}");
    // At 500 ms per wait only two polls fit in 600 ms; at 60 ms there are
    // many more.
    assert!(request_count(&server, "GET", "/v1/get_result").await >= 5);
    assert_eq!(request_count(&server, "POST", "/flux-pro-1.1").await, 1);
}

// ── Downloads (Replicate, Recraft, Fal, Luma) ────────────────────────────────

#[tokio::test]
async fn replicate_download_exhaustion_does_not_submit_a_second_prediction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/black-forest-labs/flux-schnell/predictions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "p1", "status": "succeeded", "output": [format!("{}/out.png", server.uri())]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/out.png"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after-ms", "0"))
        .mount(&server)
        .await;
    let model = create_replicate(ReplicateProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("black-forest-labs/flux-schnell");
    let error = model
        .do_generate(&image_options("replicate", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(
        request_count(
            &server,
            "POST",
            "/models/black-forest-labs/flux-schnell/predictions"
        )
        .await,
        1
    );
    assert_eq!(
        request_count(&server, "GET", "/out.png").await,
        DOWNLOAD_ATTEMPTS
    );
}

#[tokio::test]
async fn recraft_download_exhaustion_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": [{ "url": format!("{}/out.png", server.uri()) }] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/out.png"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after-ms", "0"))
        .mount(&server)
        .await;
    let model = create_recraft(RecraftProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("recraftv3");
    let error = model
        .do_generate(&image_options("recraft", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(
        request_count(&server, "POST", "/images/generations").await,
        1
    );
    assert_eq!(
        request_count(&server, "GET", "/out.png").await,
        DOWNLOAD_ATTEMPTS
    );
}

#[tokio::test]
async fn fal_image_download_exhaustion_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fal-ai/flux/schnell"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "images": [{ "url": format!("{}/out.png", server.uri()) }] }),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/out.png"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after-ms", "0"))
        .mount(&server)
        .await;
    let model = create_fal(FalProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("fal-ai/flux/schnell");
    // The image endpoint is `{base}/{model_id}`.
    let error = model
        .do_generate(&image_options("fal", 0))
        .await
        .unwrap_err();
    assert!(
        is_status(&error, 503) || is_status(&error, 404),
        "{error:?}"
    );
    assert_eq!(
        request_count(&server, "POST", "/fal-ai/flux/schnell").await,
        1
    );
    assert_eq!(
        request_count(&server, "GET", "/out.png").await,
        DOWNLOAD_ATTEMPTS
    );
}

#[tokio::test]
async fn luma_download_exhaustion_does_not_submit_a_second_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/dream-machine/v1/generations/image"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": "g1", "state": "queued" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dream-machine/v1/generations/g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "g1", "state": "completed", "assets": { "image": format!("{}/out.png", server.uri()) }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/out.png"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after-ms", "0"))
        .mount(&server)
        .await;
    let model = create_luma(LumaProviderSettings {
        api_key: key(),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image("ray2");
    let error = model
        .do_generate(&image_options("luma", 0))
        .await
        .unwrap_err();
    assert!(is_status(&error, 503), "{error:?}");
    assert_eq!(
        request_count(&server, "POST", "/dream-machine/v1/generations/image").await,
        1
    );
    assert_eq!(
        request_count(&server, "GET", "/out.png").await,
        DOWNLOAD_ATTEMPTS
    );
}

#[tokio::test]
async fn the_attempt_budget_is_not_a_provider_option() {
    // `maxPollAttempts` used to be read from providerOptions; the budget is a
    // package constant now, so the option changes nothing.
    let route = luma_route(|_| Canned::json(&json!({ "id": "gen-1", "state": "queued" })));
    let mut options = image_options("luma", 0);
    options.provider_options.insert(
        "luma".to_string(),
        json!({ "pollIntervalMillis": 0, "maxPollAttempts": 1 }),
    );
    let error = luma_model(&route).do_generate(&options).await.unwrap_err();
    assert!(matches!(error, AiMuxError::Timeout(_)), "{error:?}");
    assert_eq!(
        route.count("GET", "/dream-machine/v1/generations/gen-1"),
        LUMA_MAX_POLL_ATTEMPTS
    );
}

// ── Pacing keys never reach a vendor body ────────────────────────────────────

const CONTROL_KEYS: [&str; 4] = [
    "pollIntervalMs",
    "pollIntervalMillis",
    "pollTimeoutMillis",
    "maxPollAttempts",
];

/// Every pacing key, plus one ordinary field that must still be forwarded.
fn with_control_keys(extra: serde_json::Value) -> serde_json::Value {
    let mut fields = extra.as_object().cloned().unwrap_or_default();
    for key in CONTROL_KEYS {
        fields.insert(key.to_string(), json!(0));
    }
    serde_json::Value::Object(fields)
}

/// A transport that lets the upload-like first stage through and fails the
/// job-creating request, so the call ends right after the body was sent.
fn failing_route(let_through: &'static [&'static str]) -> std::sync::Arc<RouteFetch> {
    RouteFetch::new(move |seen| {
        let url_path = url::Url::parse(&seen.url).unwrap().path().to_string();
        if let_through.contains(&url_path.as_str()) {
            Canned::json(&json!({
                "upload_url": "https://u.example/a",
                "audio_url": "https://u.example/a"
            }))
        } else {
            Canned::json_status(400, &json!({ "error": "stop", "detail": "stop" }))
        }
    })
}

/// The JSON body of the request that created the job (the one on `post_path`).
fn created_body(route: &RouteFetch, post_path: &str) -> serde_json::Value {
    let seen = route.seen();
    let request = seen
        .iter()
        .find(|s| s.method == "POST" && url::Url::parse(&s.url).unwrap().path() == post_path)
        .unwrap_or_else(|| panic!("no POST {post_path}: {seen:?}"));
    request.json_body()
}

fn assert_no_control_keys(body: &serde_json::Value, forwarded: &str) {
    let object = body.as_object().expect("JSON object body");
    for key in CONTROL_KEYS {
        assert!(!object.contains_key(key), "{key} reached the body: {body}");
    }
    assert!(
        object.contains_key(forwarded),
        "{forwarded} not forwarded: {body}"
    );
}

#[tokio::test]
async fn luma_does_not_forward_pacing_keys() {
    let route = failing_route(&[]);
    let mut options = ImageCallOptions::new("a cat".to_string());
    options.provider_options.insert(
        "luma".to_string(),
        with_control_keys(json!({ "custom": 1 })),
    );
    let _ = luma_model(&route).do_generate(&options).await;
    assert_no_control_keys(
        &created_body(&route, "/dream-machine/v1/generations/image"),
        "custom",
    );
}

#[tokio::test]
async fn black_forest_labs_does_not_forward_pacing_keys() {
    let route = failing_route(&[]);
    let mut options = ImageCallOptions::new("a cat".to_string());
    options.provider_options.insert(
        "blackForestLabs".to_string(),
        with_control_keys(json!({ "steps": 3 })),
    );
    let model = create_black_forest_labs(BlackForestLabsProviderSettings {
        api_key: key(),
        base_url: Some("http://bfl.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .image("flux-pro-1.1");
    let _ = model.do_generate(&options).await;
    assert_no_control_keys(&created_body(&route, "/flux-pro-1.1"), "steps");
}

#[tokio::test]
async fn fal_image_does_not_forward_pacing_keys() {
    let route = failing_route(&[]);
    let mut options = ImageCallOptions::new("a cat".to_string());
    options
        .provider_options
        .insert("fal".to_string(), with_control_keys(json!({ "custom": 1 })));
    let model = create_fal(FalProviderSettings {
        api_key: key(),
        base_url: Some("http://fal.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .image("fal-ai/flux/schnell");
    let _ = model.do_generate(&options).await;
    assert_no_control_keys(&created_body(&route, "/fal-ai/flux/schnell"), "custom");
}

#[tokio::test]
async fn gladia_does_not_forward_pacing_keys() {
    let route = failing_route(&["/v2/upload"]);
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
    options.provider_options = Some(HashMap::from([(
        "gladia".to_string(),
        with_control_keys(json!({ "custom_vocabulary": ["a"] })),
    )]));
    let model = create_gladia(GladiaProviderSettings {
        api_key: key(),
        base_url: Some("http://gladia.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("default");
    let _ = model.do_generate(&options).await;
    assert_no_control_keys(
        &created_body(&route, "/v2/pre-recorded"),
        "custom_vocabulary",
    );
}

#[tokio::test]
async fn assemblyai_does_not_forward_pacing_keys() {
    let route = failing_route(&["/v2/upload"]);
    let mut options = TranscriptionCallOptions::new(AudioInput::Binary(vec![1, 2, 3]), "audio/wav");
    options.provider_options = Some(HashMap::from([(
        "assemblyai".to_string(),
        with_control_keys(json!({ "punctuate": true })),
    )]));
    let model = create_assemblyai(AssemblyAIProviderSettings {
        api_key: key(),
        base_url: Some("http://assemblyai.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .transcription("universal-2");
    let _ = model.do_generate(&options).await;
    assert_no_control_keys(&created_body(&route, "/v2/transcript"), "punctuate");
}

#[tokio::test]
async fn replicate_does_not_forward_pacing_keys() {
    let route = failing_route(&[]);
    let mut options = ImageCallOptions::new("a cat".to_string());
    options.provider_options.insert(
        "replicate".to_string(),
        with_control_keys(json!({ "custom": 1, "maxWaitTimeInSeconds": 5 })),
    );
    let model = create_replicate(ReplicateProviderSettings {
        api_key: key(),
        base_url: Some("http://replicate.test".to_string()),
        fetch: Some(route.transport()),
        ..Default::default()
    })
    .unwrap()
    .image("black-forest-labs/flux-schnell");
    let _ = model.do_generate(&options).await;
    let body = created_body(&route, "/models/black-forest-labs/flux-schnell/predictions");
    // Replicate nests the options under `input`.
    let input = &body["input"];
    assert_no_control_keys(input, "custom");
    assert!(input.get("maxWaitTimeInSeconds").is_none(), "{input}");
}
