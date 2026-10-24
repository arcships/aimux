use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs,
};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};

use crate::google::{GoogleImageModel, GoogleImageSettings, GoogleProviderSettings, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch};

const TEST_URL: &str =
    "https://api.example.com/v1beta/models/gemini-2.5-flash-image:generateContent";
const BASE_URL: &str = "https://api.example.com/v1beta";

/// The arguments of `prepareJsonResponse`.
struct JsonResponse {
    images: Vec<(&'static str, &'static str)>,
    usage: Value,
    finish_reason: &'static str,
    headers: Vec<(String, String)>,
    grounding_metadata: Option<Value>,
}

impl Default for JsonResponse {
    fn default() -> Self {
        Self {
            images: vec![("image/png", "base64-generated-image")],
            usage: json!({"promptTokenCount": 10, "candidatesTokenCount": 100, "totalTokenCount": 110}),
            finish_reason: "STOP",
            headers: Vec::new(),
            grounding_metadata: None,
        }
    }
}

/// `prepareJsonResponse`.
fn prepare_json_response(response: JsonResponse) -> Canned {
    let mut candidate = json!({
        "content": {
            "parts": response
                .images
                .iter()
                .map(|(mime_type, data)| json!({"inlineData": {"mimeType": mime_type, "data": data}}))
                .collect::<Vec<_>>(),
            "role": "model",
        },
        "finishReason": response.finish_reason,
    });
    if let Some(grounding_metadata) = response.grounding_metadata {
        candidate["groundingMetadata"] = grounding_metadata;
    }
    let mut canned = Canned::json(&json!({
        "candidates": [candidate],
        "usageMetadata": response.usage,
    }));
    canned.headers.extend(response.headers);
    canned
}

/// `new GoogleImageModel(modelId, settings, { provider: 'google.generative-ai',
/// baseURL: 'https://api.example.com/v1beta', … })`.
fn image_model(
    model_id: &str,
    settings: GoogleImageSettings,
    mock: &Arc<MockFetch>,
) -> GoogleImageModel {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .image_with_settings(model_id, settings)
}

/// The shared `model`: `gemini-2.5-flash-image` with default settings.
fn model(mock: &Arc<MockFetch>) -> GoogleImageModel {
    image_model(
        "gemini-2.5-flash-image",
        GoogleImageSettings::default(),
        mock,
    )
}

fn call_options(prompt: &str) -> ImageCallOptions {
    ImageCallOptions::new(prompt.to_string())
}

fn google_options(value: Value) -> SharedProviderOptions {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    [("google".to_string(), map)].into_iter().collect()
}

fn base64_images(outputs: &ImageOutputs) -> Vec<String> {
    match outputs {
        ImageOutputs::Base64(images) => images.clone(),
        ImageOutputs::Binary(_) => panic!("expected base64 images"),
    }
}

