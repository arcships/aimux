//! Ports of `@ai-sdk/amazon-bedrock` 5.0.100
//! `src/amazon-bedrock-image-model.test.ts`.
//!
//! Upstream builds `AmazonBedrockImageModel` directly with a base URL, plain
//! config headers and `fetch: injectFetchHeaders(..)`, so no request is
//! signed. The port does the same: `BedrockImageModel::from_config` with a
//! fixed endpoint, the config headers and the
//! [`inject_fetch_headers`](super::support::inject_fetch_headers) transport over the
//! mock transport (which stands in for the test server).
//!
//! One case stays in `aimux-providers/tests/amazon_bedrock_image_test.rs`,
//! whose test already covers it: "should throw error when request is
//! moderated".

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageResult,
};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt};

use crate::bedrock::{AmazonBedrockProviderSettings, BedrockImageModel, create_amazon_bedrock};
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, MockFetch};

use super::support::inject_fetch_headers;

const PROMPT: &str = "A cute baby sea otter";

const BASE_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";

const INVOKE_URL: &str =
    "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-canvas-v1%3A0/invoke";

/// A fixed base URL and fixed headers: the `{ baseUrl, headers }` config of
/// the upstream model.
struct FixedEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait]
impl ResolveEndpoint for FixedEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

/// `new AmazonBedrockImageModel(model_id, { baseUrl, headers, fetch })`.
fn image_model(
    model_id: &str,
    base_url: &str,
    headers: &[(&str, &str)],
    fetch: Option<FetchFunction>,
) -> BedrockImageModel {
    BedrockImageModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "amazon-bedrock".to_string(),
            FixedEndpoint {
                base_url: base_url.to_string(),
                headers: headers
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
                    .collect(),
            },
            fetch,
        ),
    )
}

/// `fakeFetchWithAuth` over `mock`.
fn fake_fetch_with_auth(mock: &Arc<MockFetch>) -> FetchFunction {
    inject_fetch_headers(&[("x-amz-auth", "test-auth")], mock.transport())
}

/// A binary JSON response, with the `content-length` the test server sends.
fn json_response(body: &Value) -> Canned {
    let mut canned = Canned::json(body);
    let length = canned.body.len().to_string();
    canned.headers.push(("content-length".to_string(), length));
    canned
}

/// Options with every field the upstream calls leave `undefined` unset.
fn options(prompt: Option<&str>) -> ImageCallOptions {
    ImageCallOptions {
        prompt: prompt.map(str::to_string),
        ..ImageCallOptions::new("")
    }
}

fn provider_options(namespace: &str, options: Value) -> SharedProviderOptions {
    aimux_core::shared::provider_namespace(namespace, options).expect("options are an object")
}

fn png(bytes: &[u8], media_type: &str) -> ImageFile {
    ImageFile::File {
        media_type: media_type.to_string(),
        data: ImageFileData::Binary(bytes.to_vec()),
    }
}

/// PNG magic bytes.
const IMAGE_DATA: [u8; 4] = [137, 80, 78, 71];
const MASK_DATA: [u8; 4] = [255, 255, 255, 0];

fn images_of(result: &ImageResult) -> Value {
    serde_json::to_value(&result.images).expect("images serialize")
}

mod capabilities {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › capabilities › advertises file=$supportsFileInputs and mask=$supportsMaskInputs for $modelId
    #[test]
    fn advertises_file_and_mask_inputs_for_model_id() {
        for (model_id, supports_file_inputs, supports_mask_inputs) in [
            ("amazon.nova-canvas-v1:0", Some(true), Some(true)),
            ("custom-image-model", None, None),
        ] {
            let model = image_model(model_id, "https://api.example.com", &[], None);

            assert_eq!(
                model.supports_file_inputs(),
                supports_file_inputs,
                "{model_id}"
            );
            assert_eq!(
                model.supports_mask_inputs(),
                supports_mask_inputs,
                "{model_id}"
            );
        }
    }
}

mod do_generate {
    use super::*;

    /// The upstream `mockConfigHeaders`.
    const MOCK_CONFIG_HEADERS: &[(&str, &str)] = &[
        ("config-header", "config-value"),
        ("shared-header", "config-shared"),
    ];

    fn default_response() -> Canned {
        json_response(&json!({ "images": ["base64-image-1", "base64-image-2"] }))
    }

