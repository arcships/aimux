use std::sync::Arc;

use serde_json::{Value, json};

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;

use crate::anthropic::options::CANONICAL;
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
