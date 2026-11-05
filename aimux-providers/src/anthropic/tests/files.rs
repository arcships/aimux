use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;

use crate::anthropic::language_model_options::CANONICAL;
use crate::anthropic::{AnthropicFiles, AnthropicProviderSettings, create_anthropic};
use crate::test_support::mock_fetch::{Canned, MockFetch};

/// `successfulResponse.body`.
fn successful_response() -> Value {
    json!({
        "id": "file-abc123",
        "type": "file",
        "filename": "test.pdf",
        "mime_type": "application/pdf",
        "size_bytes": 12345,
        "created_at": "2025-04-14T12:00:00Z",
        "downloadable": true,
    })
}

/// `createFiles()`: the files interface of a provider with the upstream
/// base URL and API key (the provider adds `anthropic-version` itself).
fn create_files(server: &Arc<MockFetch>) -> AnthropicFiles {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some("https://api.anthropic.com/v1".into()),
        fetch: Some(server.transport()),
        ..Default::default()
    })
    .unwrap()
    .files()
}

/// `{ data: { type: 'data', data }, mediaType: 'application/octet-stream', providerOptions: {} }`.
fn upload_options(data: FileBytes) -> UploadFileCallOptions {
    UploadFileCallOptions {
        provider_options: Some(Default::default()),
        ..UploadFileCallOptions::new(UploadFileData::Data { data }, "application/octet-stream")
    }
}

mod anthropic_files {
    use super::*;

    mod upload_file {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › sends POST to /v1/files with correct beta header
        #[tokio::test]
        async fn sends_post_to_v1_files_with_correct_beta_header() {
            let server = MockFetch::new(vec![Canned::json(&successful_response())]);

            let files = create_files(&server);
            files
                .upload_file(&upload_options(FileBytes::Binary(vec![1, 2, 3])))
                .await
                .unwrap();

            let calls = server.seen();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].method, "POST");
            assert_eq!(calls[0].url, "https://api.anthropic.com/v1/files");
            assert_eq!(
                calls[0].headers.get("anthropic-beta").map(String::as_str),
                Some("files-api-2025-04-14")
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › returns providerReference with anthropic key set to file ID
        #[tokio::test]
        async fn returns_provider_reference_with_anthropic_key_set_to_file_id() {
            let server = MockFetch::new(vec![Canned::json(&successful_response())]);

            let files = create_files(&server);
            let result = files
                .upload_file(&upload_options(FileBytes::Binary(vec![1, 2, 3])))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.provider_reference).unwrap(),
                json!({ CANONICAL: "file-abc123" })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › returns providerMetadata with response data
        #[tokio::test]
        async fn returns_provider_metadata_with_response_data() {
            let server = MockFetch::new(vec![Canned::json(&successful_response())]);

            let files = create_files(&server);
            let result = files
                .upload_file(&upload_options(FileBytes::Binary(vec![1, 2, 3])))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.provider_metadata).unwrap(),
                json!({
                    CANONICAL: {
                        "filename": "test.pdf",
                        "mimeType": "application/pdf",
                        "sizeBytes": 12345,
                        "createdAt": "2025-04-14T12:00:00Z",
                        "downloadable": true,
                    },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › handles base64 string data
        #[tokio::test]
        async fn handles_base64_string_data() {
            let server = MockFetch::new(vec![Canned::json(&successful_response())]);

            let files = create_files(&server);
            let result = files
                .upload_file(&upload_options(FileBytes::Base64("AQID".into())))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.provider_reference).unwrap(),
                json!({ CANONICAL: "file-abc123" })
            );
        }
    }
}

// ── Older ports (previously `tests/anthropic_files_test.rs`) ──
//
// Rust translations of the Anthropic Files provider tests.
//
// Source: `reference/ai/packages/anthropic/src/anthropic-files.test.ts`
// (12 test cases).

mod older_files {

    use crate::anthropic::{AnthropicProvider, AnthropicProviderSettings, create_anthropic};
    use aimux_core::files_model::{UploadFileCallOptions, UploadFileData};
    use aimux_core::provider::Provider;
    use aimux_core::shared::FileBytes;
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── helpers ─────────────────────────────────────────────────────────────────

    /// The standard successful file response body used by most tests.
    fn file_response_body() -> Value {
        json!({
            "id": "file-abc123",
            "type": "file",
            "filename": "test.pdf",
            "mime_type": "application/pdf",
            "size_bytes": 12345,
            "created_at": "2025-04-14T12:00:00Z",
            "downloadable": true
        })
    }

