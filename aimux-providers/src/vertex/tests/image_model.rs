use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};
use aimux_provider_utils::HeaderMapOpt;

use crate::shared::{Endpoint, EndpointConfig, ResolveEndpoint};
use crate::test_support::mock_fetch::{Canned, RouteFetch};
use crate::vertex::VertexImageModel;

const TEST_URL: &str = "https://api.example.com/models/gemini-2.5-flash-image:generateContent";

/// A base URL and headers fixed for every request (the upstream model config).
struct StaticEndpoint {
    base_url: String,
    headers: HeaderMapOpt,
}

#[async_trait::async_trait]
impl ResolveEndpoint for StaticEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        Ok(Endpoint {
            base_url: self.base_url.clone(),
            headers: self.headers.clone(),
        })
    }
}

/// `new GoogleVertexImageModel(modelId, { provider: 'google.vertex.image', baseURL: 'https://api.example.com', headers })`
/// against the test server.
fn model_with(
    model_id: &str,
    headers: &[(&str, &str)],
    server: &Arc<RouteFetch>,
) -> VertexImageModel {
    VertexImageModel::from_config(
        model_id.to_string(),
        EndpointConfig::new(
            "google.vertex.image".to_string(),
            StaticEndpoint {
                base_url: "https://api.example.com".to_string(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), Some(v.to_string())))
                    .collect(),
            },
            Some(server.transport()),
        ),
    )
}

/// The shared `model` of the upstream file.
fn model(server: &Arc<RouteFetch>) -> VertexImageModel {
    model_with("gemini-2.5-flash-image", &[("api-key", "test-key")], server)
}

fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let body = serde_json::to_vec(body).unwrap();
    let mut all = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ];
    all.extend(headers.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Canned {
        status: 200,
        headers: all,
        body,
    }
}

/// The AI SDK test server: `TEST_URL` answers `response` (when set), anything
/// else 404.
fn test_server(response: Option<Canned>) -> Arc<RouteFetch> {
    RouteFetch::new(move |seen| match &response {
        Some(canned) if seen.url == TEST_URL => canned.clone(),
        _ => Canned {
            status: 404,
            headers: Vec::new(),
            body: b"Not found".to_vec(),
        },
    })
}

/// `prepareJsonResponse({ usage, headers })` with the default image.
fn prepare_json_response(usage: Option<Value>, headers: &[(&str, &str)]) -> Arc<RouteFetch> {
    let usage = usage.unwrap_or_else(
        || json!({"promptTokenCount": 10, "candidatesTokenCount": 100, "totalTokenCount": 110}),
    );
    test_server(Some(json_response(
        &json!({
            "candidates": [{
                "content": {
                    "parts": [{"inlineData": {"mimeType": "image/png", "data": "base64-generated-image"}}],
                    "role": "model",
                },
                "finishReason": "STOP",
            }],
            "usageMetadata": usage,
        }),
        headers,
    )))
}

fn provider_options(value: Value) -> SharedProviderOptions {
    serde_json::from_value(value).unwrap()
}

/// The message of an error, without the variant's display prefix.
fn message(error: &AiMuxError) -> String {
    match error {
        AiMuxError::InvalidArgument(message)
        | AiMuxError::UnsupportedFunctionality(message)
        | AiMuxError::InvalidPrompt(message)
        | AiMuxError::InvalidResponseData(message) => message.clone(),
        other => other.to_string(),
    }
}

mod google_vertex_image_model {
    use super::*;

    mod capabilities {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › capabilities › advertises file=$supportsFileInputs and mask=$supportsMaskInputs for $modelId
        #[test]
        fn advertises_file_and_mask_support_for_model_id() {
            let server = test_server(None);
            for (model_id, supports_file_inputs, supports_mask_inputs) in [
                ("gemini-2.5-flash-image", Some(true), Some(false)),
                ("gemini-3-pro-image-preview", Some(true), Some(false)),
                ("gemini-3.1-flash-image-preview", Some(true), Some(false)),
                ("gemini-2.5-pro", None, None),
                ("gemini-3-pro-preview", None, None),
                ("gemini-custom", None, None),
                ("legacy-image-model", None, None),
            ] {
                let capability_model = model_with(model_id, &[], &server);
                assert_eq!(
                    capability_model.supports_file_inputs(),
                    supports_file_inputs,
                    "supportsFileInputs for {model_id}"
                );
                assert_eq!(
                    capability_model.supports_mask_inputs(),
                    supports_mask_inputs,
                    "supportsMaskInputs for {model_id}"
                );
            }
        }
    }

