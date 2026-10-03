//! Rust translations of the OpenAI Files provider tests.
//!
//! Source: `reference/ai/packages/openai/src/files/openai-files.test.ts`
//! (8 test cases).
//!
//! Each test uses `wiremock` to stub the OpenAI `/v1/files` endpoint,
//! uploads a file via `OpenAIFiles::upload_file`, and asserts on the
//! request or the returned result.

use std::collections::HashMap;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::{FileBytes, SharedProviderOptions};
use aimux_provider_utils::Resolvable;
use aimux_providers::openai::{OpenAIProvider, OpenAIProviderSettings, create_openai};

/// A native OpenAI provider against `base_url`. The key is an explicit value,
/// so the environment is never consulted.
fn provider_with(api_key: &str, base_url: impl Into<String>) -> OpenAIProvider {
    create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value(api_key.to_string())),
        base_url: Some(base_url.into()),
        ..Default::default()
    })
    .expect("settings are valid")
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// The standard successful file response body used by most tests.
fn file_response_body(id: &str) -> Value {
    json!({
        "id": id,
        "object": "file",
        "bytes": 1024,
        "created_at": 1700000000,
        "filename": "test.csv",
        "purpose": "assistants",
        "status": "processed",
        "expires_at": null
    })
}

/// Build an `OpenAIProvider` pointing at the mock server.
fn provider(server: &MockServer) -> OpenAIProvider {
    provider_with("test-api-key", server.uri())
}

/// Build `UploadFileCallOptions` with binary data and the given provider options.
fn upload_options(provider_options: Option<SharedProviderOptions>) -> UploadFileCallOptions {
    UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(vec![1, 2, 3]),
        },
        media_type: "application/octet-stream".to_string(),
        filename: None,
        provider_options,
        abort_signal: None,
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn should_send_multipart_request_with_purpose() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let mut opts = upload_options(None);
    let mut po = HashMap::new();
    po.insert("openai".to_string(), json!({ "purpose": "assistants" }));
    opts.provider_options = Some(po);

    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body_str = String::from_utf8_lossy(&requests[0].body);
    // The purpose field should be present in the multipart body.
    assert!(
        body_str.contains("assistants"),
        "multipart body should contain 'assistants': {body_str}"
    );
    // The file field should be present.
    assert!(
        body_str.contains("name=\"file\""),
        "multipart body should contain a 'file' part: {body_str}"
    );
}

#[tokio::test]
async fn should_return_provider_reference_with_openai_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-xyz789")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert("openai".to_string(), json!({ "purpose": "assistants" }));
    let opts = upload_options(Some(po));

    let result = files.upload_file(&opts).await.unwrap();

    assert_eq!(
        result.provider_reference.get("openai"),
        Some(&"file-xyz789".to_string())
    );
}

#[tokio::test]
async fn should_return_provider_metadata_from_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert("openai".to_string(), json!({ "purpose": "assistants" }));
    let opts = upload_options(Some(po));

    let result = files.upload_file(&opts).await.unwrap();

    let metadata = result.provider_metadata.expect("metadata");
    let openai = metadata.get("openai").expect("openai metadata");
    assert_eq!(openai["filename"], "test.csv");
    assert_eq!(openai["purpose"], "assistants");
    assert_eq!(openai["bytes"], 1024);
    assert_eq!(openai["createdAt"], 1700000000);
    assert_eq!(openai["status"], "processed");
}

#[tokio::test]
async fn should_default_purpose_to_assistants() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    // No provider options — purpose should default to "assistants".
    let opts = upload_options(None);
    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("assistants"),
        "default purpose should be 'assistants': {body_str}"
    );
}

#[tokio::test]
async fn should_pass_expires_after_when_provided() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert(
        "openai".to_string(),
        json!({ "purpose": "assistants", "expiresAfter": 3600 }),
    );
    let opts = upload_options(Some(po));

    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let body_str = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body_str.contains("3600"),
        "expires_after should be '3600' in multipart body: {body_str}"
    );
}

#[tokio::test]
async fn should_pass_auth_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let mut extra_headers = HashMap::new();
    extra_headers.insert("Custom-Header".to_string(), "custom-value".to_string());

    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".to_string())),
        base_url: Some(server.uri()),
        organization: Some("test-org".to_string()),
        project: Some("test-project".to_string()),
        headers: Some(
            extra_headers
                .into_iter()
                .map(|(k, v)| (k, Some(v)))
                .collect(),
        ),
        ..Default::default()
    })
    .unwrap();
    let files = provider.files();

    let mut po = HashMap::new();
    po.insert("openai".to_string(), json!({ "purpose": "assistants" }));
    let opts = upload_options(Some(po));

    files.upload_file(&opts).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(
        headers.get("authorization").and_then(|v| v.to_str().ok()),
        Some("Bearer test-api-key")
    );
    assert_eq!(
        headers
            .get("openai-organization")
            .and_then(|v| v.to_str().ok()),
        Some("test-org")
    );
    assert_eq!(
        headers.get("openai-project").and_then(|v| v.to_str().ok()),
        Some("test-project")
    );
    assert_eq!(
        headers.get("custom-header").and_then(|v| v.to_str().ok()),
        Some("custom-value")
    );
}

#[tokio::test]
async fn should_handle_base64_string_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_response_body("file-abc123")))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let files = provider.files();

    // "hello world" encoded as base64.
    let opts = UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Base64("aGVsbG8gd29ybGQ=".to_string()),
        },
        media_type: "application/octet-stream".to_string(),
        filename: None,
        provider_options: None,
        abort_signal: None,
    };

    let result = files.upload_file(&opts).await.unwrap();

    assert_eq!(
        result.provider_reference.get("openai"),
        Some(&"file-abc123".to_string())
    );
}

#[tokio::test]
async fn should_set_provider() {
    let provider = create_openai(OpenAIProviderSettings::default()).unwrap();
    assert_eq!(provider.files().provider(), "openai.files");

    let named = create_openai(OpenAIProviderSettings {
        name: Some("proxy".to_string()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(named.files().provider(), "proxy.files");
}

/// A transient 503 on the upload is reported as it happened: like the AI SDK's
/// `uploadFile`, nothing retries it, so the file body is sent once.
#[tokio::test]
async fn transient_upload_failure_is_not_retried() {
    let server = MockServer::start().await;
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let responder_attempts = std::sync::Arc::clone(&attempts);
    Mock::given(method("POST"))
        .and(path("/files"))
        .respond_with(move |_: &wiremock::Request| {
            if responder_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
                    .insert_header("retry-after-ms", "0")
                    .set_body_json(json!({"error": {"message": "try again"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(file_response_body("file-retried"))
            }
        })
        .mount(&server)
        .await;

    let provider = provider_with("test-api-key", server.uri());
    let files = provider.files();

    let error = files.upload_file(&upload_options(None)).await.unwrap_err();

    assert!(matches!(
        error,
        aimux_core::AiMuxError::ApiCall(ref detail) if detail.status_code == Some(503)
    ));
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
}
