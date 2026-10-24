use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageFileData, ImageModel};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};

const PROMPT: &str = "A cute baby sea otter";
const BASE_URL: &str = "https://api.openai.com/v1";
const PINNED_USER_AGENT: &str = "ai-sdk-openai/4.0.83";

fn fixture(name: &str) -> Value {
    let text = match name {
        "openai-image" => include_str!("fixtures/openai-image.json"),
        "openai-image-edit" => include_str!("fixtures/openai-image-edit.json"),
        other => panic!("unknown fixture {other}"),
    };
    serde_json::from_str(text).unwrap()
}

/// The test server's `json-value` response: the compact JSON body with its
/// `content-type` and `content-length`, plus the given headers.
fn json_response(body: &Value, headers: &[(&str, &str)]) -> Canned {
    let mut canned = Canned::json(body);
    canned
        .headers
        .push(("content-length".into(), canned.body.len().to_string()));
    for (name, value) in headers {
        canned.headers.push(((*name).into(), (*value).into()));
    }
    canned
}

fn provider_with(mock: &Arc<MockFetch>, settings: OpenAIProviderSettings) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(BASE_URL.into()),
        fetch: Some(mock.transport()),
        ..settings
    })
    .unwrap()
}

fn provider(mock: &Arc<MockFetch>) -> OpenAIProvider {
    provider_with(mock, OpenAIProviderSettings::default())
}

/// A provider for the cases that make no request.
fn offline_provider() -> OpenAIProvider {
    provider(&MockFetch::new(vec![]))
}

/// `{ prompt, n: 1, size, providerOptions }`, everything else undefined.
fn options(size: Option<Size>, provider_options: Value) -> ImageCallOptions {
    ImageCallOptions {
        size,
        provider_options: serde_json::from_value::<SharedProviderOptions>(provider_options)
            .unwrap(),
        ..ImageCallOptions::new(PROMPT)
    }
}

fn size_1024() -> Option<Size> {
    Some(Size::new(1024, 1024))
}

fn png_file() -> ImageFile {
    ImageFile::File {
        media_type: "image/png".into(),
        data: ImageFileData::Binary(vec![137, 80, 78, 71]),
    }
}

fn edit_options(size: Option<Size>, provider_options: Value) -> ImageCallOptions {
    ImageCallOptions {
        files: Some(vec![png_file()]),
        ..options(size, provider_options)
    }
}

/// The test server's `requestBodyMultipart`: text fields as strings (a
/// repeated field becomes an array), file fields as
/// `{ "filename", "contentType", "size" }`.
fn multipart_body(seen: &Seen) -> Map<String, Value> {
    let content_type = seen.headers.get("content-type").expect("content-type");
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart boundary");
    let delimiter = format!("--{boundary}");
    let body = &seen.body;
    let mut fields = Map::new();
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(offset) = find(&body[start..], delimiter.as_bytes()) {
        positions.push(start + offset);
        start += offset + delimiter.len();
    }
    for window in positions.windows(2) {
        let part = &body[window[0] + delimiter.len()..window[1]];
        let part = part.strip_prefix(b"\r\n").unwrap_or(part);
        let part = part.strip_suffix(b"\r\n").unwrap_or(part);
        let header_end = find(part, b"\r\n\r\n").expect("part headers");
        let headers = String::from_utf8_lossy(&part[..header_end]).to_string();
        let content = &part[header_end + 4..];
        let name = between(&headers, "name=\"", "\"").expect("part name");
        let value = match between(&headers, "filename=\"", "\"") {
            Some(filename) => json!({
                "filename": filename,
                "contentType": headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Type: "))
                    .unwrap_or_default(),
                "size": content.len(),
            }),
            None => Value::String(String::from_utf8(content.to_vec()).unwrap()),
        };
        match fields.get_mut(&name) {
            Some(Value::Array(values)) => values.push(value),
            Some(existing) => *existing = Value::Array(vec![existing.take(), value]),
            None => {
                fields.insert(name, value);
            }
        }
    }
    fields
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn between(text: &str, start: &str, end: &str) -> Option<String> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(text[from..to].to_string())
}

/// `toMatchObject`: every key of `expected` matches in `actual` (objects
/// recursively, arrays element by element with the same length).
fn assert_matches_object(actual: &Value, expected: &Value) {
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
        "expected {actual:#} to match {expected:#}"
    );
}

