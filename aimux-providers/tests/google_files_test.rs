//! Rust translations of the Google Files provider tests.
//!
//! Source: `reference/ai/packages/google/src/google-files.test.ts`
//! (21 test cases).
//!
//! Google uses a resumable upload protocol with three phases:
//! 1. POST `/upload/v1beta/files` (init) - returns `x-goog-upload-url` header.
//! 2. POST to the upload URL - returns `{ "file": { ... } }`.
//! 3. GET `/{file.name}` (poll) - returns the file resource with updated state.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::{FileBytes, SharedProviderOptions};
use aimux_providers::{GoogleConfig, GoogleProvider};

// -- helpers -----------------------------------------------------------------

fn default_file_resource() -> Value {
    json!({
        "name": "files/abc123",
        "displayName": "test-file",
        "mimeType": "application/pdf",
        "sizeBytes": "1024",
        "createTime": "2025-01-01T00:00:00Z",
        "updateTime": "2025-01-01T00:00:00Z",
        "expirationTime": "2025-01-02T00:00:00Z",
        "sha256Hash": "abc123hash",
        "uri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
        "state": "ACTIVE"
    })
}

fn provider(server: &MockServer) -> GoogleProvider {
    let config =
        GoogleConfig::new("test-api-key").with_base_url(format!("{}/v1beta", server.uri()));
    GoogleProvider::new(config)
}

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

/// Mount the standard init + upload mocks for a successful upload with an
/// immediately-ACTIVE file.
async fn mount_success_mocks(server: &MockServer) {
    let upload_url = format!("{}/resume", server.uri());

    // Init endpoint.
    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url))
        .mount(server)
        .await;

    // Upload endpoint - returns file resource with ACTIVE state.
    Mock::given(method("POST"))
        .and(path("/resume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "file": default_file_resource()
        })))
        .mount(server)
        .await;
}

// -- constructor tests -------------------------------------------------------

#[tokio::test]
async fn should_expose_correct_provider_and_specification_version() {
    let provider = GoogleProvider::new(GoogleConfig::new("test-api-key"));
    let files = provider.files();
    assert_eq!(files.provider(), "google.generative-ai");
    assert_eq!(files.specification_version(), "v4");
}

// -- upload initiation tests -------------------------------------------------

#[tokio::test]
async fn should_send_correct_headers_for_resumable_upload_initiation() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1, 2, 3]),
        },
        media_type: "application/pdf".to_string(),
        abort_signal: None,
        ..upload_options()
    };
    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    // First request should be the init request.
    let init_req = &requests[0];
    assert_eq!(init_req.method.as_str(), "POST");
    let headers = &init_req.headers;
    assert_eq!(
        headers
            .get("x-goog-upload-protocol")
            .and_then(|v| v.to_str().ok()),
        Some("resumable")
    );
    assert_eq!(
        headers
            .get("x-goog-upload-command")
            .and_then(|v| v.to_str().ok()),
        Some("start")
    );
    assert_eq!(
        headers
            .get("x-goog-upload-header-content-length")
            .and_then(|v| v.to_str().ok()),
        Some("3")
    );
    assert_eq!(
        headers
            .get("x-goog-upload-header-content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/pdf")
    );
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()),
        Some("test-api-key")
    );
}

#[tokio::test]
async fn should_include_display_name_in_initiation_body_when_provided() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert(
        "google".to_string(),
        json!({ "displayName": "my-document" }),
    );
    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1]),
        },
        media_type: "text/plain".to_string(),
        filename: None,
        provider_options: Some(po),
        abort_signal: None,
    };
    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body, json!({ "file": { "display_name": "my-document" } }));
}

#[tokio::test]
async fn should_omit_display_name_from_body_when_not_provided() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    files.upload_file(&upload_options()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body, json!({ "file": {} }));
}

// -- upload data tests -------------------------------------------------------

#[tokio::test]
async fn should_send_file_data_to_upload_url_with_correct_headers() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![10, 20, 30]),
        },
        abort_signal: None,
        ..upload_options()
    };
    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    // Second request should be the upload request.
    let upload_req = &requests[1];
    assert_eq!(upload_req.method.as_str(), "POST");
    let headers = &upload_req.headers;
    assert_eq!(
        headers.get("content-length").and_then(|v| v.to_str().ok()),
        Some("3")
    );
    assert_eq!(
        headers
            .get("x-goog-upload-offset")
            .and_then(|v| v.to_str().ok()),
        Some("0")
    );
    assert_eq!(
        headers
            .get("x-goog-upload-command")
            .and_then(|v| v.to_str().ok()),
        Some("upload, finalize")
    );
    assert_eq!(&upload_req.body, &[10, 20, 30]);
}