    /// Build an `AnthropicProvider` pointing at the mock server.
    fn provider(server: &MockServer) -> AnthropicProvider {
        create_anthropic(AnthropicProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(format!("{}/v1", server.uri())),
            ..Default::default()
        })
        .unwrap()
    }

    /// Build `UploadFileCallOptions` with binary data.
    fn upload_options() -> UploadFileCallOptions {
        UploadFileCallOptions {
            data: UploadFileData::Data {
                data: FileBytes::Binary(vec![1, 2, 3]),
            },
            media_type: "application/octet-stream".to_string(),
            filename: None,
            provider_options: None,
            abort_signal: None,
        }
    }

    // ── tests ───────────────────────────────────────────────────────────────────

    mod anthropic_files {
        use super::*;

        mod upload_file {
            use super::*;

            /// partial: `sends POST to /v1/files with correct beta header`; full port in src/anthropic/tests/files.rs.
            #[tokio::test]
            async fn sends_post_to_v1_files_with_beta_header() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                files.upload_file(&upload_options()).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method.as_str(), "POST");
                assert_eq!(
                    requests[0]
                        .headers
                        .get("anthropic-beta")
                        .and_then(|v| v.to_str().ok()),
                    Some("files-api-2025-04-14")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › sends x-api-key header
            #[tokio::test]
            async fn sends_x_api_key_header() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                files.upload_file(&upload_options()).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                assert_eq!(
                    requests[0]
                        .headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok()),
                    Some("test-api-key")
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › sends multipart form data with file
            #[tokio::test]
            async fn sends_multipart_form_data_with_file() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                files.upload_file(&upload_options()).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                let body_str = String::from_utf8_lossy(&requests[0].body);
                assert!(
                    body_str.contains("name=\"file\""),
                    "multipart body should contain a 'file' part: {body_str}"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › uses default filename "blob" when not specified
            #[tokio::test]
            async fn uses_default_filename_blob_when_not_specified() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                files.upload_file(&upload_options()).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                let body_str = String::from_utf8_lossy(&requests[0].body);
                assert!(
                    body_str.contains("filename=\"blob\""),
                    "default filename should be 'blob': {body_str}"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › uses custom filename from spec options
            #[tokio::test]
            async fn uses_custom_filename_from_options() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let mut opts = upload_options();
                opts.filename = Some("custom-name.pdf".to_string());
                files.upload_file(&opts).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                let body_str = String::from_utf8_lossy(&requests[0].body);
                assert!(
                    body_str.contains("filename=\"custom-name.pdf\""),
                    "custom filename should be used: {body_str}"
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › uses mediaType from spec options
            #[tokio::test]
            async fn uses_media_type_from_options() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let mut opts = upload_options();
                opts.media_type = "application/pdf".to_string();
                files.upload_file(&opts).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                let body_str = String::from_utf8_lossy(&requests[0].body);
                assert!(
                    body_str.contains("Content-Type: application/pdf"),
                    "media type should be application/pdf: {body_str}"
                );
            }

            /// partial: `returns providerReference with anthropic key set to file ID`; full port in src/anthropic/tests/files.rs.
            #[tokio::test]
            async fn returns_provider_reference_with_anthropic_key() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let result = files.upload_file(&upload_options()).await.unwrap();

                assert_eq!(
                    result.provider_reference.get("anthropic"),
                    Some(&"file-abc123".to_string())
                );
            }

            /// partial: `returns providerMetadata with response data`; full port in src/anthropic/tests/files.rs.
            #[tokio::test]
            async fn returns_provider_metadata_with_response_data() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let result = files.upload_file(&upload_options()).await.unwrap();

                let metadata = result.provider_metadata.expect("metadata");
                let anthropic = metadata.get("anthropic").expect("anthropic metadata");
                assert_eq!(anthropic["filename"], "test.pdf");
                assert_eq!(anthropic["mimeType"], "application/pdf");
                assert_eq!(anthropic["sizeBytes"], 12345);
                assert_eq!(anthropic["createdAt"], "2025-04-14T12:00:00Z");
                assert_eq!(anthropic["downloadable"], true);
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › omits downloadable from providerMetadata when null
            #[tokio::test]
            async fn omits_downloadable_when_null() {
                let server = MockServer::start().await;
                let body = json!({
                    "id": "file-abc123",
                    "type": "file",
                    "filename": "test.pdf",
                    "mime_type": "application/pdf",
                    "size_bytes": 12345,
                    "created_at": "2025-04-14T12:00:00Z",
                    "downloadable": null
                });
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(body))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let result = files.upload_file(&upload_options()).await.unwrap();

                let metadata = result.provider_metadata.expect("metadata");
                let anthropic = metadata.get("anthropic").expect("anthropic metadata");
                assert!(
                    anthropic.get("downloadable").is_none(),
                    "downloadable should be omitted when null: {anthropic:?}"
                );
            }

            /// partial: `handles base64 string data`; full port in src/anthropic/tests/files.rs.
            #[tokio::test]
            async fn handles_base64_string_data() {
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/v1/files"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
                    .mount(&server)
                    .await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let opts = UploadFileCallOptions {
                    data: UploadFileData::Data {
                        data: FileBytes::Base64("AQID".to_string()),
                    },
                    media_type: "application/octet-stream".to_string(),
                    filename: None,
                    provider_options: None,
                    abort_signal: None,
                };

                let result = files.upload_file(&opts).await.unwrap();

                assert_eq!(
                    result.provider_reference.get("anthropic"),
                    Some(&"file-abc123".to_string())
                );
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › has specificationVersion v4
            #[tokio::test]
            async fn has_specification_version_v4() {
                let provider = create_anthropic(AnthropicProviderSettings {
                    api_key: Some("test-api-key".to_string()),
                    ..Default::default()
                })
                .unwrap();
                let files = Provider::files(&provider).unwrap();

                assert_eq!(files.specification_version(), "v4");
            }

            /// upstream: @ai-sdk/anthropic src/anthropic-files.test.ts › AnthropicFiles › uploadFile › has correct provider name
            #[tokio::test]
            async fn has_correct_provider_name() {
                let provider = create_anthropic(AnthropicProviderSettings {
                    api_key: Some("test-api-key".to_string()),
                    name: Some("anthropic.files".to_string()),
                    ..Default::default()
                })
                .unwrap();
                let files = Provider::files(&provider).unwrap();

                assert_eq!(files.provider(), "anthropic.files");
            }
        }
    }
}
