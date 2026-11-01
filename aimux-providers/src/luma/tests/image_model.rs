//! Port of `@ai-sdk/luma@3.0.54` `src/luma-image-model.test.ts`.
//!
//! Harness: upstream builds `new LumaImageModel('test-model', { provider:
//! 'luma', baseURL: 'https://api.example.com', headers })` against a test
//! server; here the model is built from the same config with a fixed
//! endpoint, and a [`RouteFetch`] answers the three URLs. A test that
//! replaces one route's response (`server.urls[url].response = ...`) does
//! so on its own [`Server`]. The test server reports `user-agent` apart from
//! the request headers, so it is left out of the header comparison.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, Size, Warning};
use aimux_provider_utils::HeaderMapOpt;

use crate::luma::LumaImageModel;
use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

const PROMPT: &str = "A cute baby sea otter";
const BASE_URL: &str = "https://api.example.com";
const GENERATIONS_URL: &str = "https://api.example.com/dream-machine/v1/generations/image";
const GENERATION_URL: &str =
    "https://api.example.com/dream-machine/v1/generations/test-generation-id";
const IMAGE_URL: &str = "https://api.example.com/image.png";

/// The base URL and the `headers` function of upstream's model config.
struct StaticEndpoint {
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: BASE_URL.to_string(),
            headers: self.headers.clone(),
        })
    }
}

fn json_value(body: &Value) -> Canned {
    Canned::json(body)
}

/// The submit answer of upstream's test server.
fn queued_generation() -> Value {
    json!({
        "id": "test-generation-id",
        "generation_type": "image",
        "state": "queued",
        "created_at": "2024-01-01T00:00:00Z",
        "model": "test-model",
        "request": {
            "generation_type": "image",
            "model": "test-model",
            "prompt": "A cute baby sea otter",
        },
    })
}

/// The poll answer of upstream's test server, with `request` replaced when
/// a test passes one.
fn completed_generation(request: Option<Value>) -> Value {
    json!({
        "id": "test-generation-id",
        "generation_type": "image",
        "state": "completed",
        "created_at": "2024-01-01T00:00:00Z",
        "assets": { "image": "https://api.example.com/image.png" },
        "model": "test-model",
        "request": request.unwrap_or_else(|| json!({
            "generation_type": "image",
            "model": "test-model",
            "prompt": "A cute baby sea otter",
        })),
    })
}

/// `createTestServer` with the three URLs of upstream's file.
struct Server {
    routes: Arc<Mutex<HashMap<&'static str, Canned>>>,
    fetch: Arc<RouteFetch>,
}

