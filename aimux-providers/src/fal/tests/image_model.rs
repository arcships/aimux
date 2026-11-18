//! Port of `@ai-sdk/fal@3.0.54` `src/fal-image-model.test.ts`.
//!
//! Upstream serves every URL from one `createTestServer` on
//! `https://api.example.com`. Here each test starts its own wiremock server
//! and uses it as the `base_url`: the generated images are downloaded through
//! the SSRF guard, which ignores an injected transport and only lets a
//! loopback URL through when it is same-origin with the `base_url`.

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResult,
};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size, Warning};
use aimux_provider_utils::Resolvable;

use crate::fal::{FalImageModel, FalProviderSettings, create_fal};

const PROMPT: &str = "A cute baby sea otter";
const MODEL_ID: &str = "fal-ai/qwen-image";
const IMAGE_BYTES: &[u8] = b"test-binary-content";

/// `createTestServer` without the model endpoints: `/image.png` answers the
/// image bytes.
async fn test_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/image.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(IMAGE_BYTES))
        .mount(&server)
        .await;
    server
}

/// `server.urls['…/<model_id>'].response = …`.
async fn respond(server: &MockServer, model_id: &str, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(format!("/{model_id}")))
        .respond_with(response)
        .mount(server)
        .await;
}

/// The URL of the image the server hands out (upstream's
/// `https://api.example.com/image.png`).
fn image_url(server: &MockServer) -> String {
    format!("{}/image.png", server.uri())
}

/// The default model endpoint response: one 1024×1024 PNG.
fn default_response(server: &MockServer) -> Value {
    json!({
        "images": [{
            "url": image_url(server),
            "width": 1024,
            "height": 1024,
            "content_type": "image/png",
        }],
    })
}

/// A server whose `model_id` endpoint answers the default response.
async fn default_server(model_id: &str) -> MockServer {
    let server = test_server().await;
    respond(
        &server,
        model_id,
        ResponseTemplate::new(200).set_body_json(default_response(&server)),
    )
    .await;
    server
}

/// `new FalImageModel(modelId, { provider: 'fal.image', baseURL, headers:
/// { 'api-key': 'test-key' } })`: the key goes out as fal's
/// `Authorization: Key …`.
fn image_model(server: &MockServer, model_id: &str) -> FalImageModel {
    create_fal(FalProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .image(model_id)
}

/// `createBasicModel()`.
fn create_basic_model(server: &MockServer) -> FalImageModel {
    image_model(server, MODEL_ID)
}

fn call_options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

fn fal_options(value: Value) -> SharedProviderOptions {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    [("fal".to_string(), map)].into_iter().collect()
}

/// `{ type: 'file', data: new Uint8Array(bytes), mediaType: 'image/png' }`.
fn png_file(bytes: &[u8]) -> ImageFile {
    ImageFile::File {
        media_type: "image/png".to_string(),
        data: ImageFileData::Binary(bytes.to_vec()),
    }
}

/// The PNG magic bytes upstream uses as the input image.
const PNG_MAGIC: [u8; 4] = [137, 80, 78, 71];

/// `await server.calls[0].requestBodyJson`: the body of the model call (the
/// downloads are recorded too).
async fn request_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .expect("the model endpoint was called");
    serde_json::from_slice(&post.body).unwrap()
}

fn binary_images(outputs: &ImageOutputs) -> &[Vec<u8>] {
    match outputs {
        ImageOutputs::Binary(images) => images,
        ImageOutputs::Base64(_) => panic!("expected binary images"),
    }
}

fn fal_metadata(result: &ImageResult) -> Value {
    serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap()["fal"].clone()
}

fn other_warning(warning: &Warning) -> &str {
    match warning {
        Warning::Other { message } => message,
        other => panic!("expected an `other` warning, got {other:?}"),
    }
}