// -- result tests ------------------------------------------------------------

#[tokio::test]
async fn should_return_provider_reference_with_google_key_set_to_file_uri() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert_eq!(
        result.provider_reference.get("google"),
        Some(&"https://generativelanguage.googleapis.com/v1beta/files/abc123".to_string())
    );
}

#[tokio::test]
async fn should_return_empty_warnings_when_filename_not_provided() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert!(result.warnings.is_empty());
}

#[tokio::test]
async fn should_return_unsupported_warning_when_filename_provided() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let mut opts = upload_options();
    opts.filename = Some("test.pdf".to_string());
    let result = files.upload_file(&opts).await.unwrap();

    assert_eq!(result.warnings.len(), 1);
    match &result.warnings[0] {
        aimux_core::types::Warning::Unsupported { feature, .. } => {
            assert_eq!(feature, "filename");
        }
        other => panic!("expected Unsupported warning, got {other:?}"),
    }
}

#[tokio::test]
async fn should_return_provider_metadata_with_file_details() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    let metadata = result.provider_metadata.expect("metadata");
    let google = metadata.get("google").expect("google metadata");
    assert_eq!(google["name"], "files/abc123");
    assert_eq!(google["displayName"], "test-file");
    assert_eq!(google["mimeType"], "application/pdf");
    assert_eq!(google["sizeBytes"], "1024");
    assert_eq!(google["state"], "ACTIVE");
    assert_eq!(
        google["uri"],
        "https://generativelanguage.googleapis.com/v1beta/files/abc123"
    );
    assert_eq!(google["createTime"], "2025-01-01T00:00:00Z");
    assert_eq!(google["updateTime"], "2025-01-01T00:00:00Z");
    assert_eq!(google["expirationTime"], "2025-01-02T00:00:00Z");
    assert_eq!(google["sha256Hash"], "abc123hash");
}

#[tokio::test]
async fn should_handle_base64_string_data() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    // "hello" encoded as base64.
    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Base64("aGVsbG8=".to_string()),
        },
        abort_signal: None,
        ..upload_options()
    };
    let result = files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    // The upload request body should be the decoded bytes.
    assert_eq!(&requests[1].body, b"hello");

    assert!(result.provider_reference.contains_key("google"));
}

// -- polling tests -----------------------------------------------------------

#[tokio::test]
async fn should_not_poll_when_file_is_immediately_active() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    files.upload_file(&upload_options()).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    // Only 2 requests: init + upload. No poll requests.
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn should_throw_when_file_state_is_failed() {
    let server = MockServer::start().await;
    let upload_url = format!("{}/resume", server.uri());

    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url))
        .mount(&server)
        .await;

    let failed_resource = {
        let mut r = default_file_resource();
        r["state"] = json!("FAILED");
        r
    };
    Mock::given(method("POST"))
        .and(path("/resume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "file": failed_resource
        })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("File processing failed"),
        "error should mention 'File processing failed': {err}"
    );
}

// -- error handling tests ----------------------------------------------------

#[tokio::test]
async fn should_throw_when_initiation_request_fails() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Init error"))
        .mount(&server)
        .await;

    // A persistent failure exhausts retries either way; pin max_retries to 0
    // so this test (which only checks the message text, not retry
    // exhaustion) stays fast.
    let config = GoogleConfig::new("test-api-key")
        .with_base_url(format!("{}/v1beta", server.uri()))
        .with_retry_config(aimux_core::retry::RetryConfig {
            max_retries: 0,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string()
            .contains("Failed to initiate resumable upload"),
        "error should mention 'Failed to initiate': {err}"
    );
}

#[tokio::test]
async fn should_throw_when_no_upload_url_returned() {
    let server = MockServer::start().await;

    // Return 200 but without the x-goog-upload-url header.
    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("No upload URL"),
        "error should mention 'No upload URL': {err}"
    );
}

