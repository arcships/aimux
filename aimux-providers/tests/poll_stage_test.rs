//! The request that creates an asynchronous job is sent exactly once: an
//! exhausted poll reports its error and never submits a second generation.
//!
//! The provider transport is a scripted [`RouteFetch`].

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use serde_json::json;

use aimux_core::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_provider_utils::Resolvable;
use aimux_providers::luma::{LumaProviderSettings, create_luma};

use mock_fetch::{Canned, RouteFetch};

/// The attempt budget the package fixes. Kept equal to the package's constant
/// on purpose: a change of a budget is a decision, and it shows up here.
const LUMA_MAX_POLL_ATTEMPTS: usize = 120;

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

fn image_options(poll_interval_ms: u64) -> ImageCallOptions {
    let mut options = ImageCallOptions::new("a cat".to_string());
    options.provider_options = aimux_core::shared::provider_namespace(
        "luma",
        json!({ "pollIntervalMillis": poll_interval_ms }),
    );
    options
}

fn is_status(error: &AiMuxError, status: u16) -> bool {
    matches!(error, AiMuxError::ApiCall(detail) if detail.status_code == Some(status))
}

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
        .do_generate(&image_options(0))
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
