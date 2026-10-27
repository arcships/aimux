//! `skills/anthropic-skills.test.ts` of `@ai-sdk/anthropic` 4.0.71.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};

use aimux_core::files_model::UploadFileData;
use aimux_core::shared::FileBytes;
use aimux_core::skills_model::{SkillFile, Skills, UploadSkillCallOptions};

use crate::anthropic::language_model_options::CANONICAL;
use crate::anthropic::{AnthropicProvider, AnthropicProviderSettings, create_anthropic};
use crate::test_support::mock_fetch::{Canned, MockFetch, Seen};

const SKILL_CREATE: &str = include_str!("fixtures/anthropic-skill-create.json");
const SKILL_VERSION_CREATE: &str = include_str!("fixtures/anthropic-skill-version-create.json");

/// `createAnthropic({ apiKey: 'test-api-key', baseURL: 'https://api.anthropic.com/v1', fetch })`.
fn provider(server: &Arc<MockFetch>) -> AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some("https://api.anthropic.com/v1".into()),
        fetch: Some(server.transport()),
        ..Default::default()
    })
    .unwrap()
}

/// `prepareResponse` for `/v1/skills` with `anthropic-skill-create` plus
/// `prepareVersionMetadataResponse()`: the two exchanges of one upload.
fn prepared_server() -> Arc<MockFetch> {
    MockFetch::new(vec![
        Canned::json(&serde_json::from_str::<Value>(SKILL_CREATE).unwrap()),
        Canned::json(&serde_json::from_str::<Value>(SKILL_VERSION_CREATE).unwrap()),
    ])
}