/// `expect(actual).toMatchObject(expected)`.
fn assert_match_object(actual: &Value, expected: &Value) {
    fn matches(actual: &Value, expected: &Value) -> bool {
        match (actual, expected) {
            (Value::Object(actual), Value::Object(expected)) => expected
                .iter()
                .all(|(key, value)| actual.get(key).is_some_and(|a| matches(a, value))),
            (Value::Array(actual), Value::Array(expected)) => {
                actual.len() == expected.len()
                    && actual.iter().zip(expected).all(|(a, e)| matches(a, e))
            }
            _ => actual == expected,
        }
    }
    assert!(
        matches(actual, expected),
        "{actual:#} does not match {expected:#}"
    );
}

mod fal_image_model {
    use super::*;

    mod capabilities {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › capabilities › advertises file=$supportsFileInputs and mask=$supportsMaskInputs for $modelId
        #[test]
        fn advertises_file_and_mask_inputs_for_model_id() {
            for (model_id, supports_file_inputs, supports_mask_inputs) in [
                ("fal-ai/flux/dev/image-to-image", Some(true), Some(false)),
                ("fal-ai/flux-2/edit", Some(true), Some(false)),
                ("fal-ai/flux-pro/kontext", Some(true), Some(false)),
                ("fal-ai/flux-pro/kontext/max", Some(true), Some(false)),
                ("fal-ai/flux-general/inpainting", Some(true), Some(true)),
                ("fal-ai/recraft/v3/text-to-image", Some(false), Some(false)),
                ("custom/image-model", None, None),
                ("custom/image-to-image", None, None),
                ("custom/inpainting", None, None),
                ("custom/text-to-image", None, None),
                ("fal-ai/custom/image-to-image", None, None),
                ("fal-ai/custom/inpainting", None, None),
                ("fal-ai/custom/text-to-image", None, None),
            ] {
                let model = create_fal(FalProviderSettings {
                    base_url: Some("https://api.example.com".to_string()),
                    ..Default::default()
                })
                .unwrap()
                .image(model_id);

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

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should pass the correct parameters including size
        #[tokio::test]
        async fn should_pass_the_correct_parameters_including_size() {
            let server = default_server(MODEL_ID).await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.size = Some(Size::new(1024, 1024));
            options.seed = Some(123);
            options.provider_options = fal_options(json!({ "additional_param": "value" }));
            model.do_generate(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "seed": 123,
                    "image_size": { "width": 1024, "height": 1024 },
                    "num_images": 1,
                    "additional_param": "value",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should convert camelCase provider options to snake_case for API
        #[tokio::test]
        async fn should_convert_camel_case_provider_options_to_snake_case_for_api() {
            let server = default_server(MODEL_ID).await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({
                "imageUrl": "https://example.com/image.png",
                "guidanceScale": 7.5,
                "numInferenceSteps": 50,
                "enableSafetyChecker": false,
            }));
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "num_images": 1,
                    "image_url": "https://example.com/image.png",
                    "guidance_scale": 7.5,
                    "num_inference_steps": 50,
                    "enable_safety_checker": false,
                })
            );
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should accept deprecated snake_case provider options with warning
        #[tokio::test]
        async fn should_accept_deprecated_snake_case_provider_options_with_warning() {
            let server = default_server(MODEL_ID).await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.provider_options = fal_options(json!({
                "image_url": "https://example.com/image.png",
                "guidance_scale": 7.5,
                "num_inference_steps": 50,
            }));
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "num_images": 1,
                    "image_url": "https://example.com/image.png",
                    "guidance_scale": 7.5,
                    "num_inference_steps": 50,
                })
            );

            assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
            let message = other_warning(&result.warnings[0]);
            assert!(message.contains("deprecated snake_case"), "{message}");
            assert!(
                message.contains("'image_url' (use 'imageUrl')"),
                "{message}"
            );
            assert!(
                message.contains("'guidance_scale' (use 'guidanceScale')"),
                "{message}"
            );
            assert!(
                message.contains("'num_inference_steps' (use 'numInferenceSteps')"),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should convert aspect ratio to size
        #[tokio::test]
        async fn should_convert_aspect_ratio_to_size() {
            let server = default_server(MODEL_ID).await;
            let model = create_basic_model(&server);

            let mut options = call_options(PROMPT);
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            model.do_generate(&options).await.unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "prompt": PROMPT,
                    "image_size": "landscape_16_9",
                    "num_images": 1,
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let server = default_server(MODEL_ID).await;
            // The config headers are only `Custom-Provider-Header`: the
            // credential header is removed.
            let model_with_headers = create_fal(FalProviderSettings {
                api_key: Some(Resolvable::Value("test-key".to_string())),
                base_url: Some(server.uri()),
                headers: Some(
                    [
                        ("Authorization".to_string(), None),
                        (
                            "Custom-Provider-Header".to_string(),
                            Some("provider-header-value".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            })
            .unwrap()
            .image(MODEL_ID);

            let mut options = call_options(PROMPT);
            options.headers = Some(
                [(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )]
                .into_iter()
                .collect(),
            );
            model_with_headers.do_generate(&options).await.unwrap();

            let requests = server.received_requests().await.unwrap();
            let post = requests
                .iter()
                .find(|request| request.method.as_str() == "POST")
                .unwrap();
            // The test server reports neither the user agent nor the
            // headers the HTTP client adds on its own.
            let headers: serde_json::Map<String, Value> = post
                .headers
                .iter()
                .filter(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "user-agent" | "host" | "content-length" | "accept" | "accept-encoding"
                    )
                })
                .map(|(name, value)| (name.to_string(), json!(value.to_str().unwrap())))
                .collect();
            assert_eq!(
                Value::Object(headers),
                json!({
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › should handle API errors
        #[tokio::test]
        async fn should_handle_api_errors() {
            let server = test_server().await;
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(400).set_body_string(
                    json!({
                        "detail": [{
                            "loc": ["prompt"],
                            "msg": "Invalid prompt",
                            "type": "value_error",
                        }],
                    })
                    .to_string(),
                ),
            )
            .await;

            let error = create_basic_model(&server)
                .do_generate(&call_options(PROMPT))
                .await
                .unwrap_err();

            let AiMuxError::ApiCall(error) = error else {
                panic!("expected an API call error, got {error:?}")
            };
            assert_eq!(error.message, "prompt: Invalid prompt");
            assert_eq!(error.status_code, Some(400));
            assert_eq!(error.url, format!("{}/{MODEL_ID}", server.uri()));
        }

        mod response_metadata {
            use super::*;

            /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › response metadata › should include timestamp, headers and modelId in response
            #[tokio::test]
            async fn should_include_timestamp_headers_and_model_id_in_response() {
                let server = default_server(MODEL_ID).await;
                // No `currentDate` hook in Rust: the timestamp is the real
                // time.
                let before = aimux_core::Timestamp::now();
                let result = create_basic_model(&server)
                    .do_generate(&call_options(PROMPT))
                    .await
                    .unwrap();
                let after = aimux_core::Timestamp::now();

                let timestamp = result.response.timestamp.unwrap();
                assert!(before <= timestamp && timestamp <= after);
                assert_eq!(result.response.model_id.as_deref(), Some(MODEL_ID));
                assert!(result.response.headers.is_some());
            }
        }

        mod provider_meta_data {
            use super::*;

            /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › providerMetaData › for lora
            #[tokio::test]
            async fn for_lora() {
                let server = test_server().await;
                respond(
                    &server,
                    MODEL_ID,
                    ResponseTemplate::new(200).set_body_json(json!({
                        "images": [{
                            "url": image_url(&server),
                            "width": 1024,
                            "height": 1024,
                            "content_type": "image/png",
                            "file_data": "<image file_data>",
                            "file_size": 123,
                            "file_name": "<image file_name>",
                        }],
                        "prompt": "<prompt>",
                        "seed": 123,
                        "has_nsfw_concepts": [true],
                        "debug_latents": {
                            "url": "<debug_latents url>",
                            "content_type": "<debug_latents content_type>",
                            "file_name": "<debug_latents file_name>",
                            "file_data": "<debug_latents file_data>",
                            "file_size": 123,
                        },
                        "debug_per_pass_latents": {
                            "url": "<debug_per_pass_latents url>",
                            "content_type": "<debug_per_pass_latents content_type>",
                            "file_name": "<debug_per_pass_latents file_name>",
                            "file_data": "<debug_per_pass_latents file_data>",
                            "file_size": 456,
                        },
                    })),
                )
                .await;

                let result = create_basic_model(&server)
                    .do_generate(&call_options(PROMPT))
                    .await
                    .unwrap();

                assert_eq!(
                    serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
                    json!({
                        "fal": {
                            "images": [{
                                "width": 1024,
                                "height": 1024,
                                "contentType": "image/png",
                                "fileName": "<image file_name>",
                                "fileData": "<image file_data>",
                                "fileSize": 123,
                                "nsfw": true,
                            }],
                            "seed": 123,
                            "debug_latents": {
                                "url": "<debug_latents url>",
                                "content_type": "<debug_latents content_type>",
                                "file_name": "<debug_latents file_name>",
                                "file_data": "<debug_latents file_data>",
                                "file_size": 123,
                            },
                            "debug_per_pass_latents": {
                                "url": "<debug_per_pass_latents url>",
                                "content_type": "<debug_per_pass_latents content_type>",
                                "file_name": "<debug_per_pass_latents file_name>",
                                "file_data": "<debug_per_pass_latents file_data>",
                                "file_size": 456,
                            },
                        },
                    })
                );
            }

            /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › doGenerate › providerMetaData › for lcm
            #[tokio::test]
            async fn for_lcm() {
                let server = test_server().await;
                respond(
                    &server,
                    MODEL_ID,
                    ResponseTemplate::new(200).set_body_json(json!({
                        "images": [{
                            "url": image_url(&server),
                            "width": 1024,
                            "height": 1024,
                        }],
                        "seed": 123,
                        "num_inference_steps": 456,
                        "nsfw_content_detected": [false],
                    })),
                )
                .await;

                let result = create_basic_model(&server)
                    .do_generate(&call_options(PROMPT))
                    .await
                    .unwrap();

                assert_eq!(
                    serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
                    json!({
                        "fal": {
                            "images": [{
                                "width": 1024,
                                "height": 1024,
                                "nsfw": false,
                            }],
                            "seed": 123,
                            "num_inference_steps": 456,
                        },
                    })
                );
            }
        }
    }

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › constructor › should expose correct provider and model information
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let model = create_fal(FalProviderSettings {
                api_key: Some(Resolvable::Value("test-key".to_string())),
                base_url: Some("https://api.example.com".to_string()),
                ..Default::default()
            })
            .unwrap()
            .image(MODEL_ID);

            assert_eq!(model.provider(), "fal.image");
            assert_eq!(model.model_id(), MODEL_ID);
            assert_eq!(model.specification_version(), "v4");
            assert_eq!(model.max_images_per_call(), Some(1));
        }
    }

    mod image_editing {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send edit request with files as data URI
        #[tokio::test]
        async fn should_send_edit_request_with_files_as_data_uri() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Turn the cat into a dog");
            options.files = Some(vec![png_file(&PNG_MAGIC)]);
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "data:image/png;base64,iVBORw==",
                    "num_images": 1,
                    "prompt": "Turn the cat into a dog",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send edit request with files and mask
        #[tokio::test]
        async fn should_send_edit_request_with_files_and_mask() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Add a flamingo to the pool");
            options.files = Some(vec![png_file(&PNG_MAGIC)]);
            options.mask = Some(png_file(&[255, 255, 255, 0]));
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "data:image/png;base64,iVBORw==",
                    "mask_url": "data:image/png;base64,////AA==",
                    "num_images": 1,
                    "prompt": "Add a flamingo to the pool",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send edit request with URL-based file
        #[tokio::test]
        async fn should_send_edit_request_with_url_based_file() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit this image");
            options.files = Some(vec![ImageFile::Url {
                url: "https://example.com/input.png".to_string(),
            }]);
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "https://example.com/input.png",
                    "num_images": 1,
                    "prompt": "Edit this image",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send edit request with base64 string data
        #[tokio::test]
        async fn should_send_edit_request_with_base64_string_data() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit this image");
            options.files = Some(vec![ImageFile::File {
                media_type: "image/png".to_string(),
                data: ImageFileData::Base64("iVBORw0KGgoAAAANSUhEUgAAAAE=".to_string()),
            }]);
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAE=",
                    "num_images": 1,
                    "prompt": "Edit this image",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should warn when multiple files are provided
        #[tokio::test]
        async fn should_warn_when_multiple_files_are_provided() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit images");
            options.files = Some(vec![png_file(&PNG_MAGIC), png_file(&PNG_MAGIC)]);
            let result = create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
            let message = other_warning(&result.warnings[0]);
            assert!(
                message.contains("useMultipleImages is not enabled"),
                "{message}"
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send files as image_urls for fal-ai/flux-2/edit
        #[tokio::test]
        async fn should_send_files_as_image_urls_for_fal_ai_flux_2_edit() {
            let server = default_server("fal-ai/flux-2/edit").await;
            let model = image_model(&server, "fal-ai/flux-2/edit");

            let mut options = call_options("Edit these images");
            options.files = Some(vec![png_file(&PNG_MAGIC), png_file(&PNG_MAGIC)]);
            options.provider_options = fal_options(json!({ "useMultipleImages": true }));
            model.do_generate(&options).await.unwrap();

            let request_body = request_body(&server).await;
            assert_match_object(
                &request_body,
                &json!({
                    "image_urls": [
                        "data:image/png;base64,iVBORw==",
                        "data:image/png;base64,iVBORw==",
                    ],
                    "num_images": 1,
                    "prompt": "Edit these images",
                }),
            );
            assert_eq!(request_body.get("image_url"), None);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should not warn when multiple files provided with useMultipleImages
        #[tokio::test]
        async fn should_not_warn_when_multiple_files_provided_with_use_multiple_images() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit images");
            options.files = Some(vec![png_file(&PNG_MAGIC), png_file(&PNG_MAGIC)]);
            options.provider_options = fal_options(json!({ "useMultipleImages": true }));
            let result = create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should send single image as image_urls array when useMultipleImages is true
        #[tokio::test]
        async fn should_send_single_image_as_image_urls_array_when_use_multiple_images_is_true() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit this image");
            options.files = Some(vec![png_file(&PNG_MAGIC)]);
            options.provider_options = fal_options(json!({ "useMultipleImages": true }));
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            let request_body = request_body(&server).await;
            assert_match_object(
                &request_body,
                &json!({
                    "image_urls": ["data:image/png;base64,iVBORw=="],
                    "num_images": 1,
                    "prompt": "Edit this image",
                }),
            );
            assert_eq!(request_body.get("image_url"), None);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should allow imageUrl via provider options
        #[tokio::test]
        async fn should_allow_image_url_via_provider_options() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Edit via provider options");
            options.provider_options =
                fal_options(json!({ "imageUrl": "https://example.com/provider-image.png" }));
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "https://example.com/provider-image.png",
                    "num_images": 1,
                    "prompt": "Edit via provider options",
                })
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › Image Editing › should allow maskUrl via provider options
        #[tokio::test]
        async fn should_allow_mask_url_via_provider_options() {
            let server = default_server(MODEL_ID).await;

            let mut options = call_options("Inpaint this");
            options.provider_options = fal_options(json!({
                "imageUrl": "https://example.com/image.png",
                "maskUrl": "https://example.com/mask.png",
            }));
            create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(
                request_body(&server).await,
                json!({
                    "image_url": "https://example.com/image.png",
                    "mask_url": "https://example.com/mask.png",
                    "num_images": 1,
                    "prompt": "Inpaint this",
                })
            );
        }
    }

    mod response_schema_validation {
        use super::*;

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › response schema validation › should parse single image response
        #[tokio::test]
        async fn should_parse_single_image_response() {
            let server = test_server().await;
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({
                    "image": {
                        "url": image_url(&server),
                        "width": 1024,
                        "height": 1024,
                        "content_type": "image/png",
                    },
                })),
            )
            .await;

            let result = create_basic_model(&server)
                .do_generate(&call_options(PROMPT))
                .await
                .unwrap();

            assert_eq!(binary_images(&result.images), [IMAGE_BYTES]);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › response schema validation › should parse multiple images response
        #[tokio::test]
        async fn should_parse_multiple_images_response() {
            let server = test_server().await;
            let image = json!({
                "url": image_url(&server),
                "width": 1024,
                "height": 1024,
                "content_type": "image/png",
            });
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({ "images": [image, image] })),
            )
            .await;

            let mut options = call_options(PROMPT);
            options.n = 2;
            let result = create_basic_model(&server)
                .do_generate(&options)
                .await
                .unwrap();

            assert_eq!(binary_images(&result.images), [IMAGE_BYTES, IMAGE_BYTES]);
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › response schema validation › should handle null file_name and file_size values
        #[tokio::test]
        async fn should_handle_null_file_name_and_file_size_values() {
            let server = test_server().await;
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({
                    "images": [{
                        "url": image_url(&server),
                        "content_type": "image/png",
                        "file_name": null,
                        "file_size": null,
                        "width": 944,
                        "height": 1104,
                    }],
                    "timings": { "inference": 5.875932216644287 },
                    "seed": 328395684,
                    "has_nsfw_concepts": [false],
                    "prompt": "A female model holding this book, keeping the book unchanged.",
                })),
            )
            .await;

            let result = create_basic_model(&server)
                .do_generate(&call_options(PROMPT))
                .await
                .unwrap();

            assert_eq!(binary_images(&result.images), [IMAGE_BYTES]);
            assert_match_object(
                &fal_metadata(&result),
                &json!({
                    "images": [{
                        "width": 944,
                        "height": 1104,
                        "contentType": "image/png",
                        "fileName": null,
                        "fileSize": null,
                        "nsfw": false,
                    }],
                    "timings": { "inference": 5.875932216644287 },
                    "seed": 328395684,
                }),
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › response schema validation › should handle empty timings object
        #[tokio::test]
        async fn should_handle_empty_timings_object() {
            let server = test_server().await;
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({
                    "images": [{
                        "url": image_url(&server),
                        "content_type": "image/png",
                        "file_name": null,
                        "file_size": null,
                        "width": 880,
                        "height": 1184,
                    }],
                    "timings": {},
                    "seed": 235205040,
                    "has_nsfw_concepts": [false],
                    "prompt": "Change the plates to colorful ones",
                })),
            )
            .await;

            let result = create_basic_model(&server)
                .do_generate(&call_options(PROMPT))
                .await
                .unwrap();

            assert_eq!(binary_images(&result.images), [IMAGE_BYTES]);
            assert_match_object(
                &fal_metadata(&result),
                &json!({
                    "images": [{
                        "width": 880,
                        "height": 1184,
                        "contentType": "image/png",
                        "fileName": null,
                        "fileSize": null,
                        "nsfw": false,
                    }],
                    "timings": {},
                    "seed": 235205040,
                }),
            );
        }

        /// upstream: @ai-sdk/fal src/fal-image-model.test.ts › FalImageModel › response schema validation › should handle null width and height values with images array only
        #[tokio::test]
        async fn should_handle_null_width_and_height_values_with_images_array_only() {
            let server = test_server().await;
            respond(
                &server,
                MODEL_ID,
                ResponseTemplate::new(200).set_body_json(json!({
                    "images": [{
                        "url": image_url(&server),
                        "content_type": "image/png",
                        "file_name": "output.png",
                        "file_size": 663399,
                        "width": null,
                        "height": null,
                    }],
                    "description": "here is an image with null width and height",
                })),
            )
            .await;

            let result = create_basic_model(&server)
                .do_generate(&call_options(PROMPT))
                .await
                .unwrap();

            assert_eq!(binary_images(&result.images), [IMAGE_BYTES]);
            assert_match_object(
                &fal_metadata(&result),
                &json!({
                    "images": [{
                        "width": null,
                        "height": null,
                        "contentType": "image/png",
                        "fileName": "output.png",
                        "fileSize": 663399,
                    }],
                    "description": "here is an image with null width and height",
                }),
            );
        }
    }
}