impl Server {
    fn new() -> Self {
        let routes: Arc<Mutex<HashMap<&'static str, Canned>>> = Arc::new(Mutex::new(
            [
                (GENERATIONS_URL, json_value(&queued_generation())),
                (GENERATION_URL, json_value(&completed_generation(None))),
                (
                    IMAGE_URL,
                    Canned {
                        status: 200,
                        headers: Vec::new(),
                        body: b"test-binary-content".to_vec(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        ));
        let table = Arc::clone(&routes);
        let fetch = RouteFetch::new(move |seen: &Seen| {
            table
                .lock()
                .unwrap()
                .get(seen.url.as_str())
                .cloned()
                .unwrap_or(Canned {
                    status: 404,
                    headers: Vec::new(),
                    body: b"Not found".to_vec(),
                })
        });
        Self { routes, fetch }
    }

    /// `server.urls[url].response = response`.
    fn set(&self, url: &'static str, response: Canned) {
        self.routes.lock().unwrap().insert(url, response);
    }

    /// `server.calls`.
    fn calls(&self) -> Vec<Seen> {
        self.fetch.seen()
    }

    /// `createBasicModel({ headers })`: `test-model`, provider `luma`, the
    /// default headers `{ 'api-key': 'test-key' }`.
    fn model(&self, headers: Option<&[(&str, &str)]>) -> LumaImageModel {
        let headers = headers
            .unwrap_or(&[("api-key", "test-key")])
            .iter()
            .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
            .collect();
        LumaImageModel::from_config(
            "test-model".to_string(),
            EndpointConfig::new(
                "luma".to_string(),
                StaticEndpoint { headers },
                Some(self.fetch.transport()),
            ),
        )
    }
}

/// `{ prompt, files: undefined, mask: undefined, n: 1, size: undefined,
/// aspectRatio: undefined, seed: undefined, providerOptions }`.
fn options(prompt: &str, provider_options: Value) -> ImageCallOptions {
    ImageCallOptions {
        provider_options: serde_json::from_value(provider_options).unwrap(),
        ..ImageCallOptions::new(prompt)
    }
}

fn url_file(url: &str) -> ImageFile {
    ImageFile::Url {
        url: url.to_string(),
    }
}

fn png_file(bytes: &[u8]) -> ImageFile {
    ImageFile::File {
        media_type: "image/png".to_string(),
        data: ImageFileData::Binary(bytes.to_vec()),
    }
}

/// `{ prompt, files, ... providerOptions }` for the image editing cases.
fn editing_options(
    prompt: &str,
    files: Vec<ImageFile>,
    provider_options: Value,
) -> ImageCallOptions {
    ImageCallOptions {
        files: Some(files),
        ..options(prompt, provider_options)
    }
}

/// `await server.calls[0].requestBodyJson` after a successful call.
async fn first_request_body(server: &Server, call: &ImageCallOptions) -> Value {
    server.model(None).do_generate(call).await.unwrap();
    server.calls()[0].json_body()
}

/// `rejects.toThrow(message)`: the message contains `message`.
async fn assert_throws(call: &ImageCallOptions, message: &str) {
    let server = Server::new();
    let error = server.model(None).do_generate(call).await.unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
}

mod luma_image_model {
    use super::*;

    mod capabilities {
        use super::*;

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › capabilities › advertises file=$supportsFileInputs and mask=$supportsMaskInputs for $modelId
        #[test]
        fn advertises_file_and_mask_support_per_model() {
            for (model_id, supports_file_inputs, supports_mask_inputs) in [
                ("photon-1", Some(true), Some(false)),
                ("custom-image-model", None, None),
            ] {
                let model = LumaImageModel::from_config(
                    model_id.to_string(),
                    EndpointConfig::new(
                        "luma.image".to_string(),
                        StaticEndpoint {
                            headers: HeaderMapOpt::new(),
                        },
                        None,
                    ),
                );
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

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should pass the correct parameters including aspect ratio
        #[tokio::test]
        async fn should_pass_the_correct_parameters_including_aspect_ratio() {
            let server = Server::new();
            let call = ImageCallOptions {
                aspect_ratio: Some(AspectRatio::new(16, 9)),
                ..options(PROMPT, json!({ "luma": { "additional_param": "value" } }))
            };
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "prompt": PROMPT,
                    "aspect_ratio": "16:9",
                    "model": "test-model",
                    "additional_param": "value",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should call the correct urls in sequence
        #[tokio::test]
        async fn should_call_the_correct_urls_in_sequence() {
            let server = Server::new();
            let call = ImageCallOptions {
                aspect_ratio: Some(AspectRatio::new(16, 9)),
                ..options(PROMPT, json!({}))
            };
            server.model(None).do_generate(&call).await.unwrap();

            let calls = server.calls();
            assert_eq!(calls[0].method, "POST");
            assert_eq!(calls[0].url, GENERATIONS_URL);
            assert_eq!(calls[1].method, "GET");
            assert_eq!(calls[1].url, GENERATION_URL);
            assert_eq!(calls[2].method, "GET");
            assert_eq!(calls[2].url, IMAGE_URL);
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let server = Server::new();
            let call = ImageCallOptions {
                headers: Some(
                    [(
                        "Custom-Request-Header".to_string(),
                        "request-header-value".to_string(),
                    )]
                    .into(),
                ),
                ..options(PROMPT, json!({}))
            };
            server
                .model(Some(
                    &[("Custom-Provider-Header", "provider-header-value")][..],
                ))
                .do_generate(&call)
                .await
                .unwrap();

            let mut headers: BTreeMap<String, String> = server.calls()[0].headers.clone();
            headers.remove("user-agent");
            assert_eq!(
                serde_json::to_value(headers).unwrap(),
                json!({
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should not pass providerOptions.{pollIntervalMillis,maxPollAttempts}
        #[tokio::test]
        async fn should_not_pass_poll_interval_millis_or_max_poll_attempts() {
            let server = Server::new();
            let call = ImageCallOptions {
                aspect_ratio: Some(AspectRatio::new(16, 9)),
                ..options(
                    PROMPT,
                    json!({ "luma": {
                        "pollIntervalMillis": 1000,
                        "maxPollAttempts": 5,
                        "additional_param": "value",
                    } }),
                )
            };
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "prompt": PROMPT,
                    "aspect_ratio": "16:9",
                    "model": "test-model",
                    "additional_param": "value",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should handle API errors
        #[tokio::test]
        async fn should_handle_api_errors() {
            let server = Server::new();
            server.set(
                GENERATIONS_URL,
                Canned {
                    status: 400,
                    headers: Vec::new(),
                    body: b"Bad Request".to_vec(),
                },
            );
            let error = server
                .model(None)
                .do_generate(&options(PROMPT, json!({})))
                .await
                .unwrap_err();

            let AiMuxError::ApiCall(error) = error else {
                panic!("expected an API call error, got {error:?}");
            };
            assert_eq!(error.message, "Bad Request");
            assert_eq!(error.status_code, Some(400));
            assert_eq!(error.url, GENERATIONS_URL);
            assert_eq!(error.request_body_values["prompt"], PROMPT);
            assert_eq!(error.response_body.as_deref(), Some("Bad Request"));
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › should handle failed generation state
        #[tokio::test]
        async fn should_handle_failed_generation_state() {
            let server = Server::new();
            server.set(
                GENERATION_URL,
                json_value(&json!({
                    "id": "test-generation-id",
                    "generation_type": "image",
                    "state": "failed",
                    "failure_reason": "Generation failed",
                    "created_at": "2024-01-01T00:00:00Z",
                    "model": "test-model",
                    "request": {
                        "generation_type": "image",
                        "model": "test-model",
                        "prompt": "A cute baby sea otter",
                    },
                })),
            );
            let error = server
                .model(None)
                .do_generate(&options(PROMPT, json!({})))
                .await
                .unwrap_err();

            assert!(
                matches!(error, AiMuxError::InvalidResponseData { .. }),
                "{error:?}"
            );
        }

        mod warnings {
            use super::*;

            /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › warnings › should return warnings for unsupported parameters
            #[tokio::test]
            async fn should_return_warnings_for_unsupported_parameters() {
                let server = Server::new();
                let call = ImageCallOptions {
                    size: Some(Size::new(1024, 1024)),
                    seed: Some(123),
                    ..options(PROMPT, json!({}))
                };
                let result = server.model(None).do_generate(&call).await.unwrap();

                assert_eq!(
                    result.warnings,
                    vec![
                        Warning::Unsupported {
                            feature: "seed".to_string(),
                            details: Some(
                                "This model does not support the `seed` option.".to_string()
                            ),
                        },
                        Warning::Unsupported {
                            feature: "size".to_string(),
                            details: Some(
                                "This model does not support the `size` option. Use `aspectRatio` instead."
                                    .to_string()
                            ),
                        },
                    ]
                );
            }
        }

        mod response_metadata {
            use super::*;

            /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › doGenerate › response metadata › should include timestamp, headers and modelId in response
            ///
            /// Upstream injects the date (`_internal.currentDate`); aimux reads
            /// the clock, so the timestamp is checked to fall within the call.
            #[tokio::test]
            async fn should_include_timestamp_headers_and_model_id_in_response() {
                let server = Server::new();
                let before = aimux_core::Timestamp::now();
                let result = server
                    .model(None)
                    .do_generate(&options(PROMPT, json!({})))
                    .await
                    .unwrap();
                let after = aimux_core::Timestamp::now();

                let timestamp = result.response.timestamp.expect("a timestamp");
                assert!(before <= timestamp && timestamp <= after);
                assert_eq!(result.response.model_id.as_deref(), Some("test-model"));
                assert!(result.response.headers.is_some());
            }
        }
    }

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › constructor › should expose correct provider and model information
        ///
        /// `specificationVersion` has no Rust counterpart (the trait is the
        /// version).
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let model = Server::new().model(None);

            assert_eq!(model.provider(), "luma");
            assert_eq!(model.model_id(), "test-model");
            assert_eq!(model.max_images_per_call(), Some(1));
        }
    }

    mod image_editing {
        use super::*;

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send image by default when URL file is provided
        #[tokio::test]
        async fn should_send_image_by_default_when_url_file_is_provided() {
            let server = Server::new();
            let call = editing_options(
                "A warrior with sunglasses",
                vec![url_file("https://example.com/input.jpg")],
                json!({}),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "image": [{ "url": "https://example.com/input.jpg", "weight": 0.85 }],
                    "model": "test-model",
                    "prompt": "A warrior with sunglasses",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send modify_image when referenceType is set
        #[tokio::test]
        async fn should_send_modify_image_when_reference_type_is_set() {
            let server = Server::new();
            let call = editing_options(
                "Transform flowers to sunflowers",
                vec![url_file("https://example.com/input.jpg")],
                json!({ "luma": { "referenceType": "modify_image" } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "model": "test-model",
                    "modify_image": { "url": "https://example.com/input.jpg", "weight": 1.0 },
                    "prompt": "Transform flowers to sunflowers",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send style when referenceType is style
        #[tokio::test]
        async fn should_send_style_when_reference_type_is_style() {
            let server = Server::new();
            let call = editing_options(
                "A dog in this style",
                vec![url_file("https://example.com/style.jpg")],
                json!({ "luma": { "referenceType": "style" } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "model": "test-model",
                    "prompt": "A dog in this style",
                    "style": [{ "url": "https://example.com/style.jpg", "weight": 0.8 }],
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send character when referenceType is character
        #[tokio::test]
        async fn should_send_character_when_reference_type_is_character() {
            let server = Server::new();
            let call = editing_options(
                "A warrior",
                vec![
                    url_file("https://example.com/person1.jpg"),
                    url_file("https://example.com/person2.jpg"),
                ],
                json!({ "luma": { "referenceType": "character" } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "character": {
                        "identity0": {
                            "images": [
                                "https://example.com/person1.jpg",
                                "https://example.com/person2.jpg",
                            ],
                        },
                    },
                    "model": "test-model",
                    "prompt": "A warrior",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send character with custom identity id from images config
        #[tokio::test]
        async fn should_send_character_with_custom_identity_id_from_images_config() {
            let server = Server::new();
            let call = editing_options(
                "A woman with a cat",
                vec![url_file("https://example.com/person.jpg")],
                json!({ "luma": {
                    "referenceType": "character",
                    "images": [{ "id": "identity0" }],
                } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "character": {
                        "identity0": { "images": ["https://example.com/person.jpg"] },
                    },
                    "model": "test-model",
                    "prompt": "A woman with a cat",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should send character with multiple identities from images config
        #[tokio::test]
        async fn should_send_character_with_multiple_identities_from_images_config() {
            let server = Server::new();
            let call = editing_options(
                "Two people talking",
                vec![
                    url_file("https://example.com/person1.jpg"),
                    url_file("https://example.com/person2.jpg"),
                ],
                json!({ "luma": {
                    "referenceType": "character",
                    "images": [{ "id": "identity0" }, { "id": "identity1" }],
                } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "character": {
                        "identity0": { "images": ["https://example.com/person1.jpg"] },
                        "identity1": { "images": ["https://example.com/person2.jpg"] },
                    },
                    "model": "test-model",
                    "prompt": "Two people talking",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should support multiple images for image
        #[tokio::test]
        async fn should_support_multiple_images_for_image() {
            let server = Server::new();
            let call = editing_options(
                "Combine these concepts",
                vec![
                    url_file("https://example.com/input1.jpg"),
                    url_file("https://example.com/input2.jpg"),
                ],
                json!({}),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "image": [
                        { "url": "https://example.com/input1.jpg", "weight": 0.85 },
                        { "url": "https://example.com/input2.jpg", "weight": 0.85 },
                    ],
                    "model": "test-model",
                    "prompt": "Combine these concepts",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should use custom weights from images config
        #[tokio::test]
        async fn should_use_custom_weights_from_images_config() {
            let server = Server::new();
            let call = editing_options(
                "Styled image",
                vec![url_file("https://example.com/input.jpg")],
                json!({ "luma": { "images": [{ "weight": 0.5 }] } }),
            );
            assert_eq!(
                first_request_body(&server, &call).await,
                json!({
                    "image": [{ "url": "https://example.com/input.jpg", "weight": 0.5 }],
                    "model": "test-model",
                    "prompt": "Styled image",
                })
            );
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should throw error when mask is provided
        #[tokio::test]
        async fn should_throw_error_when_mask_is_provided() {
            let call = ImageCallOptions {
                mask: Some(url_file("https://example.com/mask.png")),
                ..editing_options(
                    "Replace sky with sunset",
                    vec![url_file("https://example.com/input.jpg")],
                    json!({}),
                )
            };
            assert_throws(&call, "Luma AI does not support mask-based image editing").await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should throw error when base64 file data is provided
        #[tokio::test]
        async fn should_throw_error_when_base64_file_data_is_provided() {
            let call = editing_options(
                "Edit this image",
                vec![png_file(&[137, 80, 78, 71])],
                json!({}),
            );
            assert_throws(&call, "Luma AI only supports URL-based images").await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should throw error when base64 mask data is provided
        #[tokio::test]
        async fn should_throw_error_when_base64_mask_data_is_provided() {
            let call = ImageCallOptions {
                mask: Some(png_file(&[255, 255, 255, 0])),
                ..editing_options(
                    "Edit with mask",
                    vec![url_file("https://example.com/input.jpg")],
                    json!({}),
                )
            };
            assert_throws(&call, "Luma AI does not support mask-based image editing").await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should throw error when more than 4 images for image
        #[tokio::test]
        async fn should_throw_error_when_more_than_4_images_for_image() {
            let call = editing_options(
                "Too many images",
                (1..=5)
                    .map(|i| url_file(&format!("https://example.com/{i}.jpg")))
                    .collect(),
                json!({}),
            );
            assert_throws(&call, "Luma AI image supports up to 4 reference images").await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › Image Editing › should throw error when multiple files for modify_image
        #[tokio::test]
        async fn should_throw_error_when_multiple_files_for_modify_image() {
            let call = editing_options(
                "Edit multiple images",
                vec![
                    url_file("https://example.com/input1.jpg"),
                    url_file("https://example.com/input2.jpg"),
                ],
                json!({ "luma": { "referenceType": "modify_image" } }),
            );
            assert_throws(
                &call,
                "Luma AI modify_image only supports a single input image",
            )
            .await;
        }
    }

    mod response_schema_validation {
        use super::*;

        /// The poll answers with `request` set to `request`; the call
        /// succeeds with one image (upstream: `result.images` is defined).
        async fn parses_with_request(request: Value) {
            let server = Server::new();
            server.set(
                GENERATION_URL,
                json_value(&completed_generation(Some(request))),
            );
            let result = server
                .model(None)
                .do_generate(&options(PROMPT, json!({})))
                .await
                .unwrap();
            let ImageOutputs::Binary(images) = result.images else {
                panic!("expected binary images");
            };
            assert_eq!(images, vec![b"test-binary-content".to_vec()]);
        }

        fn base_request() -> Value {
            json!({
                "generation_type": "image",
                "model": "test-model",
                "prompt": "A cute baby sea otter",
            })
        }

        fn with(mut request: Value, key: &str, value: Value) -> Value {
            request[key] = value;
            request
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › response schema validation › should parse response with image references
        #[tokio::test]
        async fn should_parse_response_with_image_references() {
            parses_with_request(with(
                base_request(),
                "image_ref",
                json!([{ "url": "https://example.com/ref1.jpg", "weight": 0.85 }]),
            ))
            .await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › response schema validation › should parse response with style references
        #[tokio::test]
        async fn should_parse_response_with_style_references() {
            parses_with_request(with(
                base_request(),
                "style_ref",
                json!([{ "url": "https://example.com/style1.jpg", "weight": 0.8 }]),
            ))
            .await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › response schema validation › should parse response with character references
        #[tokio::test]
        async fn should_parse_response_with_character_references() {
            parses_with_request(with(
                base_request(),
                "character_ref",
                json!({ "identity0": { "images": ["https://example.com/character1.jpg"] } }),
            ))
            .await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › response schema validation › should parse response with modify image reference
        #[tokio::test]
        async fn should_parse_response_with_modify_image_reference() {
            parses_with_request(with(
                base_request(),
                "modify_image_ref",
                json!({ "url": "https://example.com/modify.jpg", "weight": 1.0 }),
            ))
            .await;
        }

        /// upstream: @ai-sdk/luma src/luma-image-model.test.ts › LumaImageModel › response schema validation › should parse response with multiple reference types
        #[tokio::test]
        async fn should_parse_response_with_multiple_reference_types() {
            let request = with(
                with(
                    with(
                        with(
                            base_request(),
                            "image_ref",
                            json!([{ "url": "https://example.com/ref1.jpg", "weight": 0.85 }]),
                        ),
                        "style_ref",
                        json!([{ "url": "https://example.com/style1.jpg", "weight": 0.8 }]),
                    ),
                    "character_ref",
                    json!({ "identity0": { "images": ["https://example.com/character1.jpg"] } }),
                ),
                "modify_image_ref",
                json!({ "url": "https://example.com/modify.jpg", "weight": 1.0 }),
            );
            parses_with_request(request).await;
        }
    }
}