#[tokio::test]
async fn should_throw_when_upload_request_fails() {
    let server = MockServer::start().await;
    let upload_url = format!("{}/resume", server.uri());

    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/resume"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Upload error"))
        .mount(&server)
        .await;

    // A persistent failure exhausts retries either way; pin max_retries to 0
    // so this test (which only checks the message text, not retry
    // exhaustion) stays fast.
    let config = GoogleConfig::new("test-api-key")
        .with_base_url(format!("{}/v1beta", server.uri()))
        .with_retry_config(aimux_core::retry::RetryConfig {
            max_retries: 0,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Failed to upload file data"),
        "error should mention 'Failed to upload file data': {err}"
    );
}

// -- provider options tests --------------------------------------------------

#[tokio::test]
async fn should_accept_valid_provider_options() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert(
        "google".to_string(),
        json!({ "displayName": "test", "pollIntervalMs": 5000, "pollTimeoutMs": 60000 }),
    );
    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1]),
        },
        media_type: "text/plain".to_string(),
        filename: None,
        provider_options: Some(po),
        abort_signal: None,
    };

    let result = files.upload_file(&opts).await.unwrap();

    assert!(result.provider_reference.contains_key("google"));
}

#[tokio::test]
async fn should_work_without_provider_options() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert!(result.provider_reference.contains_key("google"));
}

#[tokio::test]
async fn should_pass_through_unknown_properties() {
    let server = MockServer::start().await;
    mount_success_mocks(&server).await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po: SharedProviderOptions = HashMap::new();
    po.insert(
        "google".to_string(),
        json!({ "customField": "custom-value" }),
    );
    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1]),
        },
        media_type: "text/plain".to_string(),
        filename: None,
        provider_options: Some(po),
        abort_signal: None,
    };

    let result = files.upload_file(&opts).await.unwrap();

    assert!(result.provider_reference.contains_key("google"));
}

// -- response metadata tests -------------------------------------------------

#[tokio::test]
async fn should_omit_optional_fields_from_metadata_when_not_present() {
    let server = MockServer::start().await;
    let upload_url = format!("{}/resume", server.uri());

    let minimal_resource = json!({
        "name": "files/minimal",
        "mimeType": "text/plain",
        "uri": "https://generativelanguage.googleapis.com/v1beta/files/minimal",
        "state": "ACTIVE"
    });

    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/resume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "file": minimal_resource })))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    let metadata = result.provider_metadata.expect("metadata");
    let google = metadata.get("google").expect("google metadata");
    assert_eq!(google["name"], "files/minimal");
    assert_eq!(google["displayName"], Value::Null);
    assert_eq!(google["mimeType"], "text/plain");
    assert_eq!(google["sizeBytes"], Value::Null);
    assert_eq!(google["state"], "ACTIVE");
    assert_eq!(
        google["uri"],
        "https://generativelanguage.googleapis.com/v1beta/files/minimal"
    );
    assert!(google.get("createTime").is_none());
    assert!(google.get("expirationTime").is_none());
    assert!(google.get("sha256Hash").is_none());
}

/// A transient 503 on the upload stage followed by 200 must succeed, and
/// must not re-run the init stage — a retried upload reuses the
/// already-minted `upload_url` instead of requesting a new one.
#[tokio::test]
async fn transient_upload_failure_is_retried_without_re_initiating() {
    let server = MockServer::start().await;
    let upload_url = format!("{}/resume", server.uri());

    let init_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let init_observed = std::sync::Arc::clone(&init_attempts);
    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(move |_: &wiremock::Request| {
            init_observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).insert_header("x-goog-upload-url", upload_url.as_str())
        })
        .mount(&server)
        .await;

    let upload_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upload_observed = std::sync::Arc::clone(&upload_attempts);
    Mock::given(method("POST"))
        .and(path("/resume"))
        .respond_with(move |_: &wiremock::Request| {
            if upload_observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({"error": {"message": "try again"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({ "file": default_file_resource() }))
            }
        })
        .mount(&server)
        .await;

    let config = GoogleConfig::new("test-api-key")
        .with_base_url(format!("{}/v1beta", server.uri()))
        .with_retry_config(aimux_core::retry::RetryConfig {
            max_retries: 1,
            ..Default::default()
        });
    let provider = GoogleProvider::new(config);
    let files = provider.files();

    let result = files.upload_file(&upload_options()).await.unwrap();

    assert_eq!(
        result.provider_reference.get("google"),
        Some(&"https://generativelanguage.googleapis.com/v1beta/files/abc123".to_string())
    );
    assert_eq!(
        init_attempts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the init stage must not be replayed by an upload-stage retry"
    );
    assert_eq!(upload_attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}
