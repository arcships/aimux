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
