//! Port of `@ai-sdk/openai-compatible` 3.0.62
//! `src/image/openai-compatible-image-model.test.ts`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::AiMuxError;
use aimux_core::Timestamp;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, Size};
use aimux_provider_utils::{HeaderMapOpt, ProviderErrorParts};

use crate::openai_compatible::OpenAICompatibleImageModel;
use crate::openai_compatible::error::ErrorStructure;
use crate::openai_compatible::{BaseUrl, ChatSettings, CompatModelConfig};
use crate::shared::{Credential, ProviderHeaders};
use crate::test_support::mock_fetch::{Canned, RouteFetch, Seen};

const PROMPT: &str = "A photorealistic astronaut riding a horse";

const GENERATIONS_URL: &str = "https://api.example.com/dall-e-3/images/generations";
const RECRAFT_URL: &str = "https://external.api.recraft.ai/v1/images/generations";
const EDITS_URL: &str = "https://api.example.com/dall-e-3/images/edits";

/// The provider-utils part of the user agent the openai-compatible package
/// pins (upstream's helper layer adds it; its test server lists headers
/// without it).
const USER_AGENT: &str = "ai-sdk-provider-utils/5.0.53";

fn json_response(body: Value) -> Canned {
    Canned::json(&body)
}

/// Upstream's `createTestServer` routes: the generation, recraft and edit
/// endpoints with their default responses; `generations` / `edits` replace
/// a route's response.
fn create_test_server(generations: Option<Canned>, edits: Option<Canned>) -> Arc<RouteFetch> {
    let generations = generations.unwrap_or_else(|| {
        json_response(json!({
            "data": [{ "b64_json": "test1234" }, { "b64_json": "test5678" }],
        }))
    });
    let recraft = json_response(json!({ "data": [{ "b64_json": "recraft-test-image" }] }));
    let edits = edits.unwrap_or_else(|| {
        json_response(json!({ "data": [{ "b64_json": "edited-image-base64" }] }))
    });
    RouteFetch::new(move |seen: &Seen| match seen.url.as_str() {
        GENERATIONS_URL => generations.clone(),
        RECRAFT_URL => recraft.clone(),
        EDITS_URL => edits.clone(),
        _ => Canned {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        },
    })
}

fn server() -> Arc<RouteFetch> {
    create_test_server(None, None)
}

fn header_opt_map(entries: &[(&str, &str)]) -> HeaderMapOpt {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_string(), Some((*value).to_string())))
        .collect()
}

/// `new OpenAICompatibleImageModel(modelId, { provider, headers, url,
/// errorStructure })` on the test server's transport. The base URL is what
/// upstream's `url({ modelId, path })` puts before `path`.
fn image_model(
    server: &Arc<RouteFetch>,
    model_id: &str,
    provider: &str,
    base_url: &str,
    headers: &[(&str, &str)],
    error_structure: Option<ErrorStructure>,
) -> OpenAICompatibleImageModel {
    let mut chat = ChatSettings::default();
    if let Some(error_structure) = error_structure {
        chat.error_structure = error_structure;
    }
    OpenAICompatibleImageModel::from_config(
        model_id.to_string(),
        CompatModelConfig {
            provider: provider.to_string(),
            base_url: BaseUrl::Fixed(base_url.to_string()),
            query_params: None,
            headers: ProviderHeaders::bearer(
                Credential::None,
                Vec::new(),
                Some(header_opt_map(headers)),
            ),
            fetch_headers: None,
            fetch: Some(crate::shared::package_fetch(
                Some(server.transport()),
                crate::shared::aisdk_pins::OPENAI_COMPATIBLE,
            )),
            body_rewrite: None,
            transform_request_body: None,
            chat,
        },
    )
}

/// Upstream's `createBasicModel({ headers, errorStructure })`.
fn create_basic_model(
    server: &Arc<RouteFetch>,
    headers: Option<&[(&str, &str)]>,
    error_structure: Option<ErrorStructure>,
) -> OpenAICompatibleImageModel {
    image_model(
        server,
        "dall-e-3",
        "openai-compatible",
        "https://api.example.com/dall-e-3",
        headers.unwrap_or(&[("Authorization", "Bearer test-key")]),
        error_structure,
    )
}

