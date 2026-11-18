//! Port of `@ai-sdk/fal@3.0.54` `src/fal-provider.test.ts`.
//!
//! Upstream mocks the model constructors and asserts what `createFal` passes
//! to them. Rust has no constructor mock, so the same configuration is
//! observed on what a model does: its `provider()` string, and the request a
//! [`RouteFetch`] transport sees.

use std::sync::Arc;

use serde_json::json;

use crate::fal::{FalProvider, FalProviderSettings, create_fal};
use crate::test_support::mock_fetch::{Canned, RouteFetch};
use aimux_core::image_model::{ImageCallOptions, ImageModel, generate_image};
use aimux_core::video_model::{VideoCallOptions, VideoModel};

/// The key every test passes explicitly, so `FAL_KEY` in the environment
/// never decides the outcome.
const API_KEY: &str = "test-api-key";

/// The body of a successful image submission: no images means no download.
fn image_response() -> Canned {
    Canned::json(&json!({ "images": [] }))
}

/// The body of a successful video submission.
fn video_response() -> Canned {
    Canned::json(&json!({
        "request_id": "test-request-id",
        "response_url": "https://queue.fal.run/fal-ai/test/requests/test-request-id",
    }))
}

/// A transport that records every request and answers it with `response`.
fn route(response: Canned) -> Arc<RouteFetch> {
    RouteFetch::new(move |_| response.clone())
}

/// A provider from `settings`, with [`API_KEY`] filled in when the test sets
/// none.
fn provider(settings: FalProviderSettings) -> FalProvider {
    create_fal(FalProviderSettings {
        api_key: settings
            .api_key
            .clone()
            .or_else(|| Some(API_KEY.to_string().into())),
        ..settings
    })
    .unwrap()
}

mod image {
    use super::*;

    /// upstream: @ai-sdk/fal src/fal-provider.test.ts › createFal › image › should construct an image model with default configuration
    #[tokio::test]
    async fn should_construct_an_image_model_with_default_configuration() {
        let mock = route(image_response());
        let fal = provider(FalProviderSettings {
            fetch: Some(mock.transport()),
            ..FalProviderSettings::default()
        });
        let model_id = "fal-ai/flux/dev";

        let model = fal.image(model_id);

        assert_eq!(ImageModel::provider(&model), "fal.image");
        generate_image(&model, ImageCallOptions::new("A cute baby sea otter"))
            .await
            .unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, format!("https://fal.run/{model_id}"));
        assert_eq!(
            seen[0].headers.get("authorization").map(String::as_str),
            Some("Key test-api-key")
        );
    }

    /// upstream: @ai-sdk/fal src/fal-provider.test.ts › createFal › image › should respect custom configuration options
    #[tokio::test]
    async fn should_respect_custom_configuration_options() {
        let mock = route(image_response());
        let fal = provider(FalProviderSettings {
            api_key: Some("custom-api-key".to_string().into()),
            base_url: Some("https://custom.fal.run".into()),
            headers: Some([("X-Custom-Header".to_string(), Some("value".to_string()))].into()),
            fetch: Some(mock.transport()),
            ..FalProviderSettings::default()
        });
        let model_id = "fal-ai/flux/dev";

        let model = fal.image(model_id);
        assert_eq!(ImageModel::provider(&model), "fal.image");

        generate_image(&model, ImageCallOptions::new("A cute baby sea otter"))
            .await
            .unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, format!("https://custom.fal.run/{model_id}"));
        assert_eq!(
            seen[0].headers.get("x-custom-header").map(String::as_str),
            Some("value")
        );
        assert_eq!(
            seen[0].headers.get("authorization").map(String::as_str),
            Some("Key custom-api-key")
        );
    }
}

mod video {
    use super::*;

    /// upstream: @ai-sdk/fal src/fal-provider.test.ts › createFal › video › should construct a video model with default configuration
    #[tokio::test]
    async fn should_construct_a_video_model_with_default_configuration() {
        let mock = route(video_response());
        let fal = provider(FalProviderSettings {
            fetch: Some(mock.transport()),
            ..FalProviderSettings::default()
        });
        let model_id = "luma-dream-machine";

        let model = fal.video(model_id);

        assert_eq!(VideoModel::provider(&model), "fal.video");
        model
            .do_start(&VideoCallOptions::new("A cute baby sea otter"))
            .await
            .unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].url,
            format!("https://queue.fal.run/fal-ai/{model_id}")
        );
    }

    /// upstream: @ai-sdk/fal src/fal-provider.test.ts › createFal › video › should respect custom configuration options
    #[tokio::test]
    async fn should_respect_custom_configuration_options() {
        let mock = route(video_response());
        let fal = provider(FalProviderSettings {
            api_key: Some("custom-api-key".to_string().into()),
            headers: Some([("X-Custom-Header".to_string(), Some("value".to_string()))].into()),
            fetch: Some(mock.transport()),
            ..FalProviderSettings::default()
        });
        let model_id = "luma-ray-2";

        let model = fal.video(model_id);
        assert_eq!(VideoModel::provider(&model), "fal.video");

        model
            .do_start(&VideoCallOptions::new("A cute baby sea otter"))
            .await
            .unwrap();

        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].url,
            format!("https://queue.fal.run/fal-ai/{model_id}")
        );
        assert_eq!(
            seen[0].headers.get("x-custom-header").map(String::as_str),
            Some("value")
        );
        assert_eq!(
            seen[0].headers.get("authorization").map(String::as_str),
            Some("Key custom-api-key")
        );
    }

    /// upstream: @ai-sdk/fal src/fal-provider.test.ts › createFal › video › should support various video model IDs
    #[tokio::test]
    async fn should_support_various_video_model_ids() {
        let model_ids = [
            "luma-dream-machine",
            "luma-ray-2",
            "luma-ray-2-flash",
            "minimax-video",
            "minimax-video-01",
            "hunyuan-video",
            "custom-model",
        ];

        let fal = provider(FalProviderSettings::default());

        for model_id in model_ids {
            let model = fal.video(model_id);

            assert_eq!(VideoModel::provider(&model), "fal.video");
            assert_eq!(VideoModel::model_id(&model), model_id);
        }
    }
}
