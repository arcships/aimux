//! `GoogleFiles` (`google-files.test.ts`).
//!
//! Upstream scripts one `fetch` mock that answers the resumable-upload
//! initiation, the upload URL and the polls. aimux sends the upload itself
//! through its guarded download path (never through an injected transport),
//! so the script runs on a local `wiremock` server instead: the base URL is
//! `{server}/v1beta` and the upload URL is a path on the same server, where
//! upstream uses `generativelanguage.googleapis.com` and
//! `upload.example.com`. The provider's `x-goog-api-key` header comes from
//! `api_key`.
//!
//! The other cases of this file keep their ports in
//! `aimux-providers/tests/google_files_test.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use aimux_core::AbortSignal;
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;

use crate::google::{GoogleFiles, GoogleProviderSettings, create_google};

/// The path of the base URL (`{server}/v1beta`); requests are compared by
/// path, the origin being the local server's.
const BASE_PATH: &str = "/v1beta/";

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
        "state": "ACTIVE",
    })
}

fn with_state(resource: &Value, state: &str) -> Value {
    let mut resource = resource.clone();
    resource["state"] = json!(state);
    resource
}

/// `createMockFiles` options.
struct MockFilesOptions {
    file_resource: Value,
    upload_path: &'static str,
    init_status: u16,
    upload_status: u16,
    poll_responses: Vec<&'static str>,
}

impl Default for MockFilesOptions {
    fn default() -> Self {
        Self {
            file_resource: default_file_resource(),
            upload_path: "/resume",
            init_status: 200,
            upload_status: 200,
            poll_responses: Vec::new(),
        }
    }
}

/// The poll endpoint: the n-th poll answers `poll_responses[n]` (the upload
/// answered index 0), then the resource's own state.
struct PollResponder {
    file_resource: Value,
    poll_responses: Vec<&'static str>,
    poll_index: AtomicUsize,
}

impl Respond for PollResponder {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let poll_index = self.poll_index.fetch_add(1, Ordering::SeqCst) + 1;
        let state = self
            .poll_responses
            .get(poll_index)
            .copied()
            .unwrap_or_else(|| self.file_resource["state"].as_str().unwrap());
        ResponseTemplate::new(200).set_body_json(with_state(&self.file_resource, state))
    }
}

struct MockFiles {
    server: MockServer,
    files: GoogleFiles,
}

impl MockFiles {
    async fn requests(&self) -> Vec<Request> {
        self.server.received_requests().await.unwrap()
    }

    /// The requests sent to the upload URL.
    async fn upload_requests(&self, upload_path: &str) -> Vec<Request> {
        self.requests()
            .await
            .into_iter()
            .filter(|request| request.url.path() == upload_path)
            .collect()
    }

    /// The requests sent to the initiation endpoint.
    async fn init_requests(&self) -> Vec<Request> {
        self.requests()
            .await
            .into_iter()
            .filter(|request| request.url.path().contains("/upload/v1beta/files"))
            .collect()
    }

    /// The requests under the base URL (the polls).
    async fn poll_requests(&self) -> Vec<Request> {
        self.requests()
            .await
            .into_iter()
            .filter(|request| request.url.path().starts_with(BASE_PATH))
            .collect()
    }
}

fn files_for(server: &MockServer) -> GoogleFiles {
    create_google(GoogleProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some(format!("{}/v1beta", server.uri())),
        ..Default::default()
    })
    .expect("valid settings")
    .files()
}