    /// The upstream `model` over a mock that answers `responses`.
    fn model_and_mock(responses: Vec<Canned>) -> (BedrockImageModel, Arc<MockFetch>) {
        let mock = MockFetch::new(responses);
        let model = image_model(
            "amazon.nova-canvas-v1:0",
            BASE_URL,
            MOCK_CONFIG_HEADERS,
            Some(fake_fetch_with_auth(&mock)),
        );
        (model, mock)
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should pass the model and the settings
    #[tokio::test]
    async fn should_pass_the_model_and_the_settings() {
        let (model, mock) = model_and_mock(vec![default_response()]);

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                seed: Some(1234),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "negativeText": "bad", "quality": "premium", "cfgScale": 1.2 }),
                ),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        let seen = mock.seen();
        assert_eq!(seen[0].url, INVOKE_URL);
        assert_eq!(
            seen[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "cfgScale": 1.2,
                    "height": 1024,
                    "numberOfImages": 1,
                    "quality": "premium",
                    "seed": 1234,
                    "width": 1024,
                },
                "taskType": "TEXT_IMAGE",
                "textToImageParams": {
                    "negativeText": "bad",
                    "text": "A cute baby sea otter",
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should accept the new `amazonBedrock` providerOptions key
    #[tokio::test]
    async fn should_accept_the_new_amazon_bedrock_provider_options_key() {
        let (model, mock) = model_and_mock(vec![default_response()]);

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                seed: Some(1234),
                provider_options: provider_options(
                    "amazonBedrock",
                    json!({
                        "negativeText": "bad",
                        "quality": "premium",
                        "cfgScale": 1.2,
                        "style": "PHOTOREALISM",
                    }),
                ),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "cfgScale": 1.2,
                    "height": 1024,
                    "numberOfImages": 1,
                    "quality": "premium",
                    "seed": 1234,
                    "width": 1024,
                },
                "taskType": "TEXT_IMAGE",
                "textToImageParams": {
                    "negativeText": "bad",
                    "style": "PHOTOREALISM",
                    "text": "A cute baby sea otter",
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should prefer `amazonBedrock` providerOptions over the legacy `bedrock` key
    #[tokio::test]
    async fn should_prefer_amazon_bedrock_provider_options_over_the_legacy_bedrock_key() {
        let (model, mock) = model_and_mock(vec![default_response()]);
        let mut provider_options = provider_options(
            "bedrock",
            json!({
                "negativeText": "legacy",
                "quality": "standard",
                "cfgScale": 1,
                "style": "DESIGN_SKETCH",
            }),
        );
        provider_options.extend(super::provider_options(
            "amazonBedrock",
            json!({
                "negativeText": "preferred",
                "quality": "premium",
                "cfgScale": 2,
                "style": "PHOTOREALISM",
            }),
        ));

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                provider_options,
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "cfgScale": 2,
                    "numberOfImages": 1,
                    "quality": "premium",
                },
                "taskType": "TEXT_IMAGE",
                "textToImageParams": {
                    "negativeText": "preferred",
                    "style": "PHOTOREALISM",
                    "text": "A cute baby sea otter",
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should reject invalid provider options
    #[tokio::test]
    async fn should_reject_invalid_provider_options() {
        let (model, _mock) = model_and_mock(vec![default_response()]);

        let error = model
            .do_generate(&ImageCallOptions {
                n: 1,
                provider_options: provider_options("amazonBedrock", json!({ "quality": "ultra" })),
                ..options(Some(PROMPT))
            })
            .await
            .expect_err("the options are rejected");

        assert!(
            error
                .to_string()
                .contains("invalid amazonBedrock provider options"),
            "unexpected error: {error}"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should properly combine headers from all sources
    #[tokio::test]
    async fn should_properly_combine_headers_from_all_sources() {
        let mock = MockFetch::new(vec![default_response()]);
        let model_with_headers = image_model(
            "amazon.nova-canvas-v1:0",
            BASE_URL,
            &[
                ("model-header", "model-value"),
                ("shared-header", "model-shared"),
            ],
            Some(inject_fetch_headers(
                &[
                    ("signed-header", "signed-value"),
                    ("authorization", "AWS4-HMAC-SHA256..."),
                ],
                mock.transport(),
            )),
        );

        model_with_headers
            .do_generate(&ImageCallOptions {
                n: 1,
                headers: Some(HashMap::from([
                    ("options-header".to_string(), "options-value".to_string()),
                    ("shared-header".to_string(), "options-shared".to_string()),
                ])),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        let request_headers = &mock.seen()[0].headers;
        assert_eq!(request_headers["options-header"], "options-value");
        assert_eq!(request_headers["model-header"], "model-value");
        assert_eq!(request_headers["signed-header"], "signed-value");
        assert_eq!(request_headers["authorization"], "AWS4-HMAC-SHA256...");
        assert_eq!(request_headers["shared-header"], "options-shared");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should respect maxImagesPerCall setting
    #[test]
    fn should_respect_max_images_per_call_setting() {
        let provider = create_amazon_bedrock(AmazonBedrockProviderSettings::default())
            .expect("default settings are valid");

        let default_model = provider.image("amazon.nova-canvas-v1:0");
        assert_eq!(default_model.max_images_per_call(), Some(5));

        let unknown_model = provider.image("unknown-model");
        assert_eq!(unknown_model.max_images_per_call(), Some(1));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should return warnings for unsupported settings
    #[tokio::test]
    async fn should_return_warnings_for_unsupported_settings() {
        let (model, _mock) = model_and_mock(vec![default_response()]);

        let result = model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                aspect_ratio: Some(AspectRatio::new(1, 1)),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            serde_json::to_value(&result.warnings).expect("warnings serialize"),
            json!([
                {
                    "details": "This model does not support aspect ratio. Use `size` instead.",
                    "feature": "aspectRatio",
                    "type": "unsupported",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should extract the generated images
    #[tokio::test]
    async fn should_extract_the_generated_images() {
        let (model, _mock) = model_and_mock(vec![default_response()]);

        let result = model
            .do_generate(&ImageCallOptions {
                n: 1,
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            images_of(&result),
            json!(["base64-image-1", "base64-image-2"])
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
    ///
    /// Upstream fixes the timestamp through `_internal.currentDate`.
    /// `BedrockImageModel` stamps `Timestamp::now()` and takes no date
    /// provider, so the timestamp is asserted to be the call time instead of
    /// `2024-03-15T12:00:00.000Z`.
    #[tokio::test]
    async fn should_include_response_data_with_timestamp_model_id_and_headers() {
        let mock = MockFetch::new(vec![default_response()]);
        let custom_model = image_model(
            "amazon.nova-canvas-v1:0",
            BASE_URL,
            &[],
            Some(mock.transport()),
        );

        let before = aimux_core::Timestamp::now();
        let result = custom_model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");
        let after = aimux_core::Timestamp::now();

        assert_eq!(
            result.response.headers,
            Some(HashMap::from([
                ("content-length".to_string(), "46".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ]))
        );
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("amazon.nova-canvas-v1:0")
        );
        let timestamp = result.response.timestamp.expect("timestamp").unix_millis();
        assert!(before.unix_millis() <= timestamp && timestamp <= after.unix_millis());
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should use real date when no custom date provider is specified
    #[tokio::test]
    async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
        let (model, _mock) = model_and_mock(vec![default_response()]);

        let before_date = aimux_core::Timestamp::now();
        let result = model
            .do_generate(&ImageCallOptions {
                n: 1,
                seed: Some(1234),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");
        let after_date = aimux_core::Timestamp::now();

        let timestamp = result.response.timestamp.expect("timestamp").unix_millis();
        assert!(timestamp >= before_date.unix_millis());
        assert!(timestamp <= after_date.unix_millis());
        assert_eq!(
            result.response.model_id.as_deref(),
            Some("amazon.nova-canvas-v1:0")
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should pass the style parameter when provided
    #[tokio::test]
    async fn should_pass_the_style_parameter_when_provided() {
        let (model, mock) = model_and_mock(vec![default_response()]);

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                seed: Some(1234),
                provider_options: provider_options(
                    "bedrock",
                    json!({
                        "negativeText": "bad",
                        "quality": "premium",
                        "cfgScale": 1.2,
                        "style": "PHOTOREALISM",
                    }),
                ),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "cfgScale": 1.2,
                    "height": 1024,
                    "numberOfImages": 1,
                    "quality": "premium",
                    "seed": 1234,
                    "width": 1024,
                },
                "taskType": "TEXT_IMAGE",
                "textToImageParams": {
                    "negativeText": "bad",
                    "style": "PHOTOREALISM",
                    "text": "A cute baby sea otter",
                },
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should not include style parameter when not provided
    #[tokio::test]
    async fn should_not_include_style_parameter_when_not_provided() {
        let (model, mock) = model_and_mock(vec![default_response()]);

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                size: Some(Size::new(1024, 1024)),
                seed: Some(1234),
                provider_options: provider_options("bedrock", json!({ "quality": "standard" })),
                ..options(Some(PROMPT))
            })
            .await
            .expect("generation succeeds");

        let request_body = mock.seen()[0].json_body();
        assert!(
            request_body["textToImageParams"].get("style").is_none(),
            "{request_body}"
        );
    }

    // "should throw error when request is moderated": the marked test in
    // aimux-providers/tests/amazon_bedrock_image_test.rs.

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › doGenerate › should throw error when no images are returned
    #[tokio::test]
    async fn should_throw_error_when_no_images_are_returned() {
        let (model, _mock) = model_and_mock(vec![json_response(&json!({ "images": [] }))]);

        let error = model
            .do_generate(&ImageCallOptions {
                n: 1,
                ..options(Some("Generate an image"))
            })
            .await
            .expect_err("generation fails");

        assert!(
            error
                .to_string()
                .contains("Amazon Bedrock returned no images"),
            "unexpected error: {error}"
        );
    }
}

mod image_editing {
    use super::*;

    fn edited_response() -> Canned {
        json_response(&json!({ "images": ["edited-image-base64"] }))
    }

    /// The upstream `model` of this describe over a mock that answers once.
    fn model_and_mock() -> (BedrockImageModel, Arc<MockFetch>) {
        let mock = MockFetch::new(vec![edited_response()]);
        let model = image_model(
            "amazon.nova-canvas-v1:0",
            BASE_URL,
            &[],
            Some(fake_fetch_with_auth(&mock)),
        );
        (model, mock)
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send inpainting request with files and maskPrompt
    #[tokio::test]
    async fn should_send_inpainting_request_with_files_and_mask_prompt() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                seed: Some(42),
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "maskPrompt": "cat", "quality": "standard", "cfgScale": 7.0 }),
                ),
                ..options(Some("a cute corgi dog"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "cfgScale": 7.0,
                    "numberOfImages": 1,
                    "quality": "standard",
                    "seed": 42,
                },
                "inPaintingParams": {
                    "image": "iVBORw==",
                    "maskPrompt": "cat",
                    "text": "a cute corgi dog",
                },
                "taskType": "INPAINTING",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send inpainting request with files and mask image
    #[tokio::test]
    async fn should_send_inpainting_request_with_files_and_mask_image() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                mask: Some(png(&MASK_DATA, "image/png")),
                provider_options: provider_options("bedrock", json!({ "quality": "standard" })),
                ..options(Some(
                    "A sunlit indoor lounge area with a pool containing a flamingo",
                ))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "numberOfImages": 1,
                    "quality": "standard",
                },
                "inPaintingParams": {
                    "image": "iVBORw==",
                    "maskImage": "////AA==",
                    "text": "A sunlit indoor lounge area with a pool containing a flamingo",
                },
                "taskType": "INPAINTING",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send inpainting request with base64 string data
    #[tokio::test]
    async fn should_send_inpainting_request_with_base64_string_data() {
        let (model, mock) = model_and_mock();
        let base64_image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk";

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![ImageFile::File {
                    media_type: "image/png".to_string(),
                    data: ImageFileData::Base64(base64_image.to_string()),
                }]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "maskPrompt": "background" }),
                ),
                ..options(Some("Edit this image"))
            })
            .await
            .expect("generation succeeds");

        let request_body = mock.seen()[0].json_body();
        assert_eq!(request_body["taskType"], "INPAINTING");
        assert_eq!(request_body["inPaintingParams"]["image"], base64_image);
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should include negativeText in inpainting params
    #[tokio::test]
    async fn should_include_negative_text_in_inpainting_params() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "maskPrompt": "sky", "negativeText": "clouds, rain" }),
                ),
                ..options(Some("a beautiful garden"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "numberOfImages": 1,
                },
                "inPaintingParams": {
                    "image": "iVBORw==",
                    "maskPrompt": "sky",
                    "negativeText": "clouds, rain",
                    "text": "a beautiful garden",
                },
                "taskType": "INPAINTING",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should extract edited images from response
    #[tokio::test]
    async fn should_extract_edited_images_from_response() {
        let (model, _mock) = model_and_mock();

        let result = model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options("bedrock", json!({ "maskPrompt": "object" })),
                ..options(Some("Edit this image"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(images_of(&result), json!(["edited-image-base64"]));
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should throw error for URL-based images
    #[tokio::test]
    async fn should_throw_error_for_url_based_images() {
        let (model, _mock) = model_and_mock();

        let error = model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![ImageFile::Url {
                    url: "https://example.com/image.png".to_string(),
                }]),
                ..options(Some("Edit this image"))
            })
            .await
            .expect_err("generation fails");

        assert!(
            error
                .to_string()
                .contains("URL-based images are not supported for Amazon Bedrock image editing."),
            "unexpected error: {error}"
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send outpainting request with taskType OUTPAINTING
    #[tokio::test]
    async fn should_send_outpainting_request_with_task_type_outpainting() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                mask: Some(png(&MASK_DATA, "image/png")),
                provider_options: provider_options(
                    "bedrock",
                    json!({
                        "taskType": "OUTPAINTING",
                        "outPaintingMode": "DEFAULT",
                        "negativeText": "bad quality",
                    }),
                ),
                ..options(Some("Extend the background with a beautiful sunset"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "numberOfImages": 1,
                },
                "outPaintingParams": {
                    "image": "iVBORw==",
                    "maskImage": "////AA==",
                    "negativeText": "bad quality",
                    "outPaintingMode": "DEFAULT",
                    "text": "Extend the background with a beautiful sunset",
                },
                "taskType": "OUTPAINTING",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send outpainting request with maskPrompt
    #[tokio::test]
    async fn should_send_outpainting_request_with_mask_prompt() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "taskType": "OUTPAINTING", "maskPrompt": "background" }),
                ),
                ..options(Some("Replace the background with mountains"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "numberOfImages": 1,
                },
                "outPaintingParams": {
                    "image": "iVBORw==",
                    "maskPrompt": "background",
                    "text": "Replace the background with mountains",
                },
                "taskType": "OUTPAINTING",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send background removal request
    #[tokio::test]
    async fn should_send_background_removal_request() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "taskType": "BACKGROUND_REMOVAL" }),
                ),
                ..options(None)
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "backgroundRemovalParams": {
                    "image": "iVBORw==",
                },
                "taskType": "BACKGROUND_REMOVAL",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send image variation request with single image
    #[tokio::test]
    async fn should_send_image_variation_request_with_single_image() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 3,
                size: Some(Size::new(512, 512)),
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options(
                    "bedrock",
                    json!({
                        "taskType": "IMAGE_VARIATION",
                        "similarityStrength": 0.7,
                        "negativeText": "bad quality, low resolution",
                    }),
                ),
                ..options(Some("Create a variation in anime style"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "height": 512,
                    "numberOfImages": 3,
                    "width": 512,
                },
                "imageVariationParams": {
                    "images": ["iVBORw=="],
                    "negativeText": "bad quality, low resolution",
                    "similarityStrength": 0.7,
                    "text": "Create a variation in anime style",
                },
                "taskType": "IMAGE_VARIATION",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should send image variation request with multiple images
    #[tokio::test]
    async fn should_send_image_variation_request_with_multiple_images() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![
                    png(&IMAGE_DATA, "image/png"),
                    png(&[255, 216, 255, 224], "image/jpeg"),
                ]),
                provider_options: provider_options(
                    "bedrock",
                    json!({ "taskType": "IMAGE_VARIATION" }),
                ),
                ..options(Some("Combine these images into one cohesive scene"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "imageGenerationConfig": {
                    "numberOfImages": 1,
                },
                "imageVariationParams": {
                    "images": ["iVBORw==", "/9j/4A=="],
                    "text": "Combine these images into one cohesive scene",
                },
                "taskType": "IMAGE_VARIATION",
            })
        );
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should default to IMAGE_VARIATION when files provided without mask or maskPrompt
    #[tokio::test]
    async fn should_default_to_image_variation_when_files_provided_without_mask_or_mask_prompt() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                ..options(Some("Create variations"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(mock.seen()[0].json_body()["taskType"], "IMAGE_VARIATION");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should default to INPAINTING when files provided with mask
    #[tokio::test]
    async fn should_default_to_inpainting_when_files_provided_with_mask() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                mask: Some(png(&MASK_DATA, "image/png")),
                ..options(Some("Edit masked area"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(mock.seen()[0].json_body()["taskType"], "INPAINTING");
    }

    /// upstream: @ai-sdk/amazon-bedrock src/amazon-bedrock-image-model.test.ts › Image Editing › should default to INPAINTING when files provided with maskPrompt
    #[tokio::test]
    async fn should_default_to_inpainting_when_files_provided_with_mask_prompt() {
        let (model, mock) = model_and_mock();

        model
            .do_generate(&ImageCallOptions {
                n: 1,
                files: Some(vec![png(&IMAGE_DATA, "image/png")]),
                provider_options: provider_options("bedrock", json!({ "maskPrompt": "cat" })),
                ..options(Some("Edit the cat"))
            })
            .await
            .expect("generation succeeds");

        assert_eq!(mock.seen()[0].json_body()["taskType"], "INPAINTING");
    }
}
