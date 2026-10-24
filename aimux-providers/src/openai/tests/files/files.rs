use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::{FileBytes, SharedProviderOptions};

const BASE_URL: &str = "https://api.openai.com/v1";
const FILES_URL: &str = "https://api.openai.com/v1/files";

/// `prepareFileResponse({ id })`.
fn file_response(id: &str) -> Canned {
    Canned::json(&json!({
        "id": id,
        "object": "file",
        "bytes": 1024,
        "created_at": 1700000000,
        "filename": "test.csv",
        "purpose": "assistants",
        "status": "processed",
        "expires_at": null,
    }))
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

/// `{ data: { type: 'data', data: new Uint8Array([1, 2, 3]) }, mediaType, providerOptions }`.
fn upload_options(provider_options: Option<Value>) -> UploadFileCallOptions {
    UploadFileCallOptions {
        provider_options: provider_options
            .map(|value| serde_json::from_value::<SharedProviderOptions>(value).unwrap()),
        ..UploadFileCallOptions::new(
            UploadFileData::Data {
                data: FileBytes::Binary(vec![1, 2, 3]),
            },
            "application/octet-stream",
        )
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

/// `toMatchObject` on a flat object.
fn assert_matches_object(actual: &Value, expected: &Value) {
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(actual.get(key), Some(value), "key {key} in {actual}");
    }
}

mod openai_files_upload_file {
    use super::*;

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should send correct multipart request with purpose
    #[tokio::test]
    async fn should_send_correct_multipart_request_with_purpose() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        provider(&mock)
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants" } }),
            )))
            .await
            .unwrap();

        let seen = &mock.seen()[0];
        assert_eq!(seen.url, FILES_URL);
        let multipart = Value::Object(multipart_body(seen));
        assert_matches_object(&multipart, &json!({ "purpose": "assistants" }));
        assert!(multipart.get("file").is_some());
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should return providerReference with openai key
    #[tokio::test]
    async fn should_return_provider_reference_with_openai_key() {
        let mock = MockFetch::new(vec![file_response("file-xyz789")]);

        let result = provider(&mock)
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants" } }),
            )))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.provider_reference).unwrap(),
            json!({ "openai": "file-xyz789" })
        );
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should return providerMetadata from response
    #[tokio::test]
    async fn should_return_provider_metadata_from_response() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        let result = provider(&mock)
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants" } }),
            )))
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.provider_metadata).unwrap(),
            json!({
                "openai": {
                    "filename": "test.csv",
                    "purpose": "assistants",
                    "bytes": 1024,
                    "createdAt": 1700000000,
                    "status": "processed",
                },
            })
        );
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should default purpose to assistants when not provided
    #[tokio::test]
    async fn should_default_purpose_to_assistants_when_not_provided() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        provider(&mock)
            .files()
            .upload_file(&upload_options(None))
            .await
            .unwrap();

        assert_matches_object(
            &Value::Object(multipart_body(&mock.seen()[0])),
            &json!({ "purpose": "assistants" }),
        );
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should pass expires_after as bracketed multipart fields
    #[tokio::test]
    async fn should_pass_expires_after_as_bracketed_multipart_fields() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        provider(&mock)
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants", "expiresAfter": 3600 } }),
            )))
            .await
            .unwrap();

        let multipart = Value::Object(multipart_body(&mock.seen()[0]));
        assert_matches_object(
            &multipart,
            &json!({
                "expires_after[anchor]": "created_at",
                "expires_after[seconds]": "3600",
            }),
        );
        assert!(multipart.get("expires_after").is_none());
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should omit expires_after fields when no expiry is requested
    #[tokio::test]
    async fn should_omit_expires_after_fields_when_no_expiry_is_requested() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        provider(&mock)
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants" } }),
            )))
            .await
            .unwrap();

        let multipart = multipart_body(&mock.seen()[0]);
        assert!(multipart.get("expires_after[anchor]").is_none());
        assert!(multipart.get("expires_after[seconds]").is_none());
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should pass auth headers
    #[tokio::test]
    async fn should_pass_auth_headers() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);
        let provider = provider_with(
            &mock,
            OpenAIProviderSettings {
                organization: Some("test-org".into()),
                project: Some("test-project".into()),
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("custom-value".to_string()),
                )])),
                ..Default::default()
            },
        );

        provider
            .files()
            .upload_file(&upload_options(Some(
                json!({ "openai": { "purpose": "assistants" } }),
            )))
            .await
            .unwrap();

        assert_matches_object(
            &serde_json::to_value(&mock.seen()[0].headers).unwrap(),
            &json!({
                "authorization": "Bearer test-api-key",
                "openai-organization": "test-org",
                "openai-project": "test-project",
                "custom-header": "custom-value",
            }),
        );
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should handle base64 string data
    #[tokio::test]
    async fn should_handle_base64_string_data() {
        let mock = MockFetch::new(vec![file_response("file-abc123")]);

        let result = provider(&mock)
            .files()
            .upload_file(&UploadFileCallOptions {
                data: UploadFileData::Data {
                    // btoa('hello world')
                    data: FileBytes::Base64("aGVsbG8gd29ybGQ=".into()),
                },
                ..upload_options(Some(json!({ "openai": { "purpose": "assistants" } })))
            })
            .await
            .unwrap();

        assert_eq!(
            serde_json::to_value(&result.provider_reference).unwrap(),
            json!({ "openai": "file-abc123" })
        );
    }

    /// upstream: @ai-sdk/openai src/files/openai-files.test.ts › OpenAI Files - uploadFile › should set specificationVersion and provider
    #[test]
    fn should_set_specification_version_and_provider() {
        let files = provider(&MockFetch::new(vec![])).files();

        assert_eq!(files.specification_version(), "v4");
        assert_eq!(files.provider(), "openai.files");
    }
}

// Not ported (sub-feature):
// - OpenAI Files - uploadFile (stream data) › all three cases, and
//   OpenAI Files - uploadFile (pre-request rejection) › should cancel stream
//   data when provider options are invalid: `UploadFileData` has no stream
//   variant.
// - OpenAI Files - uploadFile (result fields) › should expose
//   byteSize/createdAt/expiresAt from the upload response: `UploadFileResult`
//   has no `byte_size` / `created_at` / `expires_at`.
// - OpenAI Files - getFileMetadata, downloadFile, deleteFile › all cases: the
//   `Files` trait has only `upload_file` (no `get_file_metadata`,
//   `download_file`, `delete_file`).