/// Upstream's `createDefaultGenerateParams()`.
fn create_default_generate_params() -> ImageCallOptions {
    ImageCallOptions {
        size: Some(Size::new(1024, 1024)),
        headers: Some(HashMap::new()),
        ..ImageCallOptions::new(PROMPT)
    }
}

fn with_provider_options(
    mut options: ImageCallOptions,
    provider_options: Value,
) -> ImageCallOptions {
    options.provider_options = serde_json::from_value(provider_options).unwrap();
    options
}

fn png_file(bytes: &[u8]) -> ImageFile {
    ImageFile::File {
        media_type: "image/png".to_string(),
        data: ImageFileData::Binary(bytes.to_vec()),
    }
}

fn base64_images(outputs: &ImageOutputs) -> Vec<String> {
    match outputs {
        ImageOutputs::Base64(images) => images.clone(),
        ImageOutputs::Binary(_) => panic!("expected base64 images"),
    }
}

/// The request headers without `user-agent`, which is checked against the
/// pinned value.
fn request_headers(seen: &Seen) -> BTreeMap<String, String> {
    let mut headers = seen.headers.clone();
    assert_eq!(
        headers.remove("user-agent").as_deref(),
        Some(USER_AGENT),
        "pinned user agent"
    );
    headers
}

/// The API call error's `message`, `statusCode` and `url`
/// (`rejects.toMatchObject`).
fn api_error_parts(error: &AiMuxError) -> (String, Option<u16>, String) {
    match error {
        AiMuxError::ApiCall(error) => (error.message.clone(), error.status_code, error.url.clone()),
        other => panic!("expected an API call error, got {other:?}"),
    }
}

mod open_ai_compatible_image_model {
    use super::*;

