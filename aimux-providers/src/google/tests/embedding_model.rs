use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};
use aimux_core::shared::SharedProviderOptions;

use crate::google::{GoogleEmbeddingModel, GoogleProviderSettings, create_google};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// The pinned user agent (upstream mocks `0.0.0-test`).
const PINNED_USER_AGENT: &str = "ai-sdk-google/4.0.87 ai-sdk-provider-utils/5.0.53";

const TEST_VALUES: [&str; 2] = ["sunny day at the beach", "rainy day in the city"];

fn dummy_embeddings() -> Value {
    json!([[0.1, 0.2, 0.3, 0.4, 0.5], [0.6, 0.7, 0.8, 0.9, 1]])
}

/// `prepareBatchJsonResponse`, the body as the test server writes it.
fn batch_json_response(headers: &[(&str, &str)]) -> Canned {
    let body = json!({
        "embeddings": dummy_embeddings()
            .as_array()
            .unwrap()
            .iter()
            .map(|embedding| json!({"values": embedding}))
            .collect::<Vec<_>>(),
    });
    let mut canned = Canned::json(&body);
    canned.headers.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
    );
    canned
}

/// `prepareSingleJsonResponse`.
fn single_json_response() -> Canned {
    Canned::json(&json!({"embedding": {"values": dummy_embeddings()[0]}}))
}

/// `createGoogle({ apiKey: 'test-api-key' })` with the mock transport.
fn embedding_model(mock: &Arc<MockFetch>, model_id: &str) -> GoogleEmbeddingModel {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .unwrap()
    .embedding(model_id)
}

/// The shared `model` (`gemini-embedding-001`).
fn model(mock: &Arc<MockFetch>) -> GoogleEmbeddingModel {
    embedding_model(mock, "gemini-embedding-001")
}

/// The shared `multimodalModel` (`gemini-embedding-2-preview`).
fn multimodal_model(mock: &Arc<MockFetch>) -> GoogleEmbeddingModel {
    embedding_model(mock, "gemini-embedding-2-preview")
}

fn embed_options(values: &[&str]) -> EmbeddingCallOptions {
    EmbeddingCallOptions {
        values: values.iter().map(ToString::to_string).collect(),
        abort_signal: None,
        provider_options: None,
        headers: None,
        max_retries: None,
        timeout: None,
    }
}

fn google_options(value: Value) -> Option<SharedProviderOptions> {
    let Value::Object(map) = value else {
        panic!("options must be an object")
    };
    Some([("google".to_string(), map)].into_iter().collect())
}