    /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › should return 1 for maxImagesPerCall
    #[test]
    fn should_return_1_for_max_images_per_call() {
        let server = test_server(None);
        assert_eq!(model(&server).max_images_per_call(), Some(1));
    }

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should reject non-Gemini model IDs before sending a request
        #[tokio::test]
        async fn should_reject_non_gemini_model_ids_before_sending_a_request() {
            let server = test_server(None);
            let non_gemini_model =
                model_with("legacy-image-model", &[("api-key", "test-key")], &server);
            let error = non_gemini_model
                .do_generate(&ImageCallOptions::new("A beautiful sunset"))
                .await
                .unwrap_err();
            assert!(
                message(&error).contains(
                    "Google image models other than Gemini are no longer supported. Use a model ID that starts with `gemini-`."
                ),
                "{error:?}"
            );
            assert!(server.seen().is_empty());
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should use the language model endpoint and extract generated images
        #[tokio::test]
        async fn should_use_the_language_model_endpoint_and_extract_generated_images() {
            let server = prepare_json_response(None, &[]);
            let result = model(&server)
                .do_generate(&ImageCallOptions::new("A beautiful sunset"))
                .await
                .unwrap();
            assert_eq!(server.seen()[0].url, TEST_URL);
            assert!(
                matches!(&result.images, ImageOutputs::Base64(images) if images == &["base64-generated-image"]),
                "{:?}",
                result.images
            );
            assert_eq!(
                serde_json::to_value(result.provider_metadata).unwrap(),
                json!({
                    "googleVertex": {"images": [{}]},
                    "vertex": {"images": [{}]},
                })
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should send response modalities, aspect ratio, seed, and headers
        #[tokio::test]
        async fn should_send_response_modalities_aspect_ratio_seed_and_headers() {
            let server = prepare_json_response(None, &[]);
            let model_with_headers = model_with(
                "gemini-2.5-flash-image",
                &[("Custom-Provider-Header", "provider-header-value")],
                &server,
            );
            let mut options = ImageCallOptions::new("A beautiful sunset");
            options.aspect_ratio = Some(AspectRatio::new(21, 9));
            options.seed = Some(12345);
            options.headers = Some(
                [(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )]
                .into(),
            );
            model_with_headers.do_generate(&options).await.unwrap();
            let seen = server.seen();
            assert_eq!(
                seen[0]
                    .headers
                    .get("custom-provider-header")
                    .map(String::as_str),
                Some("provider-header-value")
            );
            assert_eq!(
                seen[0]
                    .headers
                    .get("custom-request-header")
                    .map(String::as_str),
                Some("request-header-value")
            );
            assert_eq!(
                seen[0].json_body(),
                json!({
                    "contents": [{"parts": [{"text": "A beautiful sunset"}], "role": "user"}],
                    "generationConfig": {
                        "imageConfig": {"aspectRatio": "21:9"},
                        "responseModalities": ["IMAGE"],
                        "seed": 12345,
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should accept the legacy vertex options key and prefer googleVertex
        #[tokio::test]
        async fn should_accept_the_legacy_vertex_options_key_and_prefer_google_vertex() {
            let server = prepare_json_response(None, &[]);
            let mut options = ImageCallOptions::new("A beautiful sunset");
            options.aspect_ratio = Some(AspectRatio::new(16, 9));
            options.provider_options = provider_options(json!({
                "googleVertex": {"imageConfig": {"imageSize": "4K"}},
                "vertex": {"imageConfig": {"imageSize": "2K"}},
            }));
            model(&server).do_generate(&options).await.unwrap();
            assert_eq!(
                server.seen()[0].json_body()["generationConfig"]["imageConfig"],
                json!({"aspectRatio": "16:9", "imageSize": "4K"})
            );

            let mut options = ImageCallOptions::new("A beautiful sunset");
            options.provider_options =
                provider_options(json!({"vertex": {"imageConfig": {"imageSize": "2K"}}}));
            model(&server).do_generate(&options).await.unwrap();
            assert_eq!(
                server.seen()[1].json_body()["generationConfig"]["imageConfig"],
                json!({"imageSize": "2K"})
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should include usage and response metadata
        #[tokio::test]
        async fn should_include_usage_and_response_metadata() {
            let server = prepare_json_response(
                Some(
                    json!({"promptTokenCount": 20, "candidatesTokenCount": 200, "totalTokenCount": 220}),
                ),
                &[("request-id", "test-request-id")],
            );
            // aimux cannot inject `currentDate`: the timestamp is asserted to
            // be set, not to equal a fixed date.
            let before = aimux_core::Timestamp::now();
            let result = model(&server)
                .do_generate(&ImageCallOptions::new("A beautiful sunset"))
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(result.usage).unwrap(),
                json!({"inputTokens": 20, "outputTokens": 200, "totalTokens": 220})
            );
            let timestamp = result.response.timestamp.expect("timestamp");
            assert!(timestamp.unix_millis() >= before.unix_millis());
            assert_eq!(
                result.response.model_id.as_deref(),
                Some("gemini-2.5-flash-image")
            );
            assert_eq!(
                result
                    .response
                    .headers
                    .as_ref()
                    .and_then(|headers| headers.get("request-id"))
                    .map(String::as_str),
                Some("test-request-id")
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should return a warning for the unsupported size option
        #[tokio::test]
        async fn should_return_a_warning_for_the_unsupported_size_option() {
            let server = prepare_json_response(None, &[]);
            let mut options = ImageCallOptions::new("A beautiful sunset");
            options.size = Some(Size::new(1024, 1024));
            let result = model(&server).do_generate(&options).await.unwrap();
            assert_eq!(
                serde_json::to_value(result.warnings).unwrap(),
                json!([{
                    "type": "unsupported",
                    "feature": "size",
                    "details": "This model does not support the `size` option. Use `aspectRatio` instead.",
                }])
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should include input images for editing
        #[tokio::test]
        async fn should_include_input_images_for_editing() {
            let server = prepare_json_response(None, &[]);
            let mut options = ImageCallOptions::new("Add a hat to this cat");
            options.files = Some(vec![ImageFile::File {
                media_type: "image/png".into(),
                data: ImageFileData::Base64("base64-source-image".into()),
            }]);
            model(&server).do_generate(&options).await.unwrap();
            assert_eq!(
                server.seen()[0].json_body()["contents"],
                json!([{
                    "role": "user",
                    "parts": [
                        {"text": "Add a hat to this cat"},
                        {"inlineData": {"mimeType": "image/png", "data": "base64-source-image"}},
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/google-vertex src/google-vertex-image-model.test.ts › GoogleVertexImageModel › doGenerate › should reject unsupported URL editing input and masks
        #[tokio::test]
        async fn should_reject_unsupported_url_editing_input_and_masks() {
            let server = prepare_json_response(None, &[]);
            let mut options = ImageCallOptions::new("Add a hat to this cat");
            options.files = Some(vec![ImageFile::Url {
                url: "https://example.com/cat.png".into(),
            }]);
            let error = model(&server).do_generate(&options).await.unwrap_err();
            let text = message(&error);
            let media_type = text.find("media type \"image/*\"");
            assert!(
                media_type.is_some_and(|at| text[at..].contains("not passed as inline bytes")),
                "{error:?}"
            );

            let mut options = ImageCallOptions::new("Edit this image");
            options.mask = Some(ImageFile::File {
                media_type: "image/png".into(),
                data: ImageFileData::Base64("base64-mask-image".into()),
            });
            let error = model(&server).do_generate(&options).await.unwrap_err();
            assert!(
                message(&error)
                    .contains("Gemini image models do not support mask-based image editing."),
                "{error:?}"
            );
        }
    }
}