mod image_editing_capabilities {
    use super::*;

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › image editing capabilities › advertises file and mask support for %s
    #[test]
    fn advertises_file_and_mask_support_for_s() {
        for model_id in [
            "dall-e-2",
            "gpt-image-1",
            "gpt-image-1-mini",
            "gpt-image-1.5",
            "gpt-image-2",
            "gpt-image-2.5-flare",
            "gpt-image-2.5-flare-2026-09-08",
            "gpt-image-2.5-sunburst",
            "gpt-image-2.5-sunburst-2026-09-08",
            "chatgpt-image-latest",
        ] {
            let model = offline_provider().image(model_id);

            assert_eq!(model.supports_file_inputs(), Some(true), "{model_id}");
            assert_eq!(model.supports_mask_inputs(), Some(true), "{model_id}");
        }
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › image editing capabilities › advertises that dall-e-3 does not support editing inputs
    #[test]
    fn advertises_that_dall_e_3_does_not_support_editing_inputs() {
        let model = offline_provider().image("dall-e-3");

        assert_eq!(model.supports_file_inputs(), Some(false));
        assert_eq!(model.supports_mask_inputs(), Some(false));
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › image editing capabilities › leaves editing support unknown for %s
    #[test]
    fn leaves_editing_support_unknown_for_s() {
        for model_id in [
            "custom-image-model",
            "gpt-image-custom",
            "chatgpt-image-custom",
        ] {
            let model = offline_provider().image(model_id);

            assert_eq!(model.supports_file_inputs(), None, "{model_id}");
            assert_eq!(model.supports_mask_inputs(), None, "{model_id}");
        }
    }
}

mod do_generate {
    use super::*;

    mod s_quality {
        use super::*;

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › %s quality › should pass %s quality
        #[tokio::test]
        async fn should_pass_s_quality() {
            for model_id in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
                for quality in ["xhigh", "max"] {
                    let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);

                    provider(&mock)
                        .image(model_id)
                        .do_generate(&options(
                            size_1024(),
                            json!({ "openai": { "quality": quality } }),
                        ))
                        .await
                        .unwrap();

                    assert_eq!(
                        mock.seen()[0].json_body(),
                        json!({
                            "model": model_id,
                            "prompt": PROMPT,
                            "n": 1,
                            "size": "1024x1024",
                            "quality": quality,
                        })
                    );
                }
            }
        }
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should pass the model and the settings
    #[tokio::test]
    async fn should_pass_the_model_and_the_settings() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);

        provider(&mock)
            .image("dall-e-3")
            .do_generate(&options(
                size_1024(),
                json!({ "openai": { "style": "vivid" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "dall-e-3",
                "prompt": PROMPT,
                "n": 1,
                "size": "1024x1024",
                "style": "vivid",
                "response_format": "b64_json",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should map provider options to snake_case for /images/generations
    #[tokio::test]
    async fn should_map_provider_options_to_snake_case_for_images_generations() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);

        provider(&mock)
            .image("gpt-image-1")
            .do_generate(&options(
                size_1024(),
                json!({
                    "openai": {
                        "quality": "high",
                        "background": "transparent",
                        "moderation": "low",
                        "outputFormat": "webp",
                        "outputCompression": 80,
                        "user": "user-123",
                    },
                }),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "model": "gpt-image-1",
                "prompt": PROMPT,
                "n": 1,
                "size": "1024x1024",
                "quality": "high",
                "background": "transparent",
                "moderation": "low",
                "output_format": "webp",
                "output_compression": 80,
                "user": "user-123",
            })
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-organization".into()),
                project: Some("test-project".into()),
                headers: Some(HashMap::from([(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )])),
                ..Default::default()
            },
        );

        provider
            .image("dall-e-3")
            .do_generate(&ImageCallOptions {
                headers: Some(HashMap::from([(
                    "Custom-Request-Header".to_string(),
                    "request-header-value".to_string(),
                )])),
                ..options(size_1024(), json!({ "openai": { "style": "vivid" } }))
            })
            .await
            .unwrap();

        let mut headers = mock.seen()[0].headers.clone();
        let user_agent = headers.remove("user-agent").unwrap_or_default();
        assert_eq!(
            headers,
            BTreeMap::from([
                (
                    "authorization".to_string(),
                    "Bearer test-api-key".to_string()
                ),
                ("content-type".to_string(), "application/json".to_string()),
                (
                    "custom-provider-header".to_string(),
                    "provider-header-value".to_string()
                ),
                (
                    "custom-request-header".to_string(),
                    "request-header-value".to_string()
                ),
                (
                    "openai-organization".to_string(),
                    "test-organization".to_string()
                ),
                ("openai-project".to_string(), "test-project".to_string()),
            ])
        );
        assert!(user_agent.contains(PINNED_USER_AGENT), "{user_agent}");
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should include response data with timestamp, modelId and headers
    #[tokio::test]
    async fn should_include_response_data_with_timestamp_model_id_and_headers() {
        let mock = MockFetch::new(vec![json_response(
            &fixture("openai-image"),
            &[
                ("x-request-id", "test-request-id"),
                ("x-ratelimit-remaining", "123"),
            ],
        )]);
        // Upstream's custom model sends no provider headers; the response is
        // what is asserted.
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                name: Some("test-provider".into()),
                ..Default::default()
            },
        );

        let result = provider
            .image("dall-e-3")
            .do_generate(&options(size_1024(), json!({})))
            .await
            .unwrap();

        // The clock cannot be injected (upstream: `2024-03-15T12:00:00Z`).
        assert!(result.response.timestamp.is_some());
        assert_matches_object(
            &serde_json::to_value(&result.response).unwrap(),
            &json!({
                "modelId": "dall-e-3",
                "headers": {
                    "content-type": "application/json",
                    "x-request-id": "test-request-id",
                    "x-ratelimit-remaining": "123",
                },
            }),
        );
    }