/// `convertUint8ArrayToBase64(new TextEncoder().encode('console.log("hello")'))`.
fn test_file_content_base64() -> String {
    base64::engine::general_purpose::STANDARD.encode(br#"console.log("hello")"#)
}

/// `{ files: [{ path: 'index.ts', data: { type: 'data', data: testFileContentBase64 } }] }`.
fn upload_options() -> UploadSkillCallOptions {
    UploadSkillCallOptions {
        files: vec![SkillFile {
            path: "index.ts".into(),
            data: UploadFileData::Data {
                data: FileBytes::Base64(test_file_content_base64()),
            },
        }],
        display_title: None,
        provider_options: None,
    }
}

/// One part of a `multipart/form-data` body.
#[derive(Debug)]
struct MultipartPart {
    name: String,
    filename: Option<String>,
    data: Vec<u8>,
}

/// `requestBodyMultipart`: the parts of a recorded multipart request.
fn multipart_parts(call: &Seen) -> Vec<MultipartPart> {
    let content_type = &call.headers["content-type"];
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart boundary");
    let delimiter = format!("--{boundary}");
    let body = &call.body;
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(offset) = find(&body[start..], delimiter.as_bytes()) {
        positions.push(start + offset);
        start += offset + delimiter.len();
    }
    positions
        .windows(2)
        .map(|pair| {
            let part = &body[pair[0] + delimiter.len() + 2..pair[1] - 2];
            let split = find(part, b"\r\n\r\n").expect("part headers");
            let headers = String::from_utf8_lossy(&part[..split]).into_owned();
            let attribute = |key: &str| {
                let marker = format!("{key}=\"");
                headers.find(&marker).map(|index| {
                    let rest = &headers[index + marker.len()..];
                    rest[..rest.find('"').unwrap()].to_string()
                })
            };
            MultipartPart {
                name: attribute(" name").expect("part name"),
                filename: attribute("filename"),
                data: part[split + 4..].to_vec(),
            }
        })
        .collect()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

mod anthropic_skills {
    use super::*;

    mod upload_skill {
        use super::*;

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should preserve skill $skillId and version $version as URL path segments
        #[tokio::test]
        async fn should_preserve_skill_skill_id_and_version_version_as_url_path_segments() {
            let rows = [
                (
                    "skill/../../admin",
                    "v1/../../admin",
                    "skill%2F..%2F..%2Fadmin",
                    "v1%2F..%2F..%2Fadmin",
                ),
                (".", "version-1", "%252E", "version-1"),
                ("..", "version-1", "%252E%252E", "version-1"),
                ("skill-1", ".", "skill-1", "%252E"),
                ("skill-1", "..", "skill-1", "%252E%252E"),
            ];
            for (skill_id, version, expected_skill_id, expected_version) in rows {
                let server = MockFetch::new(vec![
                    Canned::json(&json!({
                        "id": skill_id,
                        "latest_version": version,
                        "source": "custom",
                        "created_at": "2026-08-24T00:00:00Z",
                        "updated_at": "2026-08-24T00:00:00Z",
                    })),
                    Canned::json(&json!({
                        "type": "skill_version",
                        "skill_id": skill_id,
                        "name": "test-skill",
                        "description": "test skill",
                    })),
                ]);

                provider(&server)
                    .skills()
                    .upload_skill(&upload_options())
                    .await
                    .unwrap();

                let request_urls: Vec<String> =
                    server.seen().into_iter().map(|call| call.url).collect();
                assert_eq!(
                    request_urls[1],
                    format!(
                        "https://api.anthropic.com/v1/skills/{expected_skill_id}/versions/{expected_version}"
                    ),
                    "skill {skill_id:?}, version {version:?}"
                );
            }
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should send files as multipart form data
        #[tokio::test]
        async fn should_send_files_as_multipart_form_data() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            skills.upload_skill(&upload_options()).await.unwrap();

            let body = multipart_parts(&server.seen()[0]);
            assert!(!body.is_empty());
            let file = body
                .iter()
                .find(|part| part.name == "files[]")
                .expect("files[] part");
            // A `File`: a part with a filename.
            assert!(file.filename.is_some(), "{file:?}");
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should include anthropic-beta header
        #[tokio::test]
        async fn should_include_anthropic_beta_header() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            skills.upload_skill(&upload_options()).await.unwrap();

            let headers = &server.seen()[0].headers;
            assert_eq!(
                headers.get("anthropic-beta").map(String::as_str),
                Some("skills-2025-10-02")
            );
            assert_eq!(
                headers.get("x-api-key").map(String::as_str),
                Some("test-api-key")
            );
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should map response to providerReference
        #[tokio::test]
        async fn should_map_response_to_provider_reference() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            let result = skills.upload_skill(&upload_options()).await.unwrap();

            assert_eq!(
                serde_json::to_value(&result.provider_reference).unwrap(),
                json!({ CANONICAL: "skill_01Xud7kLMsjLfc7Aa6RvigZf" })
            );
            assert_eq!(result.display_title.as_deref(), Some("Test Capture Skill"));
            assert_eq!(result.name.as_deref(), Some("test-capture-skill"));
            assert_eq!(
                result.description.as_deref(),
                Some("An updated test skill for fixture capture")
            );
            assert_eq!(result.latest_version.as_deref(), Some("1772078378207930"));
            assert_eq!(
                serde_json::to_value(&result.provider_metadata).unwrap(),
                json!({
                    CANONICAL: {
                        "source": "custom",
                        "createdAt": "2026-02-26T03:59:39.314772Z",
                        "updatedAt": "2026-02-26T03:59:39.314772Z",
                    },
                })
            );
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should send display_title in form data when displayTitle is provided
        #[tokio::test]
        async fn should_send_display_title_in_form_data_when_display_title_is_provided() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            skills
                .upload_skill(&UploadSkillCallOptions {
                    display_title: Some("My Custom Title".into()),
                    ..upload_options()
                })
                .await
                .unwrap();

            let body = multipart_parts(&server.seen()[0]);
            let display_title = body
                .iter()
                .find(|part| part.name == "display_title")
                .map(|part| String::from_utf8(part.data.clone()).unwrap());
            assert_eq!(display_title.as_deref(), Some("My Custom Title"));
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should not send display_title when displayTitle is not provided
        #[tokio::test]
        async fn should_not_send_display_title_when_display_title_is_not_provided() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            skills.upload_skill(&upload_options()).await.unwrap();

            let body = multipart_parts(&server.seen()[0]);
            assert!(
                !body.iter().any(|part| part.name == "display_title"),
                "{body:?}"
            );
        }

        /// upstream: @ai-sdk/anthropic src/skills/anthropic-skills.test.ts › AnthropicSkills › uploadSkill › should return no warnings
        #[tokio::test]
        async fn should_return_no_warnings() {
            let server = prepared_server();

            let skills = provider(&server).skills();
            let result = skills.upload_skill(&upload_options()).await.unwrap();

            assert_eq!(serde_json::to_value(&result.warnings).unwrap(), json!([]));
        }
    }
}