fn google_metadata(result: &aimux_core::image_model::ImageResult) -> Value {
    serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap()["google"].clone()
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

mod google_image_model {
    use super::*;

    mod capabilities {
        use super::*;

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › capabilities › advertises file=$supportsFileInputs and mask=$supportsMaskInputs for $modelId
        #[test]
        fn advertises_file_and_mask_inputs_for_model_id() {
            for (model_id, supports_file_inputs, supports_mask_inputs) in [
                ("gemini-2.5-flash-image", Some(true), Some(false)),
                ("gemini-3-pro-image-preview", Some(true), Some(false)),
                ("gemini-3.1-flash-image-preview", Some(true), Some(false)),
                ("gemini-2.5-pro", None, None),
                ("gemini-3-pro-preview", None, None),
                ("gemini-custom", None, None),
                ("legacy-image-model", None, None),
            ] {
                let capability_model = image_model(
                    model_id,
                    GoogleImageSettings::default(),
                    &MockFetch::new(vec![]),
                );

                assert_eq!(
                    capability_model.supports_file_inputs(),
                    supports_file_inputs,
                    "{model_id}"
                );
                assert_eq!(
                    capability_model.supports_mask_inputs(),
                    supports_mask_inputs,
                    "{model_id}"
                );
            }
        }
    }

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should reject non-Gemini model IDs before sending a request
        #[tokio::test]
        async fn should_reject_non_gemini_model_ids_before_sending_a_request() {
            let mock = MockFetch::new(vec![]);
            let non_gemini_model =
                image_model("legacy-image-model", GoogleImageSettings::default(), &mock);

            let error = non_gemini_model
                .do_generate(&call_options("A beautiful sunset"))
                .await
                .unwrap_err();

            assert!(
                error.to_string().contains(
                    "Google image models other than Gemini are no longer supported. Use a model ID that starts with `gemini-`."
                ),
                "{error}"
            );
            assert!(mock.seen().is_empty());
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should use the language model endpoint and extract generated images
        #[tokio::test]
        async fn should_use_the_language_model_endpoint_and_extract_generated_images() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse::default())]);

            let result = model(&mock)
                .do_generate(&call_options("A beautiful sunset"))
                .await
                .unwrap();

            assert_eq!(mock.seen()[0].url, TEST_URL);
            assert_eq!(base64_images(&result.images), ["base64-generated-image"]);
            assert_eq!(
                serde_json::to_value(result.provider_metadata.as_ref().unwrap()).unwrap(),
                json!({
                    "google": {
                        "finishMessage": null,
                        "finishReason": "STOP",
                        "groundingMetadata": null,
                        "images": [{}],
                        "promptFeedback": null,
                        "safetyRatings": null,
                        "serviceTier": null,
                        "urlContextMetadata": null,
                        "usageMetadata": {
                            "candidatesTokenCount": 100,
                            "promptTokenCount": 10,
                            "totalTokenCount": 110,
                        },
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should expose the candidate finish reason in provider metadata
        #[tokio::test]
        async fn should_expose_the_candidate_finish_reason_in_provider_metadata() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse {
                images: vec![],
                finish_reason: "IMAGE_SAFETY",
                ..Default::default()
            })]);

            let result = model(&mock)
                .do_generate(&call_options("A blocked image prompt"))
                .await
                .unwrap();

            assert_match_object(
                &google_metadata(&result),
                &json!({"finishReason": "IMAGE_SAFETY", "images": []}),
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should leave empty responses with default prompt block reason %j retryable
        #[tokio::test]
        async fn should_leave_empty_responses_with_default_prompt_block_reason_j_retryable() {
            for block_reason in ["", "BLOCK_REASON_UNSPECIFIED", "BLOCKED_REASON_UNSPECIFIED"] {
                let mock = MockFetch::new(vec![Canned::json(&json!({
                    "candidates": [],
                    "promptFeedback": {"blockReason": block_reason},
                    "usageMetadata": {"promptTokenCount": 9, "totalTokenCount": 9},
                }))]);

                let result = model(&mock)
                    .do_generate(&call_options("An image prompt with an empty response"))
                    .await
                    .unwrap();

                assert!(base64_images(&result.images).is_empty(), "{block_reason:?}");
                assert_eq!(
                    serde_json::to_value(&result).unwrap().get("isRetryable"),
                    None,
                    "{block_reason:?}"
                );
                assert_match_object(
                    &google_metadata(&result),
                    &json!({"promptFeedback": {"blockReason": block_reason}}),
                );
            }
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should send response modalities, aspect ratio, seed, and headers
        #[tokio::test]
        async fn should_send_response_modalities_aspect_ratio_seed_and_headers() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse::default())]);
            // The config headers are only `Custom-Provider-Header`: the key
            // header is removed.
            let model_with_headers = create_google(GoogleProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(BASE_URL.into()),
                headers: Some(
                    [
                        ("x-goog-api-key".to_string(), None),
                        (
                            "Custom-Provider-Header".to_string(),
                            Some("provider-header-value".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
                fetch: Some(mock.transport()),
                ..Default::default()
            })
            .unwrap()
            .image("gemini-2.5-flash-image");

            let mut options = call_options("A beautiful sunset");
            options.aspect_ratio = Some(AspectRatio::new(21, 9));
            options.seed = Some(12345);
            options.provider_options = google_options(json!({"imageConfig": {"imageSize": "4K"}}));
            options.headers = Some(
                [(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )]
                .into_iter()
                .collect(),
            );
            model_with_headers.do_generate(&options).await.unwrap();

            let seen = &mock.seen()[0];
            // The test server reports the user agent separately.
            let mut headers = seen.headers.clone();
            headers.remove("user-agent");
            assert_eq!(
                serde_json::to_value(&headers).unwrap(),
                json!({
                    "content-type": "application/json",
                    "custom-provider-header": "provider-header-value",
                    "custom-request-header": "request-header-value",
                })
            );
            assert_eq!(
                seen.json_body(),
                json!({
                    "contents": [{"parts": [{"text": "A beautiful sunset"}], "role": "user"}],
                    "generationConfig": {
                        "imageConfig": {"aspectRatio": "21:9", "imageSize": "4K"},
                        "responseModalities": ["IMAGE"],
                        "seed": 12345,
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should include usage and response metadata
        #[tokio::test]
        async fn should_include_usage_and_response_metadata() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse {
                usage: json!({"promptTokenCount": 20, "candidatesTokenCount": 200, "totalTokenCount": 220}),
                headers: vec![("request-id".into(), "test-request-id".into())],
                ..Default::default()
            })]);
            // No `currentDate` hook in Rust: the timestamp is the real time.
            let before = aimux_core::Timestamp::now();
            let result = model(&mock)
                .do_generate(&call_options("A beautiful sunset"))
                .await
                .unwrap();
            let after = aimux_core::Timestamp::now();

            assert_eq!(
                serde_json::to_value(result.usage.as_ref().unwrap()).unwrap(),
                json!({"inputTokens": 20, "outputTokens": 200, "totalTokens": 220})
            );
            let timestamp = result.response.timestamp.unwrap();
            assert!(before <= timestamp && timestamp <= after);
            assert_eq!(
                result.response.model_id.as_deref(),
                Some("gemini-2.5-flash-image")
            );
            assert_eq!(
                result
                    .response
                    .headers
                    .as_ref()
                    .unwrap()
                    .get("request-id")
                    .map(String::as_str),
                Some("test-request-id")
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should return a warning for the unsupported size option
        #[tokio::test]
        async fn should_return_a_warning_for_the_unsupported_size_option() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse::default())]);

            let mut options = call_options("A beautiful sunset");
            options.size = Some(Size::new(1024, 1024));
            let result = model(&mock).do_generate(&options).await.unwrap();

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{
                    "type": "unsupported",
                    "feature": "size",
                    "details": "This model does not support the `size` option. Use `aspectRatio` instead.",
                }])
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should forward Google Search grounding and its metadata
        #[tokio::test]
        async fn should_forward_google_search_grounding_and_its_metadata() {
            let grounding_metadata = json!({
                "webSearchQueries": ["example query"],
                "groundingChunks": [{"web": {"uri": "https://example.com/source", "title": "Example"}}],
            });
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse {
                grounding_metadata: Some(grounding_metadata),
                ..Default::default()
            })]);

            let mut options = call_options("A beautiful sunset");
            options.provider_options =
                google_options(json!({"googleSearch": {"searchTypes": {"imageSearch": {}}}}));
            let result = model(&mock).do_generate(&options).await.unwrap();

            let request_body = mock.seen()[0].json_body();
            assert_eq!(
                request_body["tools"],
                json!([{"googleSearch": {"searchTypes": {"imageSearch": {}}}}])
            );
            assert!(
                request_body["generationConfig"]
                    .get("googleSearch")
                    .is_none()
            );
            assert_eq!(
                google_metadata(&result),
                json!({
                    "finishMessage": null,
                    "finishReason": "STOP",
                    "groundingMetadata": {
                        "groundingChunks": [
                            {"web": {"title": "Example", "uri": "https://example.com/source"}},
                        ],
                        "webSearchQueries": ["example query"],
                    },
                    "images": [{}],
                    "promptFeedback": null,
                    "safetyRatings": null,
                    "serviceTier": null,
                    "urlContextMetadata": null,
                    "usageMetadata": {
                        "candidatesTokenCount": 100,
                        "promptTokenCount": 10,
                        "totalTokenCount": 110,
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should include input images for editing
        #[tokio::test]
        async fn should_include_input_images_for_editing() {
            let mock = MockFetch::new(vec![prepare_json_response(JsonResponse::default())]);

            let mut options = call_options("Add a hat to this cat");
            options.files = Some(vec![ImageFile::File {
                media_type: "image/png".into(),
                data: ImageFileData::Base64("base64-source-image".into()),
            }]);
            model(&mock).do_generate(&options).await.unwrap();

            assert_eq!(
                mock.seen()[0].json_body()["contents"],
                json!([{
                    "role": "user",
                    "parts": [
                        {"text": "Add a hat to this cat"},
                        {"inlineData": {"mimeType": "image/png", "data": "base64-source-image"}},
                    ],
                }])
            );
        }

        /// upstream: @ai-sdk/google src/google-image-model.test.ts › GoogleImageModel › doGenerate › should reject unsupported URL editing input and masks
        #[tokio::test]
        async fn should_reject_unsupported_url_editing_input_and_masks() {
            let mock = MockFetch::new(vec![
                prepare_json_response(JsonResponse::default()),
                prepare_json_response(JsonResponse::default()),
            ]);
            let model = model(&mock);

            let mut options = call_options("Add a hat to this cat");
            options.files = Some(vec![ImageFile::Url {
                url: "https://example.com/cat.png".into(),
            }]);
            let error = model.do_generate(&options).await.unwrap_err();
            assert!(
                regex::Regex::new(r#"media type "image/\*".*not passed as inline bytes"#)
                    .unwrap()
                    .is_match(&error.to_string()),
                "{error}"
            );

            let mut options = call_options("Edit this image");
            options.mask = Some(ImageFile::File {
                media_type: "image/png".into(),
                data: ImageFileData::Base64("base64-mask-image".into()),
            });
            let error = model.do_generate(&options).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Gemini image models do not support mask-based image editing."),
                "{error}"
            );
        }
    }
}