    async fn assert_no_response_format(model_id: &str) {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);

        provider(&mock)
            .image(model_id)
            .do_generate(&options(size_1024(), json!({})))
            .await
            .unwrap();

        let request_body = mock.seen().last().unwrap().json_body();
        assert_eq!(
            request_body,
            json!({
                "model": model_id,
                "prompt": PROMPT,
                "n": 1,
                "size": "1024x1024",
            })
        );
        assert!(request_body.get("response_format").is_none());
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should not include response_format for gpt-image-1
    #[tokio::test]
    async fn should_not_include_response_format_for_gpt_image_1() {
        assert_no_response_format("gpt-image-1").await;
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should not include response_format for gpt-image-2
    #[tokio::test]
    async fn should_not_include_response_format_for_gpt_image_2() {
        assert_no_response_format("gpt-image-2").await;
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should not include response_format for future gpt-image models
    #[tokio::test]
    async fn should_not_include_response_format_for_future_gpt_image_models() {
        assert_no_response_format("gpt-image-99").await;
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should not include response_format for chatgpt-image-latest
    #[tokio::test]
    async fn should_not_include_response_format_for_chatgpt_image_latest() {
        assert_no_response_format("chatgpt-image-latest").await;
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should not include response_format for date-suffixed gpt-image model IDs (Azure deployment names)
    #[tokio::test]
    async fn should_not_include_response_format_for_date_suffixed_gpt_image_model_ids() {
        assert_no_response_format("gpt-image-1.5-2025-12-16").await;
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should handle null revised_prompt responses
    #[tokio::test]
    async fn should_handle_null_revised_prompt_responses() {
        let mock = MockFetch::new(vec![json_response(
            &json!({
                "created": 1733837122,
                "data": [{ "revised_prompt": null, "b64_json": "base64-image-1" }],
            }),
            &[],
        )]);

        let result = provider(&mock)
            .image("gpt-image-1")
            .do_generate(&options(size_1024(), json!({})))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.images).unwrap(),
            json!(["base64-image-1"])
        );
        assert!(result.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(&result.provider_metadata).unwrap(),
            json!({ "openai": { "images": [{ "created": 1733837122 }] } })
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should return image meta data
    #[tokio::test]
    async fn should_return_image_meta_data() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image"), &[])]);

        let result = provider(&mock)
            .image("dall-e-3")
            .do_generate(&options(
                size_1024(),
                json!({ "openai": { "style": "vivid" } }),
            ))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.provider_metadata).unwrap(),
            json!({
                "openai": {
                    "images": [
                        {
                            "created": 1770935200,
                            "revisedPrompt": "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed.",
                        },
                        { "created": 1770935200 },
                    ],
                },
            })
        );
    }
}

mod do_generate_image_editing {
    use super::*;

    // should forward the abort signal when downloading a URL %s: not ported
    // (ts-only: upstream stubs the global `fetch` used for downloads; aimux
    // downloads image URLs with `download_blob` on the process transport
    // behind the SSRF guard, which rejects loopback, so neither the mock
    // transport nor a local server can serve the download).

    mod s_quality {
        use super::*;

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › %s quality › should pass %s quality
        #[tokio::test]
        async fn should_pass_s_quality() {
            for model_id in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
                for quality in ["xhigh", "max"] {
                    let mock =
                        MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

                    provider(&mock)
                        .image(model_id)
                        .do_generate(&edit_options(
                            size_1024(),
                            json!({ "openai": { "quality": quality } }),
                        ))
                        .await
                        .unwrap();

                    assert_matches_object(
                        &Value::Object(multipart_body(&mock.seen()[0])),
                        &json!({ "model": model_id, "quality": quality }),
                    );
                }
            }
        }
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should send image as form data with Uint8Array input
    #[tokio::test]
    async fn should_send_image_as_form_data_with_uint8array_input() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

        provider(&mock)
            .image("gpt-image-1")
            .do_generate(&edit_options(size_1024(), json!({})))
            .await
            .unwrap();

        assert_matches_object(
            &Value::Object(multipart_body(&mock.seen()[0])),
            &json!({
                "model": "gpt-image-1",
                "prompt": PROMPT,
                "n": "1",
                "size": "1024x1024",
            }),
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should send image as form data with base64 string input
    #[tokio::test]
    async fn should_send_image_as_form_data_with_base64_string_input() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

        provider(&mock)
            .image("gpt-image-1")
            .do_generate(&ImageCallOptions {
                files: Some(vec![ImageFile::File {
                    media_type: "image/png".into(),
                    data: ImageFileData::Base64("iVBORw0KGgo=".into()),
                }]),
                ..options(None, json!({}))
            })
            .await
            .unwrap();

        let multipart = multipart_body(&mock.seen()[0]);
        assert_eq!(multipart.get("model"), Some(&json!("gpt-image-1")));
        assert!(multipart.get("image").is_some());
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should pass provider options in form data
    #[tokio::test]
    async fn should_pass_provider_options_in_form_data() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

        provider(&mock)
            .image("gpt-image-1")
            .do_generate(&edit_options(
                size_1024(),
                json!({ "openai": { "quality": "high", "background": "transparent" } }),
            ))
            .await
            .unwrap();

        assert_matches_object(
            &Value::Object(multipart_body(&mock.seen()[0])),
            &json!({ "quality": "high", "background": "transparent" }),
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should map provider options to snake_case for /images/edits
    #[tokio::test]
    async fn should_map_provider_options_to_snake_case_for_images_edits() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

        provider(&mock)
            .image("gpt-image-1")
            .do_generate(&edit_options(
                size_1024(),
                json!({
                    "openai": {
                        "inputFidelity": "high",
                        "outputFormat": "webp",
                        "outputCompression": 80,
                        "user": "user-123",
                    },
                }),
            ))
            .await
            .unwrap();

        assert_matches_object(
            &Value::Object(multipart_body(&mock.seen()[0])),
            &json!({
                "input_fidelity": "high",
                "output_format": "webp",
                "output_compression": "80",
                "user": "user-123",
            }),
        );
    }

    /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should return warnings for unsupported settings in edit mode
    #[tokio::test]
    async fn should_return_warnings_for_unsupported_settings_in_edit_mode() {
        let mock = MockFetch::new(vec![json_response(&fixture("openai-image-edit"), &[])]);

        let result = provider(&mock)
            .image("gpt-image-1")
            .do_generate(&ImageCallOptions {
                aspect_ratio: Some(AspectRatio::new(16, 9)),
                seed: Some(42),
                ..edit_options(size_1024(), json!({}))
            })
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.warnings).unwrap(),
            json!([
                {
                    "details": "This model does not support aspect ratio. Use `size` instead.",
                    "feature": "aspectRatio",
                    "type": "unsupported",
                },
                { "feature": "seed", "type": "unsupported" },
            ])
        );
    }
}

// ── Older ports (previously `tests/openai_image_test.rs`) ──
//
// Rust translation of the OpenAI image model tests.
//
// Source: `reference/ai/packages/openai/src/image/openai-image-model.test.ts`
//
// Each test spins up a `wiremock` mock server, configures a JSON response,
// creates an `OpenAIImageModel` pointing at the mock, calls `do_generate`,
// and asserts on the request body / headers / result.
//
// The TS tests inject a custom `currentDate` via `_internal` for timestamp
// assertions. The Rust model always uses `Utc::now()`; timestamp tests
// verify that a timestamp is present and that `model_id` matches, rather
// than asserting an exact timestamp value.

mod older_image {
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::{OpenAIProviderSettings, create_openai};
    use aimux_core::image_model::{ImageCallOptions, ImageFile, ImageFileData, ImageOutputs};
    use aimux_core::provider::Provider;
    use aimux_core::shared::{AspectRatio, Size};

