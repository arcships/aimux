//! A synchronous image generation slower than 30s succeeds through Core
//! `generate_image` with the default retries, and a generation that times
//! out is submitted once, not three times (#527).
//!
//! The clock is paused: a transport that answers after 35 virtual seconds
//! takes no real time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::json;

use aimux_core::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, generate_image};
use aimux_core::options::TimeoutConfiguration;
use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};
use aimux_providers::openai::{OpenAIProviderSettings, create_openai};
use aimux_providers::openai_compatible::{
    OpenAICompatibleProviderSettings, create_openai_compatible,
};

/// Answers every request with one image after `delay`, counting requests.
struct SlowImageEndpoint {
    delay: Duration,
    calls: AtomicUsize,
}

impl SlowImageEndpoint {
    fn new(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            calls: AtomicUsize::new(0),
        })
    }

    fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Fetch for SlowImageEndpoint {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            headers,
            request.url,
            Bytes::from(json!({"created": 1, "data": [{"b64_json": "AAAA"}]}).to_string()),
        ))
    }
}

fn openai(endpoint: &Arc<SlowImageEndpoint>) -> Arc<dyn ImageModel> {
    Arc::new(
        create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            fetch: Some(endpoint.transport()),
            ..Default::default()
        })
        .unwrap()
        .image("gpt-image-1"),
    )
}

fn openai_compatible(endpoint: &Arc<SlowImageEndpoint>) -> Arc<dyn ImageModel> {
    Arc::new(
        create_openai_compatible(OpenAICompatibleProviderSettings {
            name: "relay".into(),
            base_url: "https://relay.example.test/v1".into(),
            api_key: Some("test-api-key".into()),
            fetch: Some(endpoint.transport()),
            ..Default::default()
        })
        .unwrap()
        .image("gpt-image-1"),
    )
}

fn models(endpoint: &Arc<SlowImageEndpoint>) -> [(&'static str, Arc<dyn ImageModel>); 2] {
    [
        ("openai", openai(endpoint)),
        ("openai_compatible", openai_compatible(endpoint)),
    ]
}

/// No `max_retries`, no `timeout`: the defaults the issue reproduces with.
#[tokio::test(start_paused = true)]
async fn an_image_generation_slower_than_30s_succeeds_once() {
    for name in ["openai", "openai_compatible"] {
        let endpoint = SlowImageEndpoint::new(Duration::from_secs(35));
        let (_, model) = models(&endpoint)
            .into_iter()
            .find(|(model, _)| *model == name)
            .unwrap();
        let result = generate_image(model.as_ref(), ImageCallOptions::new("otter"))
            .await
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let ImageOutputs::Base64(images) = result.images else {
            panic!("{name}: expected base64 images");
        };
        assert_eq!(images, vec!["AAAA".to_string()], "{name}");
        assert_eq!(endpoint.calls(), 1, "{name}");
    }
}

/// A generation that outlives its exchange guard fails without being
/// submitted again.
#[tokio::test(start_paused = true)]
async fn a_timed_out_image_generation_is_not_resubmitted() {
    for name in ["openai", "openai_compatible"] {
        let endpoint = SlowImageEndpoint::new(Duration::from_secs(11 * 60));
        let (_, model) = models(&endpoint)
            .into_iter()
            .find(|(model, _)| *model == name)
            .unwrap();
        let error = generate_image(model.as_ref(), ImageCallOptions::new("otter"))
            .await
            .unwrap_err();
        let AiMuxError::ApiCall(detail) = &error else {
            panic!("{name}: expected the exchange timeout, got {error:?}");
        };
        assert!(
            detail.message.contains("exchange response timeout"),
            "{name}: {}",
            detail.message
        );
        assert!(!detail.is_retryable, "{name}");
        assert_eq!(endpoint.calls(), 1, "{name}");
    }
}

/// A caller that allows the operation longer than the default guard gets
/// the longer bound on the exchange.
#[tokio::test(start_paused = true)]
async fn the_call_timeout_extends_the_exchange_guard() {
    let endpoint = SlowImageEndpoint::new(Duration::from_secs(15 * 60));
    let options = ImageCallOptions {
        timeout: Some(TimeoutConfiguration {
            total_ms: Some(20 * 60_000),
            ..TimeoutConfiguration::default()
        }),
        ..ImageCallOptions::new("otter")
    };
    generate_image(openai(&endpoint).as_ref(), options)
        .await
        .unwrap();
    assert_eq!(endpoint.calls(), 1);
}