mod google_embedding_model {
    use super::*;

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should expose the raw response
    #[tokio::test]
    async fn should_expose_the_raw_response() {
        // The test server writes the JSON body (80 bytes) with its length.
        let mut canned = batch_json_response(&[("test-header", "test-value")]);
        canned.body =
            br#"{"embeddings":[{"values":[0.1,0.2,0.3,0.4,0.5]},{"values":[0.6,0.7,0.8,0.9,1]}]}"#
                .to_vec();
        canned
            .headers
            .push(("content-length".into(), canned.body.len().to_string()));
        let mock = MockFetch::new(vec![canned]);

        let result = model(&mock)
            .do_embed(&embed_options(&TEST_VALUES))
            .await
            .unwrap();

        let response = result.response.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(response.headers.as_ref().unwrap()).unwrap(),
            json!({
                "content-length": "80",
                "content-type": "application/json",
                "test-header": "test-value",
            })
        );
        let snapshot: Value = serde_json::from_str(include_str!(
            "snapshots/embedding_expose_the_raw_response.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), snapshot);
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should pass the model and the values
    #[tokio::test]
    async fn should_pass_the_model_and_the_values() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        model(&mock)
            .do_embed(&embed_options(&TEST_VALUES))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {"parts": [{"text": "sunny day at the beach"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                    },
                    {
                        "content": {"parts": [{"text": "rainy day in the city"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should pass the outputDimensionality setting
    #[tokio::test]
    async fn should_pass_the_output_dimensionality_setting() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({"outputDimensionality": 64}));
        model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {"parts": [{"text": "sunny day at the beach"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                        "outputDimensionality": 64,
                    },
                    {
                        "content": {"parts": [{"text": "rainy day in the city"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                        "outputDimensionality": 64,
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should pass the taskType setting
    #[tokio::test]
    async fn should_pass_the_task_type_setting() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({"taskType": "SEMANTIC_SIMILARITY"}));
        model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {"parts": [{"text": "sunny day at the beach"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                        "taskType": "SEMANTIC_SIMILARITY",
                    },
                    {
                        "content": {"parts": [{"text": "rainy day in the city"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                        "taskType": "SEMANTIC_SIMILARITY",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should pass headers
    #[tokio::test]
    async fn should_pass_headers() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);
        let provider = create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".into()),
            headers: Some(
                [(
                    "Custom-Provider-Header".to_string(),
                    Some("provider-header-value".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
            fetch: Some(mock.transport()),
            ..Default::default()
        })
        .unwrap();

        let mut options = embed_options(&TEST_VALUES);
        options.headers = Some(
            [(
                "Custom-Request-Header".to_string(),
                "request-header-value".to_string(),
            )]
            .into_iter()
            .collect(),
        );
        provider
            .embedding("gemini-embedding-001")
            .do_embed(&options)
            .await
            .unwrap();

        // The test server reports the user agent separately.
        let mut headers = mock.seen()[0].headers.clone();
        let user_agent = headers.remove("user-agent").unwrap_or_default();
        assert_eq!(
            serde_json::to_value(&headers).unwrap(),
            json!({
                "content-type": "application/json",
                "custom-provider-header": "provider-header-value",
                "custom-request-header": "request-header-value",
                "x-goog-api-key": "test-api-key",
            })
        );
        assert!(user_agent.contains(PINNED_USER_AGENT), "{user_agent}");
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should throw an error if too many values are provided
    #[tokio::test]
    async fn should_throw_an_error_if_too_many_values_are_provided() {
        let mock = MockFetch::new(vec![]);
        let model = model(&mock);

        let too_many_values = vec!["test"; 101];
        let error = model
            .do_embed(&embed_options(&too_many_values))
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains(
                "Too many values for a single embedding call. The google.generative-ai model \"gemini-embedding-001\" can only embed up to 100 values per call, but 101 values were provided."
            ),
            "{error}"
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should use the batch embeddings endpoint
    #[tokio::test]
    async fn should_use_the_batch_embeddings_endpoint() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        model(&mock)
            .do_embed(&embed_options(&TEST_VALUES))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:batchEmbedContents"
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should use the single embeddings endpoint
    #[tokio::test]
    async fn should_use_the_single_embeddings_endpoint() {
        let mock = MockFetch::new(vec![single_json_response()]);

        model(&mock)
            .do_embed(&embed_options(&TEST_VALUES[..1]))
            .await
            .unwrap();

        assert_eq!(
            mock.seen()[0].url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent"
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should merge multimodal content for single embedding
    #[tokio::test]
    async fn should_merge_multimodal_content_for_single_embedding() {
        let mock = MockFetch::new(vec![single_json_response()]);

        let mut options = embed_options(&TEST_VALUES[..1]);
        options.provider_options = google_options(json!({
            "content": [[{"inlineData": {"mimeType": "image/png", "data": "abc123"}}]],
        }));
        model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "content": {
                    "parts": [
                        {"text": "sunny day at the beach"},
                        {"inlineData": {"data": "abc123", "mimeType": "image/png"}},
                    ],
                },
                "model": "models/gemini-embedding-001",
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should merge per-value multimodal content for batch embedding
    #[tokio::test]
    async fn should_merge_per_value_multimodal_content_for_batch_embedding() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({
            "content": [
                [{"inlineData": {"mimeType": "image/png", "data": "img1"}}],
                [{"inlineData": {"mimeType": "image/jpeg", "data": "img2"}}],
            ],
        }));
        model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {
                            "parts": [
                                {"text": "sunny day at the beach"},
                                {"inlineData": {"data": "img1", "mimeType": "image/png"}},
                            ],
                            "role": "user",
                        },
                        "model": "models/gemini-embedding-001",
                    },
                    {
                        "content": {
                            "parts": [
                                {"text": "rainy day in the city"},
                                {"inlineData": {"data": "img2", "mimeType": "image/jpeg"}},
                            ],
                            "role": "user",
                        },
                        "model": "models/gemini-embedding-001",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should handle null entries as text-only in batch embedding
    #[tokio::test]
    async fn should_handle_null_entries_as_text_only_in_batch_embedding() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({
            "content": [[{"inlineData": {"mimeType": "image/png", "data": "img1"}}], null],
        }));
        model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {
                            "parts": [
                                {"text": "sunny day at the beach"},
                                {"inlineData": {"data": "img1", "mimeType": "image/png"}},
                            ],
                            "role": "user",
                        },
                        "model": "models/gemini-embedding-001",
                    },
                    {
                        "content": {"parts": [{"text": "rainy day in the city"}], "role": "user"},
                        "model": "models/gemini-embedding-001",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should merge fileData content for single embedding
    #[tokio::test]
    async fn should_merge_file_data_content_for_single_embedding() {
        let mock = MockFetch::new(vec![single_json_response()]);

        let mut options = embed_options(&TEST_VALUES[..1]);
        options.provider_options = google_options(json!({
            "content": [[{"fileData": {"fileUri": "gs://bucket/video.mp4", "mimeType": "video/mp4"}}]],
        }));
        multimodal_model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "content": {
                    "parts": [
                        {"text": "sunny day at the beach"},
                        {"fileData": {"fileUri": "gs://bucket/video.mp4", "mimeType": "video/mp4"}},
                    ],
                },
                "model": "models/gemini-embedding-2-preview",
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should merge fileData content for batch embedding
    #[tokio::test]
    async fn should_merge_file_data_content_for_batch_embedding() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({
            "content": [
                [{"fileData": {"fileUri": "gs://bucket/video.mp4", "mimeType": "video/mp4"}}],
                null,
            ],
        }));
        multimodal_model(&mock).do_embed(&options).await.unwrap();

        assert_eq!(
            mock.seen()[0].json_body(),
            json!({
                "requests": [
                    {
                        "content": {
                            "parts": [
                                {"text": "sunny day at the beach"},
                                {"fileData": {"fileUri": "gs://bucket/video.mp4", "mimeType": "video/mp4"}},
                            ],
                            "role": "user",
                        },
                        "model": "models/gemini-embedding-2-preview",
                    },
                    {
                        "content": {"parts": [{"text": "rainy day in the city"}], "role": "user"},
                        "model": "models/gemini-embedding-2-preview",
                    },
                ],
            })
        );
    }

    /// upstream: @ai-sdk/google src/google-embedding-model.test.ts › GoogleEmbeddingModel › should throw error when content length does not match values length
    #[tokio::test]
    async fn should_throw_error_when_content_length_does_not_match_values_length() {
        let mock = MockFetch::new(vec![batch_json_response(&[])]);

        let mut options = embed_options(&TEST_VALUES);
        options.provider_options = google_options(json!({
            "content": [[{"inlineData": {"mimeType": "image/png", "data": "img1"}}]],
        }));
        let error = model(&mock).do_embed(&options).await.unwrap_err();

        assert!(
            error.to_string().contains(
                "The number of multimodal content entries (1) must match the number of values (2)."
            ),
            "{error}"
        );
    }
}
