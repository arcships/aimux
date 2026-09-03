//! Rust translations of the Anthropic Files provider tests.
//!
//! Source: `reference/ai/packages/anthropic/src/anthropic-files.test.ts`
//! (12 test cases).

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;
use aimux_providers::{AnthropicConfig, AnthropicProvider};

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
    let config = AnthropicConfig::new("test-api-key").with_base_url(server.uri());
    AnthropicProvider::new(config)
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

#[tokio::test]
async fn sends_post_to_v1_files_with_beta_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

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

#[tokio::test]
async fn sends_x_api_key_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

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

#[tokio::test]
async fn sends_multipart_form_data_with_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    files.upload_file(&upload_options()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("name=\"file\""),
        "multipart body should contain a 'file' part: {body_str}"
    );
}

#[tokio::test]
async fn uses_default_filename_blob_when_not_specified() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    files.upload_file(&upload_options()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("filename=\"blob\""),
        "default filename should be 'blob': {body_str}"
    );
}

#[tokio::test]
async fn uses_custom_filename_from_options() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

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

#[tokio::test]
async fn uses_media_type_from_options() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

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

#[tokio::test]
async fn returns_provider_reference_with_anthropic_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert_eq!(
        result.provider_reference.get("anthropic"),
        Some(&"file-abc123".to_string())
    );
}

#[tokio::test]
async fn returns_provider_metadata_with_response_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    let metadata = result.provider_metadata.expect("metadata");
    let anthropic = metadata.get("anthropic").expect("anthropic metadata");
    assert_eq!(anthropic["filename"], "test.pdf");
    assert_eq!(anthropic["mimeType"], "application/pdf");
    assert_eq!(anthropic["sizeBytes"], 12345);
    assert_eq!(anthropic["createdAt"], "2025-04-14T12:00:00Z");
    assert_eq!(anthropic["downloadable"], true);
}

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
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    let metadata = result.provider_metadata.expect("metadata");
    let anthropic = metadata.get("anthropic").expect("anthropic metadata");
    assert!(
        anthropic.get("downloadable").is_none(),
        "downloadable should be omitted when null: {anthropic}"
    );
}

#[tokio::test]
async fn handles_base64_string_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body()))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

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

#[tokio::test]
async fn has_specification_version_v4() {
    let provider = AnthropicProvider::new(AnthropicConfig::new("test-api-key"));
    let files = provider.files();

    assert_eq!(files.specification_version(), "v4");
}

#[tokio::test]
async fn has_correct_provider_name() {
    let provider = AnthropicProvider::new(AnthropicConfig::new("test-api-key"));
    let files = provider.files();

    assert_eq!(files.provider(), "anthropic.files");
}

/// A transient 503 followed by 200 must succeed: `upload_file` retries the
/// upload exchange using the provider's configured retry settings, the same
/// way `list_models` already does.
#[tokio::test]
async fn transient_failure_is_retried_and_succeeds() {
    let server = MockServer::start().await;
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let responder_attempts = std::sync::Arc::clone(&attempts);
    Mock::given(method("POST"))
        .and(path("/v1/files"))
        .respond_with(move |_: &wiremock::Request| {
            if responder_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({"error": {"message": "try again"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(file_response_body())
            }
        })
        .mount(&server)
        .await;

    let config = AnthropicConfig::new("test-api-key")
        .with_base_url(server.uri())
        .with_retry_config(aimux_core::retry::RetryConfig {
            max_retries: 1,
            ..Default::default()
        });
    let provider = AnthropicProvider::new(config);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert_eq!(
        result.provider_reference.get("anthropic"),
        Some(&"file-abc123".to_string())
    );
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}