    mod constructor {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › constructor › should expose correct provider and model information
        #[test]
        fn should_expose_correct_provider_and_model_information() {
            let model = create_basic_model(&server(), None, None);

            assert_eq!(model.provider(), "openai-compatible");
            assert_eq!(model.model_id(), "dall-e-3");
            assert_eq!(model.specification_version(), "v4");
            assert_eq!(model.max_images_per_call(), Some(10));
            assert_eq!(model.supports_file_inputs(), None);
            assert_eq!(model.supports_mask_inputs(), None);
        }
    }

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should pass the correct parameters
        #[tokio::test]
        async fn should_pass_the_correct_parameters() {
            let server = server();
            let model = create_basic_model(&server, None, None);

            let mut options = with_provider_options(
                create_default_generate_params(),
                json!({ "openaiCompatible": { "quality": "hd" } }),
            );
            options.n = 2;
            model.do_generate(&options).await.unwrap();

            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "model": "dall-e-3",
                    "prompt": PROMPT,
                    "n": 2,
                    "size": "1024x1024",
                    "quality": "hd",
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should use provider name from config for providerOptions key
        #[tokio::test]
        async fn should_use_provider_name_from_config_for_provider_options_key() {
            let server = server();
            let recraft_model = image_model(
                &server,
                "recraft-v3",
                "recraft.image",
                "https://external.api.recraft.ai/v1",
                &[("Authorization", "Bearer test-key")],
                None,
            );

            let mut options = with_provider_options(
                create_default_generate_params(),
                json!({ "recraft": { "style": "vector_illustration" } }),
            );
            options.prompt = Some("A beautiful sunset".to_string());
            recraft_model.do_generate(&options).await.unwrap();

            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "model": "recraft-v3",
                    "prompt": "A beautiful sunset",
                    "n": 1,
                    "size": "1024x1024",
                    "style": "vector_illustration",
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should pass typed image output options
        #[tokio::test]
        async fn should_pass_typed_image_output_options() {
            let server = server();
            let model = create_basic_model(&server, None, None);

            model
                .do_generate(&with_provider_options(
                    create_default_generate_params(),
                    json!({
                        "openaiCompatible": {
                            "size": "auto",
                            "quality": "high",
                            "output_format": "jpeg",
                            "output_compression": 80,
                            "background": "opaque",
                        },
                    }),
                ))
                .await
                .unwrap();

            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "model": "dall-e-3",
                    "prompt": PROMPT,
                    "n": 1,
                    "size": "auto",
                    "quality": "high",
                    "output_format": "jpeg",
                    "output_compression": 80,
                    "background": "opaque",
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should emit deprecated warning when raw provider options key is used for hyphenated provider
        #[tokio::test]
        async fn should_emit_deprecated_warning_for_raw_key_of_hyphenated_provider() {
            let server = server();
            let model = image_model(
                &server,
                "dall-e-3",
                "black-forest-labs.image",
                "https://api.example.com/dall-e-3",
                &[("Authorization", "Bearer test-key")],
                None,
            );

            let result = model
                .do_generate(&with_provider_options(
                    create_default_generate_params(),
                    json!({ "black-forest-labs": { "quality": "hd" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "message": "Use 'blackForestLabs' instead.",
                    "setting": "providerOptions key 'black-forest-labs'",
                    "type": "deprecated",
                }])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should not emit deprecated warning when camelCase provider options key is used
        #[tokio::test]
        async fn should_not_emit_deprecated_warning_when_camel_case_provider_options_key_is_used() {
            let server = server();
            let model = image_model(
                &server,
                "dall-e-3",
                "black-forest-labs.image",
                "https://api.example.com/dall-e-3",
                &[("Authorization", "Bearer test-key")],
                None,
            );

            let result = model
                .do_generate(&with_provider_options(
                    create_default_generate_params(),
                    json!({ "blackForestLabs": { "quality": "hd" } }),
                ))
                .await
                .unwrap();

            assert_eq!(serde_json::to_value(&result.warnings).unwrap(), json!([]));
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should add warnings for unsupported settings
        #[tokio::test]
        async fn should_add_warnings_for_unsupported_settings() {
            let server = server();
            let model = create_basic_model(&server, None, None);

            let mut options = create_default_generate_params();
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            options.seed = Some(123);
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([
                    {
                        "details": "This model does not support aspect ratio. Use `size` instead.",
                        "feature": "aspectRatio",
                        "type": "unsupported",
                    },
                    {
                        "feature": "seed",
                        "type": "unsupported",
                    },
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should pass headers
        #[tokio::test]
        async fn should_pass_headers() {
            let server = server();
            let model_with_headers = create_basic_model(
                &server,
                Some(&[("Custom-Provider-Header", "provider-header-value")]),
                None,
            );

            let mut options = create_default_generate_params();
            options.headers = Some(HashMap::from([(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]));
            model_with_headers.do_generate(&options).await.unwrap();

            assert_eq!(
                request_headers(&server.seen()[0]),
                BTreeMap::from([
                    ("content-type".to_string(), "application/json".to_string()),
                    (
                        "custom-provider-header".to_string(),
                        "provider-header-value".to_string()
                    ),
                    (
                        "custom-request-header".to_string(),
                        "request-header-value".to_string()
                    ),
                ])
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should handle API errors with custom error structure
        #[tokio::test]
        async fn should_handle_api_errors_with_custom_error_structure() {
            let server = create_test_server(
                Some(Canned {
                    status: 400,
                    headers: Vec::new(),
                    body: serde_json::to_vec(&json!({
                        "status": "error",
                        "details": {
                            "errorMessage": "Custom provider error format",
                            "errorCode": 1234,
                        },
                    }))
                    .unwrap(),
                }),
                None,
            );

            // `errorSchema` + `errorToMessage`: `Error ${errorCode}: ${errorMessage}`.
            let error_structure: ErrorStructure = Arc::new(|data: &Value| ProviderErrorParts {
                message: format!(
                    "Error {}: {}",
                    data["details"]["errorCode"],
                    data["details"]["errorMessage"].as_str().unwrap_or_default()
                ),
                provider_code: None,
            });
            let model = create_basic_model(&server, None, Some(error_structure));

            let error = model
                .do_generate(&create_default_generate_params())
                .await
                .unwrap_err();

            assert_eq!(
                api_error_parts(&error),
                (
                    "Error 1234: Custom provider error format".to_string(),
                    Some(400),
                    GENERATIONS_URL.to_string(),
                )
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should handle API errors with default error structure
        #[tokio::test]
        async fn should_handle_api_errors_with_default_error_structure() {
            let server = create_test_server(
                Some(Canned {
                    status: 400,
                    headers: Vec::new(),
                    body: serde_json::to_vec(&json!({
                        "error": {
                            "message": "Invalid prompt content",
                            "type": "invalid_request_error",
                            "param": null,
                            "code": null,
                        },
                    }))
                    .unwrap(),
                }),
                None,
            );

            let model = create_basic_model(&server, None, None);

            let error = model
                .do_generate(&create_default_generate_params())
                .await
                .unwrap_err();

            assert_eq!(
                api_error_parts(&error),
                (
                    "Invalid prompt content".to_string(),
                    Some(400),
                    GENERATIONS_URL.to_string(),
                )
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should return the raw b64_json content
        #[tokio::test]
        async fn should_return_the_raw_b64_json_content() {
            let model = create_basic_model(&server(), None, None);
            let mut options = create_default_generate_params();
            options.n = 2;
            let result = model.do_generate(&options).await.unwrap();

            let images = base64_images(&result.images);
            assert_eq!(images.len(), 2);
            assert_eq!(images[0], "test1234");
            assert_eq!(images[1], "test5678");
        }

        mod usage {
            use super::*;

            /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › usage › should map the usage object reported by the provider
            #[tokio::test]
            async fn should_map_the_usage_object_reported_by_the_provider() {
                let server = create_test_server(
                    Some(json_response(json!({
                        "data": [{ "b64_json": "test1234" }],
                        "usage": {
                            "input_tokens": 12,
                            "output_tokens": 4,
                            "total_tokens": 16,
                        },
                    }))),
                    None,
                );

                let model = create_basic_model(&server, None, None);
                let result = model
                    .do_generate(&create_default_generate_params())
                    .await
                    .unwrap();

                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({ "inputTokens": 12, "outputTokens": 4, "totalTokens": 16 })
                );
            }

            /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › usage › should return undefined usage when the provider omits it
            #[tokio::test]
            async fn should_return_undefined_usage_when_the_provider_omits_it() {
                let model = create_basic_model(&server(), None, None);
                let result = model
                    .do_generate(&create_default_generate_params())
                    .await
                    .unwrap();

                assert!(result.usage.is_none());
            }

            /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › usage › should map null usage fields to undefined
            #[tokio::test]
            async fn should_map_null_usage_fields_to_undefined() {
                let server = create_test_server(
                    Some(json_response(json!({
                        "data": [{ "b64_json": "test1234" }],
                        "usage": {
                            "input_tokens": 7,
                            "output_tokens": null,
                            "total_tokens": null,
                        },
                    }))),
                    None,
                );

                let model = create_basic_model(&server, None, None);
                let result = model
                    .do_generate(&create_default_generate_params())
                    .await
                    .unwrap();

                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({ "inputTokens": 7 })
                );
            }

            /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › usage › should ignore unknown usage fields
            #[tokio::test]
            async fn should_ignore_unknown_usage_fields() {
                let server = create_test_server(
                    Some(json_response(json!({
                        "data": [{ "b64_json": "test1234" }],
                        "usage": {
                            "input_tokens": 3,
                            "output_tokens": 1,
                            "total_tokens": 4,
                            "input_tokens_details": { "image_tokens": 2, "text_tokens": 1 },
                        },
                    }))),
                    None,
                );

                let model = create_basic_model(&server, None, None);
                let result = model
                    .do_generate(&create_default_generate_params())
                    .await
                    .unwrap();

                assert_eq!(
                    serde_json::to_value(&result.usage).unwrap(),
                    json!({ "inputTokens": 3, "outputTokens": 1, "totalTokens": 4 })
                );
            }
        }

        mod response_metadata {
            use super::*;

            /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › response metadata › should include timestamp, headers and modelId in response
            #[tokio::test]
            async fn should_include_timestamp_headers_and_model_id_in_response() {
                // Upstream injects `currentDate: () => testDate`; aimux has no
                // date seam, so the timestamp is checked to be the call's time.
                let model = create_basic_model(&server(), None, None);

                let before = Timestamp::now();
                let result = model
                    .do_generate(&create_default_generate_params())
                    .await
                    .unwrap();
                let after = Timestamp::now();

                let timestamp = result.response.timestamp.expect("timestamp");
                assert!(before <= timestamp && timestamp <= after);
                assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
                assert!(result.response.headers.is_some());
            }
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should use real date when no custom date provider is specified
        #[tokio::test]
        async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
            let before_date = Timestamp::now();

            let model = image_model(
                &server(),
                "dall-e-3",
                "openai-compatible",
                "https://api.example.com/dall-e-3",
                &[("Authorization", "Bearer test-key")],
                None,
            );

            let result = model
                .do_generate(&create_default_generate_params())
                .await
                .unwrap();

            let after_date = Timestamp::now();

            let timestamp = result.response.timestamp.expect("timestamp");
            assert!(timestamp.unix_millis() >= before_date.unix_millis());
            assert!(timestamp.unix_millis() <= after_date.unix_millis());
            assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should pass the user setting in the request
        #[tokio::test]
        async fn should_pass_the_user_setting_in_the_request() {
            let server = server();
            let model = create_basic_model(&server, None, None);

            model
                .do_generate(&with_provider_options(
                    create_default_generate_params(),
                    json!({ "openaiCompatible": { "user": "test-user-id" } }),
                ))
                .await
                .unwrap();

            assert_eq!(
                server.seen()[0].json_body(),
                json!({
                    "model": "dall-e-3",
                    "prompt": PROMPT,
                    "n": 1,
                    "size": "1024x1024",
                    "user": "test-user-id",
                })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › doGenerate › should not include user field in request when not set via provider options
        #[tokio::test]
        async fn should_not_include_user_field_in_request_when_not_set_via_provider_options() {
            let server = server();
            let model = create_basic_model(&server, None, None);

            model
                .do_generate(&with_provider_options(
                    create_default_generate_params(),
                    json!({ "openaiCompatible": {} }),
                ))
                .await
                .unwrap();

            let request_body = server.seen()[0].json_body();
            assert_eq!(
                request_body,
                json!({
                    "model": "dall-e-3",
                    "prompt": PROMPT,
                    "n": 1,
                    "size": "1024x1024",
                })
            );
            assert!(request_body.get("user").is_none());
        }
    }

    mod image_editing {
        use super::*;

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should send edit request with files
        #[tokio::test]
        async fn should_send_edit_request_with_files() {
            let edit_server = server();
            let model = create_basic_model(&edit_server, None, None);

            // PNG magic bytes
            let image_data = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Turn the cat into a dog".to_string());
            options.files = Some(vec![png_file(&image_data)]);
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(base64_images(&result.images), ["edited-image-base64"]);
            assert_eq!(edit_server.seen()[0].url, EDITS_URL);
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should send edit request with files and mask
        #[tokio::test]
        async fn should_send_edit_request_with_files_and_mask() {
            let edit_server = server();
            let model = create_basic_model(&edit_server, None, None);

            let image_data = [137, 80, 78, 71];
            let mask_data = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Add a flamingo to the pool".to_string());
            options.files = Some(vec![png_file(&image_data)]);
            options.mask = Some(png_file(&mask_data));
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(base64_images(&result.images), ["edited-image-base64"]);
            assert_eq!(edit_server.seen()[0].url, EDITS_URL);
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should send edit request with Uint8Array data
        #[tokio::test]
        async fn should_send_edit_request_with_uint8_array_data() {
            let model = create_basic_model(&server(), None, None);

            let image_uint8_array = [104, 101, 108, 108, 111];

            let mut options = create_default_generate_params();
            options.prompt = Some("Edit this image".to_string());
            options.files = Some(vec![png_file(&image_uint8_array)]);
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(base64_images(&result.images), ["edited-image-base64"]);
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should send edit request with multiple images
        #[tokio::test]
        async fn should_send_edit_request_with_multiple_images() {
            let model = create_basic_model(&server(), None, None);

            let image1 = [137, 80, 78, 71];
            let image2 = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Combine these images".to_string());
            options.files = Some(vec![png_file(&image1), png_file(&image2)]);
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(base64_images(&result.images), ["edited-image-base64"]);
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should include response metadata for edit requests
        #[tokio::test]
        async fn should_include_response_metadata_for_edit_requests() {
            // Upstream injects `currentDate: () => testDate`; aimux has no
            // date seam, so the timestamp is checked to be the call's time.
            let model = create_basic_model(&server(), None, None);

            let image_data = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Edit this image".to_string());
            options.files = Some(vec![png_file(&image_data)]);
            let before = Timestamp::now();
            let result = model.do_generate(&options).await.unwrap();
            let after = Timestamp::now();

            let timestamp = result.response.timestamp.expect("timestamp");
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
            assert!(result.response.headers.is_some());
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should map usage for edit requests
        #[tokio::test]
        async fn should_map_usage_for_edit_requests() {
            let edit_server = create_test_server(
                None,
                Some(json_response(json!({
                    "data": [{ "b64_json": "edited-image-base64" }],
                    "usage": {
                        "input_tokens": 20,
                        "output_tokens": 5,
                        "total_tokens": 25,
                    },
                }))),
            );

            let model = create_basic_model(&edit_server, None, None);
            let image_data = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Edit this image".to_string());
            options.files = Some(vec![png_file(&image_data)]);
            let result = model.do_generate(&options).await.unwrap();

            assert_eq!(
                serde_json::to_value(&result.usage).unwrap(),
                json!({ "inputTokens": 20, "outputTokens": 5, "totalTokens": 25 })
            );
        }

        /// upstream: @ai-sdk/openai-compatible src/image/openai-compatible-image-model.test.ts › OpenAICompatibleImageModel › Image Editing › should return undefined usage for edit requests when omitted
        #[tokio::test]
        async fn should_return_undefined_usage_for_edit_requests_when_omitted() {
            let model = create_basic_model(&server(), None, None);
            let image_data = [137, 80, 78, 71];

            let mut options = create_default_generate_params();
            options.prompt = Some("Edit this image".to_string());
            options.files = Some(vec![png_file(&image_data)]);
            let result = model.do_generate(&options).await.unwrap();

            assert!(result.usage.is_none());
        }
    }
}

// ── Older ports (previously `tests/openai_compatible_upstream_e2e_test.rs`) ──
//
// End-to-end subset of the pinned openai-compatible upstream tests.

mod upstream_e2e {
    use aimux_core::AiMuxError;
    use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs};
    use aimux_core::shared::Size;
    use serde_json::json;

    use crate::openai_compatible::tests::support::*;
    use crate::test_support::mock_fetch::{Canned, MockFetch};

    const IMAGE_PROMPT: &str = "A photorealistic astronaut riding a horse";

    fn image_options() -> ImageCallOptions {
        let mut opts = ImageCallOptions::new(IMAGE_PROMPT);
        opts.size = Some(Size::new(1024, 1024));
        opts
    }

    /// partial: `should return the raw b64_json content`; full port in src/openai_compatible/tests/image/image_model.rs.
    /// partial: `should map the usage object reported by the provider`; full port in src/openai_compatible/tests/image/image_model.rs.
    #[tokio::test]
    async fn image_generate_raw_content_and_usage() {
        let fetch = MockFetch::new(vec![Canned::json(&json!({"data":[
            {"b64_json":"test1234"},{"b64_json":"test5678"}],
            "usage":{"input_tokens":12,"output_tokens":4,"total_tokens":16}}))]);
        let mut opts = image_options();
        opts.n = 2;
        let result = provider(&fetch)
            .image("test-image")
            .do_generate(&opts)
            .await
            .unwrap();
        assert_request(
            &fetch,
            "/images/generations",
            json!({"model":"test-image",
            "prompt":IMAGE_PROMPT,"n":2,"size":"1024x1024"}),
        );
        let ImageOutputs::Base64(images) = result.images else {
            panic!("expected base64 images");
        };
        assert_eq!(images, vec!["test1234", "test5678"]);
        let usage = result.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.total_tokens, Some(16));
        assert_eq!(result.response.model_id.as_deref(), Some("test-image"));
        assert_eq!(
            result.response.headers.unwrap()["content-type"],
            "application/json"
        );
        assert!(result.warnings.is_empty());
    }

    /// partial: `should handle API errors with default error structure`; full port in src/openai_compatible/tests/image/image_model.rs.
    #[tokio::test]
    async fn image_generate_api_error() {
        let mut canned = Canned::json(&json!({"error":{"message":"Invalid prompt content",
            "type":"invalid_request_error","param":null,"code":null}}));
        canned.status = 400;
        let fetch = MockFetch::new(vec![canned]);
        let error = provider(&fetch)
            .image("test-image")
            .do_generate(&image_options())
            .await
            .unwrap_err();
        assert_request(
            &fetch,
            "/images/generations",
            json!({"model":"test-image",
            "prompt":IMAGE_PROMPT,"n":1,"size":"1024x1024"}),
        );
        let AiMuxError::ApiCall(error) = error else {
            panic!("expected API error");
        };
        assert_eq!(error.message, "Invalid prompt content");
        assert_eq!(error.status_code, Some(400));
        assert_eq!(error.url, format!("{BASE_URL}/images/generations"));
    }
}