    // ── helpers ─────────────────────────────────────────────────────────────────

    const PROMPT: &str = "A cute baby sea otter";

    /// The standard generation fixture response (2 images, first with revised_prompt).
    fn image_response_body() -> Value {
        json!({
            "created": 1770935200,
            "data": [
                {
                    "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj",
                    "revised_prompt": "A small and adorable baby sea otter. This little creature is covered in a thick and fluffy brown fur, its tiny paws are slightly visible. The otter has bright, curious eyes and it's floating on its back on a calm sea, surrounded by floating seaweed."
                },
                {
                    "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
                }
            ]
        })
    }

    /// The standard edit fixture response (1 image).
    fn image_edit_response_body() -> Value {
        json!({
            "created": 1770935251,
            "background": "opaque",
            "data": [
                {
                    "b64_json": "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAFEE2NhQlgAAUQTanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
                }
            ],
            "output_format": "png",
            "quality": "high",
            "size": "1024x1024"
        })
    }

    /// Mount a mock JSON response at `/images/generations`.
    async fn mock_generations_response(server: &MockServer, body: Value) {
        mock_generations_response_with_headers(server, body, &[]).await;
    }

    /// Mount a mock JSON response at `/images/generations` with extra headers.
    async fn mock_generations_response_with_headers(
        server: &MockServer,
        body: Value,
        headers: &[(&str, &str)],
    ) {
        let mut template = ResponseTemplate::new(200).set_body_json(body);
        for (k, v) in headers {
            template = template.insert_header(*k, *v);
        }
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(template)
            .mount(server)
            .await;
    }

