//! Port of `@ai-sdk/luma@3.0.54` `src/luma-provider.test.ts`.
//!
//! Upstream mocks the `LumaImageModel` class and checks the config it was
//! constructed with. aimux cannot spy on the constructor, so each case makes
//! one call through a scripted transport and reads the same settings off the
//! model and the requests: the provider name from `provider()`, the base URL
//! from the URLs, the headers and the fetch from what the transport saw.
//! The poll answers `failed`, so the call ends before the image download:
//! that download is validated and goes through the download guard's own
//! transport, never the provider's `fetch`, so it would leave the test.

use std::sync::Arc;

use serde_json::json;

use aimux_core::image_model::{ImageCallOptions, ImageModel};
use aimux_provider_utils::Resolvable;

use crate::luma::{LumaProviderSettings, create_luma};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

/// Answers the submit, and the poll with `failed`, under any base URL.
fn transport() -> Arc<RouteFetch> {
    RouteFetch::new(|seen: &Seen| {
        if seen.url.ends_with("/dream-machine/v1/generations/image") {
            Canned::json(&json!({ "id": "test-generation-id", "state": "queued" }))
        } else {
            Canned::json(&json!({ "id": "test-generation-id", "state": "failed" }))
        }
    })
}

mod create_luma {
    use super::*;

    mod image {
        use super::*;

        /// upstream: @ai-sdk/luma src/luma-provider.test.ts › createLuma › image › should construct an image model with default configuration
        #[tokio::test]
        async fn should_construct_an_image_model_with_default_configuration() {
            let fetch = transport();
            let provider = create_luma(LumaProviderSettings {
                api_key: Some(Resolvable::Value("test-api-key".to_string())),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();
            let model = provider.image("luma-v1");

            assert_eq!(model.provider(), "luma.image");
            assert_eq!(model.model_id(), "luma-v1");
            model
                .do_generate(&ImageCallOptions::new("A cute baby sea otter"))
                .await
                .unwrap_err();
            assert_eq!(
                fetch.seen()[0].url,
                "https://api.lumalabs.ai/dream-machine/v1/generations/image"
            );
        }

        /// upstream: @ai-sdk/luma src/luma-provider.test.ts › createLuma › image › should respect custom configuration options
        #[tokio::test]
        async fn should_respect_custom_configuration_options() {
            let fetch = transport();
            let provider = create_luma(LumaProviderSettings {
                api_key: Some(Resolvable::Value("custom-api-key".to_string())),
                base_url: Some("https://custom-api.lumalabs.ai".to_string()),
                headers: Some([("X-Custom-Header".to_string(), Some("value".to_string()))].into()),
                fetch: Some(fetch.transport()),
                ..Default::default()
            })
            .unwrap();
            let model = provider.image("luma-v1");

            assert_eq!(model.provider(), "luma.image");
            model
                .do_generate(&ImageCallOptions::new("A cute baby sea otter"))
                .await
                .unwrap_err();
            let seen = fetch.seen();
            assert_eq!(
                seen[0].url,
                "https://custom-api.lumalabs.ai/dream-machine/v1/generations/image"
            );
            assert_eq!(seen[0].headers["x-custom-header"], "value");
            assert_eq!(seen[0].headers["authorization"], "Bearer custom-api-key");
        }
    }
}