async fn create_mock_files(options: MockFilesOptions) -> MockFiles {
    // A server of its own, not one from wiremock's pool: the abort tests drop
    // the client mid-upload, and a late upload byte must not reach the next
    // test that borrows the same pooled server.
    let server = MockServer::builder().start().await;
    let upload_url = format!("{}{}", server.uri(), options.upload_path);

    let init = if options.init_status == 200 {
        ResponseTemplate::new(200).insert_header("x-goog-upload-url", upload_url.as_str())
    } else {
        ResponseTemplate::new(options.init_status).set_body_string("Init error")
    };
    Mock::given(method("POST"))
        .and(path("/upload/v1beta/files"))
        .respond_with(init)
        .mount(&server)
        .await;

    let upload = if options.upload_status == 200 {
        let state = options
            .poll_responses
            .first()
            .copied()
            .unwrap_or_else(|| options.file_resource["state"].as_str().unwrap());
        ResponseTemplate::new(200)
            .set_body_json(json!({ "file": with_state(&options.file_resource, state) }))
    } else {
        ResponseTemplate::new(options.upload_status).set_body_string("Upload error")
    };
    Mock::given(method("POST"))
        .and(path(options.upload_path))
        .respond_with(upload)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .respond_with(PollResponder {
            file_resource: options.file_resource.clone(),
            poll_responses: options.poll_responses.clone(),
            poll_index: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;

    let files = files_for(&server);
    MockFiles { server, files }
}

fn upload_options(
    data: Vec<u8>,
    media_type: &str,
    provider_options: Option<Value>,
) -> UploadFileCallOptions {
    UploadFileCallOptions {
        data: UploadFileData::Data {
            data: FileBytes::Binary(data),
        },
        media_type: media_type.to_string(),
        filename: None,
        provider_options: provider_options
            .map(|value| serde_json::from_value(value).expect("provider options deserialize")),
        abort_signal: None,
    }
}

mod google_files {
    use super::*;

    mod upload_file {
        use super::*;

        /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should include displayName in initiation body when provided
        #[tokio::test]
        async fn should_include_display_name_in_initiation_body_when_provided() {
            let mock = create_mock_files(MockFilesOptions::default()).await;

            mock.files
                .upload_file(&upload_options(
                    vec![1],
                    "text/plain",
                    Some(json!({ "google": { "displayName": "my-document" } })),
                ))
                .await
                .unwrap();

            let init = mock.init_requests().await;
            let body: Value = serde_json::from_slice(&init[0].body).unwrap();
            assert_eq!(body, json!({ "file": { "display_name": "my-document" } }));
        }

        /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should send ArrayBuffer-backed data and let fetch derive its content length
        #[tokio::test]
        async fn should_send_array_buffer_backed_data_and_let_fetch_derive_its_content_length() {
            // Not asserted: that the code passes no `Content-Length` to fetch
            // (the server sees the wire request, whose length the HTTP client
            // derives) and the JS buffer types.
            let mock = create_mock_files(MockFilesOptions {
                upload_path: "/resume-session",
                ..Default::default()
            })
            .await;

            mock.files
                .upload_file(&upload_options(
                    vec![10, 20, 30],
                    "application/octet-stream",
                    Some(json!({})),
                ))
                .await
                .unwrap();

            let upload = mock.upload_requests("/resume-session").await;
            assert_eq!(upload.len(), 1);
            assert_eq!(upload[0].method.as_str(), "POST");
            let header = |name: &str| {
                upload[0]
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            assert_eq!(header("x-goog-upload-offset").as_deref(), Some("0"));
            assert_eq!(
                header("x-goog-upload-command").as_deref(),
                Some("upload, finalize")
            );
            assert_eq!(upload[0].body, vec![10, 20, 30]);
        }

        /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should return unsupported warning when filename is provided
        #[tokio::test]
        async fn should_return_unsupported_warning_when_filename_is_provided() {
            let mock = create_mock_files(MockFilesOptions::default()).await;

            let result = mock
                .files
                .upload_file(&UploadFileCallOptions {
                    filename: Some("test.pdf".into()),
                    ..upload_options(vec![1], "application/octet-stream", Some(json!({})))
                })
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.warnings).unwrap(),
                json!([{ "type": "unsupported", "feature": "filename" }])
            );
        }

        /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should return providerMetadata with file details
        #[tokio::test]
        async fn should_return_provider_metadata_with_file_details() {
            let mock = create_mock_files(MockFilesOptions::default()).await;

            let result = mock
                .files
                .upload_file(&upload_options(
                    vec![1],
                    "application/octet-stream",
                    Some(json!({})),
                ))
                .await
                .unwrap();

            assert_eq!(
                serde_json::to_value(&result.provider_metadata).unwrap(),
                json!({
                    "google": {
                        "name": "files/abc123",
                        "displayName": "test-file",
                        "mimeType": "application/pdf",
                        "sizeBytes": "1024",
                        "state": "ACTIVE",
                        "uri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                        "createTime": "2025-01-01T00:00:00Z",
                        "updateTime": "2025-01-01T00:00:00Z",
                        "expirationTime": "2025-01-02T00:00:00Z",
                        "sha256Hash": "abc123hash",
                    },
                })
            );
        }

        mod polling {
            use super::*;

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should preserve $name in the polling URL
            #[tokio::test]
            async fn should_preserve_name_in_the_polling_url() {
                for (name, expected_path) in [
                    ("files/abc/../../secret", "files%2Fabc%2F..%2F..%2Fsecret"),
                    ("files/.", "files/%252E"),
                    ("files/..", "files/%252E%252E"),
                    (".", "%252E"),
                    ("..", "%252E%252E"),
                ] {
                    let mut file_resource = default_file_resource();
                    file_resource["name"] = json!(name);
                    let mock = create_mock_files(MockFilesOptions {
                        file_resource,
                        poll_responses: vec!["PROCESSING", "ACTIVE"],
                        ..Default::default()
                    })
                    .await;

                    mock.files
                        .upload_file(&upload_options(
                            vec![1],
                            "application/octet-stream",
                            Some(json!({ "google": { "pollIntervalMs": 1 } })),
                        ))
                        .await
                        .unwrap();

                    let poll_url = mock
                        .poll_requests()
                        .await
                        .last()
                        .map(|request| request.url.path().to_string());
                    assert_eq!(
                        poll_url,
                        Some(format!("{BASE_PATH}{expected_path}")),
                        "{name}"
                    );
                }
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should reject promptly when aborted during the polling delay
            #[tokio::test]
            async fn should_reject_promptly_when_aborted_during_the_polling_delay() {
                let mock = create_mock_files(MockFilesOptions {
                    poll_responses: vec!["PROCESSING", "PROCESSING"],
                    ..Default::default()
                })
                .await;
                let signal = AbortSignal::new();

                // a long interval that only an abort-aware delay can escape
                let options = UploadFileCallOptions {
                    abort_signal: Some(signal.clone()),
                    ..upload_options(
                        vec![1],
                        "application/octet-stream",
                        Some(json!({
                            "google": { "pollIntervalMs": 60_000, "pollTimeoutMs": 120_000 },
                        })),
                    )
                };
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    signal.abort();
                });

                let start = Instant::now();
                let result = mock.files.upload_file(&options).await;
                assert!(result.is_err());
                assert!(start.elapsed() < Duration::from_millis(5000));
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should poll until file state becomes ACTIVE
            #[tokio::test]
            async fn should_poll_until_file_state_becomes_active() {
                let mock = create_mock_files(MockFilesOptions {
                    poll_responses: vec!["PROCESSING", "PROCESSING", "ACTIVE"],
                    ..Default::default()
                })
                .await;

                let result = mock
                    .files
                    .upload_file(&upload_options(
                        vec![1],
                        "application/octet-stream",
                        Some(json!({ "google": { "pollIntervalMs": 10 } })),
                    ))
                    .await
                    .unwrap();

                assert_eq!(
                    result.provider_reference.get("google").map(String::as_str),
                    default_file_resource()["uri"].as_str()
                );
                let poll_calls: Vec<String> = mock
                    .requests()
                    .await
                    .iter()
                    .map(|request| request.url.path().to_string())
                    .filter(|url| url.contains("files/abc123"))
                    .collect();
                assert!(!poll_calls.is_empty());
                assert_eq!(poll_calls[0], format!("{BASE_PATH}files/abc123"));
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should throw on poll timeout
            #[tokio::test]
            async fn should_throw_on_poll_timeout() {
                let processing_resource = with_state(&default_file_resource(), "PROCESSING");
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/upload/v1beta/files"))
                    .respond_with(ResponseTemplate::new(200).insert_header(
                        "x-goog-upload-url",
                        format!("{}/resume", server.uri()).as_str(),
                    ))
                    .mount(&server)
                    .await;
                Mock::given(method("POST"))
                    .and(path("/resume"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(json!({ "file": processing_resource })),
                    )
                    .mount(&server)
                    .await;
                Mock::given(method("GET"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(&processing_resource))
                    .mount(&server)
                    .await;
                let files = files_for(&server);

                let error = files
                    .upload_file(&upload_options(
                        vec![1],
                        "application/octet-stream",
                        Some(json!({ "google": { "pollIntervalMs": 10, "pollTimeoutMs": 50 } })),
                    ))
                    .await
                    .unwrap_err();

                assert!(error.to_string().contains("timed out"), "{error}");
            }
        }

        mod provider_options {
            use super::*;

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › provider options › should accept valid provider options
            #[tokio::test]
            async fn should_accept_valid_provider_options() {
                let mock = create_mock_files(MockFilesOptions::default()).await;

                let result = mock
                    .files
                    .upload_file(&upload_options(
                        vec![1],
                        "text/plain",
                        Some(json!({
                            "google": {
                                "displayName": "test",
                                "pollIntervalMs": 5000,
                                "pollTimeoutMs": 60000,
                            },
                        })),
                    ))
                    .await
                    .unwrap();

                assert!(result.provider_reference.contains_key("google"));
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › provider options › should pass through unknown properties via passthrough
            #[tokio::test]
            async fn should_pass_through_unknown_properties_via_passthrough() {
                let mock = create_mock_files(MockFilesOptions::default()).await;

                let result = mock
                    .files
                    .upload_file(&upload_options(
                        vec![1],
                        "text/plain",
                        Some(json!({ "google": { "customField": "custom-value" } })),
                    ))
                    .await
                    .unwrap();

                assert!(result.provider_reference.contains_key("google"));
            }
        }

        mod response_metadata {
            use super::*;

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › response metadata › should omit optional fields from metadata when not present
            #[tokio::test]
            async fn should_omit_optional_fields_from_metadata_when_not_present() {
                let minimal_file_resource = json!({
                    "name": "files/minimal",
                    "mimeType": "text/plain",
                    "uri": "https://generativelanguage.googleapis.com/v1beta/files/minimal",
                    "state": "ACTIVE",
                });
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(path("/upload/v1beta/files"))
                    .respond_with(ResponseTemplate::new(200).insert_header(
                        "x-goog-upload-url",
                        format!("{}/resume", server.uri()).as_str(),
                    ))
                    .mount(&server)
                    .await;
                Mock::given(method("POST"))
                    .and(path("/resume"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(json!({ "file": minimal_file_resource })),
                    )
                    .mount(&server)
                    .await;
                let files = files_for(&server);

                let result = files
                    .upload_file(&upload_options(
                        vec![1],
                        "application/octet-stream",
                        Some(json!({})),
                    ))
                    .await
                    .unwrap();

                let google =
                    serde_json::to_value(&result.provider_metadata.unwrap()["google"]).unwrap();
                assert_eq!(
                    google,
                    json!({
                        "name": "files/minimal",
                        "mimeType": "text/plain",
                        "state": "ACTIVE",
                        "uri": "https://generativelanguage.googleapis.com/v1beta/files/minimal",
                    })
                );
                assert!(google.get("createTime").is_none());
                assert!(google.get("expirationTime").is_none());
                assert!(google.get("sha256Hash").is_none());
            }
        }
    }
}

// ── Older ports (previously `tests/google_files_test.rs`) ──
//
// Rust translations of the Google Files provider tests.
//
// Source: `reference/ai/packages/google/src/google-files.test.ts`
// (16 test cases).
//
// Google uses a resumable upload protocol with three phases:
// 1. POST `/upload/v1beta/files` (init) - returns `x-goog-upload-url` header.
// 2. POST to the upload URL - returns `{ "file": { ... } }`.
// 3. GET `/{file.name}` (poll) - returns the file resource with updated state.

mod older_files {
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::google::{GoogleProvider, GoogleProviderSettings, create_google};
    use aimux_core::files_model::{UploadFileCallOptions, UploadFileData};
    use aimux_core::provider::Provider;
    use aimux_core::shared::FileBytes;

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
        create_google(GoogleProviderSettings {
            api_key: Some("test-api-key".to_string()),
            base_url: Some(format!("{}/v1beta", server.uri())),
            ..Default::default()
        })
        .unwrap()
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
            .respond_with(
                ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url),
            )
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

    // -- upload data tests -------------------------------------------------------

    /// partial: `should send ArrayBuffer-backed data and let fetch derive its content length`; full port in src/google/tests/files.rs.
    #[tokio::test]
    async fn should_send_file_data_to_upload_url_with_correct_headers() {
        let server = MockServer::start().await;
        mount_success_mocks(&server).await;

        let provider = provider(&server);
        let files = Provider::files(&provider).unwrap();

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

    /// partial: `should return unsupported warning when filename is provided`; full port in src/google/tests/files.rs.
    #[tokio::test]
    async fn should_return_unsupported_warning_when_filename_provided() {
        let server = MockServer::start().await;
        mount_success_mocks(&server).await;

        let provider = provider(&server);
        let files = Provider::files(&provider).unwrap();

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

    /// partial: `should return providerMetadata with file details`; full port in src/google/tests/files.rs.
    #[tokio::test]
    async fn should_return_provider_metadata_with_file_details() {
        let server = MockServer::start().await;
        mount_success_mocks(&server).await;

        let provider = provider(&server);
        let files = Provider::files(&provider).unwrap();

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

    /// google-files.ts:84-92 (init) and :118-126 (upload) call the provider's
    /// plain `fetch`, not `postToApi`, so neither request carries the
    /// `ai-sdk-provider-utils/<v>` part (post-to-api.ts:100-104). The init sends
    /// the provider headers, whose user-agent is `ai-sdk-google/<v>`
    /// (google-provider.ts:258-269); the upload sends only its
    /// `X-Goog-Upload-*` headers, so no user-agent.
    #[tokio::test]
    async fn upload_requests_carry_no_provider_utils_user_agent_part() {
        let server = MockServer::start().await;
        mount_success_mocks(&server).await;
        let provider = provider(&server);
        Provider::files(&provider)
            .unwrap()
            .upload_file(&upload_options())
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url.path(), "/upload/v1beta/files");
        assert_eq!(
            requests[0]
                .headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok()),
            Some("ai-sdk-google/4.0.87")
        );
        assert_eq!(requests[1].url.path(), "/resume");
        assert_eq!(requests[1].headers.get("user-agent"), None);
    }

    // -- response metadata tests -------------------------------------------------

    /// partial: `should omit optional fields from metadata when not present`; full port in src/google/tests/files.rs.
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
            .respond_with(
                ResponseTemplate::new(200).insert_header("x-goog-upload-url", &upload_url),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/resume"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "file": minimal_resource })),
            )
            .mount(&server)
            .await;

        let provider = provider(&server);
        let files = Provider::files(&provider).unwrap();

        let result = files.upload_file(&upload_options()).await.unwrap();

        let metadata = result.provider_metadata.expect("metadata");
        let google = metadata.get("google").expect("google metadata");
        assert_eq!(google["name"], "files/minimal");
        assert!(google.get("displayName").is_none());
        assert_eq!(google["mimeType"], "text/plain");
        assert!(google.get("sizeBytes").is_none());
        assert_eq!(google["state"], "ACTIVE");
        assert_eq!(
            google["uri"],
            "https://generativelanguage.googleapis.com/v1beta/files/minimal"
        );
        assert!(google.get("createTime").is_none());
        assert!(google.get("expirationTime").is_none());
        assert!(google.get("sha256Hash").is_none());
    }

    mod google_files {
        use super::*;

        mod constructor {
            use super::*;

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › constructor › should expose correct provider and specification version
            #[tokio::test]
            async fn should_expose_correct_provider_and_specification_version() {
                let provider = create_google(GoogleProviderSettings {
                    api_key: Some("test-api-key".to_string()),
                    ..Default::default()
                })
                .unwrap();
                let files = Provider::files(&provider).unwrap();
                assert_eq!(files.provider(), "google.generative-ai");
                assert_eq!(files.specification_version(), "v4");
            }
        }

        mod upload_file {
            use super::*;

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should send correct headers for resumable upload initiation
            #[tokio::test]
            async fn should_send_correct_headers_for_resumable_upload_initiation() {
                let server = MockServer::start().await;
                mount_success_mocks(&server).await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

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

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should omit displayName from body when not provided
            #[tokio::test]
            async fn should_omit_display_name_from_body_when_not_provided() {
                let server = MockServer::start().await;
                mount_success_mocks(&server).await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                files.upload_file(&upload_options()).await.unwrap();

                let requests = server.received_requests().await.unwrap();
                let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
                assert_eq!(body, json!({ "file": {} }));
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should return providerReference with google key set to file URI
            #[tokio::test]
            async fn should_return_provider_reference_with_google_key_set_to_file_uri() {
                let server = MockServer::start().await;
                mount_success_mocks(&server).await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let result = files.upload_file(&upload_options()).await.unwrap();

                assert_eq!(
                    result.provider_reference.get("google"),
                    Some(
                        &"https://generativelanguage.googleapis.com/v1beta/files/abc123"
                            .to_string()
                    )
                );
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should return empty warnings when filename is not provided
            #[tokio::test]
            async fn should_return_empty_warnings_when_filename_not_provided() {
                let server = MockServer::start().await;
                mount_success_mocks(&server).await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

                let result = files.upload_file(&upload_options()).await.unwrap();

                assert!(result.warnings.is_empty());
            }

            /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › should handle base64 string data
            #[tokio::test]
            async fn should_handle_base64_string_data() {
                let server = MockServer::start().await;
                mount_success_mocks(&server).await;

                let provider = provider(&server);
                let files = Provider::files(&provider).unwrap();

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

            mod polling {
                use super::*;

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should not poll when file is immediately ACTIVE
                #[tokio::test]
                async fn should_not_poll_when_file_is_immediately_active() {
                    let server = MockServer::start().await;
                    mount_success_mocks(&server).await;

                    let provider = provider(&server);
                    let files = Provider::files(&provider).unwrap();

                    files.upload_file(&upload_options()).await.unwrap();

                    let requests = server.received_requests().await.unwrap();
                    // Only 2 requests: init + upload. No poll requests.
                    assert_eq!(requests.len(), 2);
                }

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › polling › should throw when file state is FAILED
                #[tokio::test]
                async fn should_throw_when_file_state_is_failed() {
                    let server = MockServer::start().await;
                    let upload_url = format!("{}/resume", server.uri());

                    Mock::given(method("POST"))
                        .and(path("/upload/v1beta/files"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .insert_header("x-goog-upload-url", &upload_url),
                        )
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
                    let files = Provider::files(&provider).unwrap();

                    let result = files.upload_file(&upload_options()).await;

                    assert!(result.is_err());
                    let err = result.unwrap_err();
                    assert!(
                        err.to_string().contains("File processing failed"),
                        "error should mention 'File processing failed': {err}"
                    );
                }
            }

            mod error_handling {
                use super::*;

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › error handling › should throw when initiation request fails
                #[tokio::test]
                async fn should_throw_when_initiation_request_fails() {
                    let server = MockServer::start().await;

                    Mock::given(method("POST"))
                        .and(path("/upload/v1beta/files"))
                        .respond_with(ResponseTemplate::new(500).set_body_string("Init error"))
                        .mount(&server)
                        .await;

                    // Only the message text matters here; skip the retries so it stays fast.
                    let provider = provider(&server);
                    let files = Provider::files(&provider).unwrap();

                    let result = files.upload_file(&upload_options()).await;

                    assert!(result.is_err());
                    let err = result.unwrap_err();
                    assert!(
                        err.to_string()
                            .contains("Failed to initiate resumable upload"),
                        "error should mention 'Failed to initiate': {err}"
                    );
                }

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › error handling › should throw when no upload URL is returned
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
                    let files = Provider::files(&provider).unwrap();

                    let result = files.upload_file(&upload_options()).await;

                    assert!(result.is_err());
                    let err = result.unwrap_err();
                    assert!(
                        err.to_string().contains("No upload URL"),
                        "error should mention 'No upload URL': {err}"
                    );
                }

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › error handling › should throw when upload request fails
                #[tokio::test]
                async fn should_throw_when_upload_request_fails() {
                    let server = MockServer::start().await;
                    let upload_url = format!("{}/resume", server.uri());

                    Mock::given(method("POST"))
                        .and(path("/upload/v1beta/files"))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .insert_header("x-goog-upload-url", &upload_url),
                        )
                        .mount(&server)
                        .await;

                    Mock::given(method("POST"))
                        .and(path("/resume"))
                        .respond_with(ResponseTemplate::new(500).set_body_string("Upload error"))
                        .mount(&server)
                        .await;

                    // Only the message text matters here; skip the retries so it stays fast.
                    let provider = provider(&server);
                    let files = Provider::files(&provider).unwrap();

                    let result = files.upload_file(&upload_options()).await;

                    assert!(result.is_err());
                    let err = result.unwrap_err();
                    assert!(
                        err.to_string().contains("Failed to upload file data"),
                        "error should mention 'Failed to upload file data': {err}"
                    );
                }
            }

            mod provider_options {
                use super::*;

                /// upstream: @ai-sdk/google src/google-files.test.ts › GoogleFiles › uploadFile › provider options › should work without provider options
                #[tokio::test]
                async fn should_work_without_provider_options() {
                    let server = MockServer::start().await;
                    mount_success_mocks(&server).await;

                    let provider = provider(&server);
                    let files = Provider::files(&provider).unwrap();

                    let result = files.upload_file(&upload_options()).await.unwrap();

                    assert!(result.provider_reference.contains_key("google"));
                }
            }
        }
    }
}

/// aimux's own: an option of the wrong type fails as `parseProviderOptions`
/// fails, with the schema error as the cause, before any request.
#[tokio::test]
async fn wrong_typed_options_fail_with_the_parse_provider_options_message() {
    let server = MockServer::start().await;
    let error = files_for(&server)
        .upload_file(&upload_options(
            vec![1],
            "text/plain",
            Some(json!({ "google": { "pollIntervalMs": "soon" } })),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, aimux_core::AiMuxError::InvalidArgument { message, cause: Some(_) }
            if message == "Invalid argument for parameter providerOptions: invalid google provider options"),
        "{error:?}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