    /// Mount a mock JSON response at `/images/edits`.
    async fn mock_edits_response(server: &MockServer, body: Value) {
        mock_edits_response_with_headers(server, body, &[]).await;
    }

    /// Mount a mock JSON response at `/images/edits` with extra headers.
    async fn mock_edits_response_with_headers(
        server: &MockServer,
        body: Value,
        headers: &[(&str, &str)],
    ) {
        let mut template = ResponseTemplate::new(200).set_body_json(body);
        for (k, v) in headers {
            template = template.insert_header(*k, *v);
        }
        Mock::given(method("POST"))
            .and(path("/images/edits"))
            .respond_with(template)
            .mount(server)
            .await;
    }

    /// Build options with a prompt and n=1.
    fn options(prompt: &str) -> ImageCallOptions {
        ImageCallOptions::new(prompt.to_string())
    }

    /// Build a binary image file.
    fn binary_file(media_type: &str, bytes: &[u8]) -> ImageFile {
        ImageFile::File {
            media_type: media_type.to_string(),
            data: ImageFileData::Binary(bytes.to_vec()),
        }
    }

    /// Build a base64 image file.
    fn base64_file(media_type: &str, b64: &str) -> ImageFile {
        ImageFile::File {
            media_type: media_type.to_string(),
            data: ImageFileData::Base64(b64.to_string()),
        }
    }

    // ════════════════════════════════════════════════════════════════════════════
    // doGenerate — generation
    // ════════════════════════════════════════════════════════════════════════════

    mod do_generate {
        use super::*;

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should extract the generated images
        #[tokio::test]
        async fn should_extract_the_generated_images() {
            let server = MockServer::start().await;
            mock_generations_response(&server, image_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("dall-e-3").unwrap();

            let result = model.do_generate(&options(PROMPT)).await.unwrap();

            match &result.images {
                ImageOutputs::Base64(imgs) => {
                    assert_eq!(imgs.len(), 2);
                    assert_eq!(
                        imgs[0],
                        "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAA3CGNhQlgAADcIanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
                    );
                    assert_eq!(
                        imgs[1],
                        "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAEp2GNhQlgAASnYanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
                    );
                }
                _ => panic!("expected Base64 images"),
            }
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should return warnings for unsupported settings
        #[tokio::test]
        async fn should_return_warnings_for_unsupported_settings() {
            let server = MockServer::start().await;
            mock_generations_response(&server, image_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("dall-e-3").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.size = Some(Size::new(1024, 1024));
            opts.aspect_ratio = Some(AspectRatio::new(1, 1));
            opts.seed = Some(123);

            let result = model.do_generate(&opts).await.unwrap();

            assert_eq!(result.warnings.len(), 2);
            // aspectRatio warning
            match &result.warnings[0] {
                aimux_core::types::Warning::Unsupported { feature, details } => {
                    assert_eq!(feature, "aspectRatio");
                    assert_eq!(
                        details.as_deref(),
                        Some("This model does not support aspect ratio. Use `size` instead.")
                    );
                }
                _ => panic!("expected Unsupported warning for aspectRatio"),
            }
            // seed warning
            match &result.warnings[1] {
                aimux_core::types::Warning::Unsupported { feature, .. } => {
                    assert_eq!(feature, "seed");
                }
                _ => panic!("expected Unsupported warning for seed"),
            }
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should respect maxImagesPerCall setting
        #[tokio::test]
        async fn should_respect_max_images_per_call_setting() {
            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();

            let dall_e_2 = provider.image_model("dall-e-2").unwrap();
            assert_eq!(dall_e_2.max_images_per_call(), Some(10));

            let future_gpt = provider.image_model("gpt-image-99").unwrap();
            assert_eq!(future_gpt.max_images_per_call(), Some(10));

            let unknown = provider.image_model("unknown-model").unwrap();
            assert_eq!(unknown.max_images_per_call(), Some(1));
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should use real date when no custom date provider is specified
        #[tokio::test]
        async fn should_use_real_date_when_no_custom_date_provider_is_specified() {
            let server = MockServer::start().await;
            mock_generations_response(&server, image_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("dall-e-3").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.size = Some(Size::new(1024, 1024));

            let before = chrono::Utc::now().timestamp_millis();
            let result = model.do_generate(&opts).await.unwrap();
            let after = chrono::Utc::now().timestamp_millis();

            // A `Date`: millisecond precision.
            let ts = result.response.timestamp.unwrap().unix_millis();
            assert!(ts >= before);
            assert!(ts <= after);
            assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should include response_format for dall-e-3
        #[tokio::test]
        async fn should_include_response_format_for_dall_e_3() {
            let server = MockServer::start().await;
            mock_generations_response(&server, image_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("dall-e-3").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.size = Some(Size::new(1024, 1024));

            model.do_generate(&opts).await.unwrap();

            let requests = server.received_requests().await.expect("requests recorded");
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(body["response_format"], "b64_json");
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should map OpenAI usage to usage
        #[tokio::test]
        async fn should_map_openai_usage_to_usage() {
            let server = MockServer::start().await;
            mock_generations_response(
                &server,
                json!({
                    "created": 1733837122,
                    "data": [
                        { "b64_json": "base64-image-1" }
                    ],
                    "usage": {
                        "input_tokens": 12,
                        "output_tokens": 0,
                        "total_tokens": 12,
                        "input_tokens_details": {
                            "image_tokens": 7,
                            "text_tokens": 5
                        }
                    }
                }),
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.size = Some(Size::new(1024, 1024));

            let result = model.do_generate(&opts).await.unwrap();

            let usage = result.usage.unwrap();
            assert_eq!(usage.input_tokens, Some(12));
            assert_eq!(usage.output_tokens, Some(0));
            assert_eq!(usage.total_tokens, Some(12));

            let meta = result.provider_metadata.unwrap();
            let openai = meta.get("openai").unwrap();
            let images = openai.get("images").unwrap().as_array().unwrap();
            assert_eq!(images[0].get("imageTokens"), Some(&json!(7)));
            assert_eq!(images[0].get("textTokens"), Some(&json!(5)));
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate › should distribute input token details evenly across images
        #[tokio::test]
        async fn should_distribute_input_token_details_evenly_across_images() {
            let server = MockServer::start().await;
            mock_generations_response(
                &server,
                json!({
                    "created": 1733837122,
                    "data": [
                        { "b64_json": "base64-image-1" },
                        { "b64_json": "base64-image-2" },
                        { "b64_json": "base64-image-3" }
                    ],
                    "usage": {
                        "input_tokens": 30,
                        "output_tokens": 900,
                        "total_tokens": 930,
                        "input_tokens_details": {
                            "image_tokens": 194,
                            "text_tokens": 28
                        }
                    }
                }),
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 3;
            opts.size = Some(Size::new(1024, 1024));

            let result = model.do_generate(&opts).await.unwrap();

            let meta = result.provider_metadata.unwrap();
            let openai = meta.get("openai").unwrap();
            let images = openai.get("images").unwrap().as_array().unwrap();
            assert_eq!(images.len(), 3);
            assert_eq!(images[0].get("imageTokens"), Some(&json!(64)));
            assert_eq!(images[0].get("textTokens"), Some(&json!(9)));
            assert_eq!(images[1].get("imageTokens"), Some(&json!(64)));
            assert_eq!(images[1].get("textTokens"), Some(&json!(9)));
            assert_eq!(images[2].get("imageTokens"), Some(&json!(66)));
            assert_eq!(images[2].get("textTokens"), Some(&json!(10)));
        }
    }

    mod do_generate_image_editing {
        use super::*;

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should call /images/edits endpoint when files are provided
        #[tokio::test]
        async fn should_call_edits_endpoint_when_files_are_provided() {
            let server = MockServer::start().await;
            mock_edits_response(&server, image_edit_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.size = Some(Size::new(1024, 1024));
            opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

            model.do_generate(&opts).await.unwrap();

            let requests = server.received_requests().await.expect("requests recorded");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url.path(), "/images/edits");
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should send multiple images as form data array
        #[tokio::test]
        async fn should_send_multiple_images_as_form_data_array() {
            let server = MockServer::start().await;
            mock_edits_response(&server, image_edit_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.files = Some(vec![
                binary_file("image/png", &[137, 80, 78, 71]),
                binary_file("image/jpeg", &[255, 216, 255, 224]),
            ]);

            model.do_generate(&opts).await.unwrap();

            let requests = server.received_requests().await.expect("requests recorded");
            assert_eq!(requests.len(), 1);
            let body_str = String::from_utf8_lossy(&requests[0].body);
            assert!(
                body_str.contains("name=\"image[]\""),
                "should contain image[] field for multiple images: {body_str}"
            );
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should extract the edited images from response
        #[tokio::test]
        async fn should_extract_the_edited_images_from_response() {
            let server = MockServer::start().await;
            mock_edits_response(&server, image_edit_response_body()).await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

            let result = model.do_generate(&opts).await.unwrap();

            match &result.images {
                ImageOutputs::Base64(imgs) => {
                    assert_eq!(imgs.len(), 1);
                    assert_eq!(
                        imgs[0],
                        "iVBORw0KGgoAAAANSUhEUgAABAAAAAQACAIAAADwf7zUAAFEE2NhQlgAAUQTanVtYgAAAB5qdW1kYzJwYQARABCAAACqADibcQNj"
                    );
                }
                _ => panic!("expected Base64 images"),
            }
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should include response metadata for edited images
        #[tokio::test]
        async fn should_include_response_metadata_for_edited_images() {
            let server = MockServer::start().await;
            mock_edits_response_with_headers(
                &server,
                image_edit_response_body(),
                &[("x-request-id", "edit-request-id")],
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

            let result = model.do_generate(&opts).await.unwrap();

            assert!(result.response.timestamp.is_some());
            assert_eq!(result.response.model_id.as_deref(), Some("gpt-image-1"));
            let headers = result.response.headers.as_ref().unwrap();
            assert_eq!(
                headers.get("x-request-id").map(std::string::String::as_str),
                Some("edit-request-id")
            );
        }

        /// upstream: @ai-sdk/openai src/image/openai-image-model.test.ts › doGenerate - image editing › should return usage information for edited images
        #[tokio::test]
        async fn should_return_usage_information_for_edited_images() {
            let server = MockServer::start().await;
            mock_edits_response(
                &server,
                json!({
                    "created": 1733837122,
                    "data": [
                        { "b64_json": "edited-base64-image-1" }
                    ],
                    "usage": {
                        "input_tokens": 25,
                        "output_tokens": 0,
                        "total_tokens": 25
                    }
                }),
            )
            .await;

            let settings = OpenAIProviderSettings {
                api_key: Some("test-api-key".into()),
                base_url: Some(server.uri()),
                ..Default::default()
            };
            let provider = create_openai(settings).unwrap();
            let model = provider.image_model("gpt-image-1").unwrap();

            let mut opts = options(PROMPT);
            opts.n = 1;
            opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

            let result = model.do_generate(&opts).await.unwrap();

            let usage = result.usage.unwrap();
            assert_eq!(usage.input_tokens, Some(25));
            assert_eq!(usage.output_tokens, Some(0));
            assert_eq!(usage.total_tokens, Some(25));
        }
    }

    /// partial: `should include response data with timestamp, modelId and headers`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_include_response_data_with_timestamp_model_id_and_headers() {
        let server = MockServer::start().await;
        mock_generations_response_with_headers(
            &server,
            image_response_body(),
            &[
                ("x-request-id", "test-request-id"),
                ("x-ratelimit-remaining", "123"),
            ],
        )
        .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("dall-e-3").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        let result = model.do_generate(&opts).await.unwrap();

        assert!(result.response.timestamp.is_some());
        assert_eq!(result.response.model_id.as_deref(), Some("dall-e-3"));
        let headers = result.response.headers.as_ref().unwrap();
        assert_eq!(
            headers.get("x-request-id").map(std::string::String::as_str),
            Some("test-request-id")
        );
        assert_eq!(
            headers
                .get("x-ratelimit-remaining")
                .map(std::string::String::as_str),
            Some("123")
        );
    }

    /// partial: `should not include response_format for gpt-image-1`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_not_include_response_format_for_gpt_image_1() {
        let server = MockServer::start().await;
        mock_generations_response(&server, image_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "gpt-image-1");
        assert_eq!(body["prompt"], PROMPT);
        assert_eq!(body["n"], 1);
        assert_eq!(body["size"], "1024x1024");
        assert!(body.get("response_format").is_none());
    }

    /// partial: `should not include response_format for gpt-image-2`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_not_include_response_format_for_gpt_image_2() {
        let server = MockServer::start().await;
        mock_generations_response(&server, image_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-2").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "gpt-image-2");
        assert!(body.get("response_format").is_none());
    }

    /// partial: `should not include response_format for future gpt-image models`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_not_include_response_format_for_future_gpt_image_models() {
        let server = MockServer::start().await;
        mock_generations_response(&server, image_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-99").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "gpt-image-99");
        assert!(body.get("response_format").is_none());
    }

    /// partial: `should not include response_format for chatgpt-image-latest`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_not_include_response_format_for_chatgpt_image_latest() {
        let server = MockServer::start().await;
        mock_generations_response(&server, image_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("chatgpt-image-latest").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "chatgpt-image-latest");
        assert!(body.get("response_format").is_none());
    }

    /// partial: `should not include response_format for date-suffixed gpt-image model IDs (Azure deployment names)`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_not_include_response_format_for_date_suffixed_gpt_image() {
        let server = MockServer::start().await;
        mock_generations_response(&server, image_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1.5-2025-12-16").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["model"], "gpt-image-1.5-2025-12-16");
        assert!(body.get("response_format").is_none());
    }

    /// partial: `should handle null revised_prompt responses`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_handle_null_revised_prompt_responses() {
        let server = MockServer::start().await;
        mock_generations_response(
            &server,
            json!({
                "created": 1733837122,
                "data": [
                    {
                        "revised_prompt": null,
                        "b64_json": "base64-image-1"
                    }
                ]
            }),
        )
        .await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));

        let result = model.do_generate(&opts).await.unwrap();

        match &result.images {
            ImageOutputs::Base64(imgs) => {
                assert_eq!(imgs, &["base64-image-1".to_string()]);
            }
            _ => panic!("expected Base64 images"),
        }
        assert!(result.warnings.is_empty());

        let meta = result.provider_metadata.unwrap();
        let openai = meta.get("openai").unwrap();
        let images = openai.get("images").unwrap().as_array().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].get("created"), Some(&json!(1733837122)));
        // revisedPrompt should be absent (revised_prompt was null)
        assert!(images[0].get("revisedPrompt").is_none());
        // size, quality, background, outputFormat should be absent (not in response)
        assert!(images[0].get("size").is_none());
        assert!(images[0].get("quality").is_none());
        assert!(images[0].get("background").is_none());
        assert!(images[0].get("outputFormat").is_none());
    }

    // ════════════════════════════════════════════════════════════════════════════
    // doGenerate — image editing
    // ════════════════════════════════════════════════════════════════════════════

    /// partial: `should send image as form data with Uint8Array input`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_send_image_as_form_data_with_uint8array_input() {
        let server = MockServer::start().await;
        mock_edits_response(&server, image_edit_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));
        opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let body_str = String::from_utf8_lossy(&requests[0].body);
        assert!(
            body_str.contains("name=\"model\""),
            "should contain model field: {body_str}"
        );
        assert!(
            body_str.contains("gpt-image-1"),
            "should contain model value: {body_str}"
        );
        assert!(
            body_str.contains("name=\"prompt\""),
            "should contain prompt field: {body_str}"
        );
        assert!(
            body_str.contains(PROMPT),
            "should contain prompt value: {body_str}"
        );
        assert!(
            body_str.contains("name=\"n\""),
            "should contain n field: {body_str}"
        );
        assert!(
            body_str.contains("name=\"size\""),
            "should contain size field: {body_str}"
        );
        assert!(
            body_str.contains("1024x1024"),
            "should contain size value: {body_str}"
        );
        assert!(
            body_str.contains("name=\"image\""),
            "should contain image field: {body_str}"
        );
    }

    /// partial: `should send image as form data with base64 string input`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_send_image_as_form_data_with_base64_string_input() {
        let server = MockServer::start().await;
        mock_edits_response(&server, image_edit_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.files = Some(vec![base64_file("image/png", "iVBORw0KGgo=")]);

        model.do_generate(&opts).await.unwrap();

        let requests = server.received_requests().await.expect("requests recorded");
        assert_eq!(requests.len(), 1);
        let body_str = String::from_utf8_lossy(&requests[0].body);
        assert!(
            body_str.contains("name=\"model\""),
            "should contain model: {body_str}"
        );
        assert!(
            body_str.contains("gpt-image-1"),
            "should contain model value: {body_str}"
        );
        assert!(
            body_str.contains("name=\"image\""),
            "should contain image field: {body_str}"
        );
    }

    /// partial: `should return warnings for unsupported settings in edit mode`; full port in src/openai/tests/image/image_model.rs.
    #[tokio::test]
    async fn should_return_warnings_for_unsupported_settings_in_edit_mode() {
        let server = MockServer::start().await;
        mock_edits_response(&server, image_edit_response_body()).await;

        let settings = OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some(server.uri()),
            ..Default::default()
        };
        let provider = create_openai(settings).unwrap();
        let model = provider.image_model("gpt-image-1").unwrap();

        let mut opts = options(PROMPT);
        opts.n = 1;
        opts.size = Some(Size::new(1024, 1024));
        opts.aspect_ratio = Some(AspectRatio::new(16, 9));
        opts.seed = Some(42);
        opts.files = Some(vec![binary_file("image/png", &[137, 80, 78, 71])]);

        let result = model.do_generate(&opts).await.unwrap();

        assert_eq!(result.warnings.len(), 2);
        match &result.warnings[0] {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                assert_eq!(feature, "aspectRatio");
            }
            _ => panic!("expected Unsupported warning for aspectRatio"),
        }
        match &result.warnings[1] {
            aimux_core::types::Warning::Unsupported { feature, .. } => {
                assert_eq!(feature, "seed");
            }
            _ => panic!("expected Unsupported warning for seed"),
        }
    }

    /// File and mask support follow the model id like the AI SDK
    /// (`image/openai-image-model.ts:61-92`): known editing models `true`,
    /// `dall-e-3` `false`, anything else unknown.
    #[test]
    fn image_input_capabilities_follow_the_model_id() {
        use aimux_core::image_model::ImageModel;
        let provider = create_openai(OpenAIProviderSettings {
            api_key: Some("test-api-key".into()),
            base_url: Some("https://example.test/v1".into()),
            ..Default::default()
        })
        .unwrap();
        for (model_id, expected) in [
            ("dall-e-2", Some(true)),
            ("gpt-image-1", Some(true)),
            ("gpt-image-2.5-sunburst-2026-09-08", Some(true)),
            ("chatgpt-image-latest", Some(true)),
            ("dall-e-3", Some(false)),
            ("gpt-image-custom", None),
        ] {
            let model = provider.image(model_id);
            assert_eq!(model.supports_file_inputs(), expected, "{model_id}");
            assert_eq!(model.supports_mask_inputs(), expected, "{model_id}");
        }
    }
}
