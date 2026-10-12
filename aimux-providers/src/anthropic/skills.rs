//! Skill uploads and version metadata from the Anthropic Skills API.

use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use aimux_core::AiMuxError;
use aimux_core::files_model::UploadFileData;
use aimux_core::shared::FileBytes;
use aimux_core::skills_model::{Skills, UploadSkillCallOptions, UploadSkillResult};
use aimux_provider_utils::{HttpBody, HttpRequest};

use super::config::AnthropicModelConfig;
use super::options::CANONICAL;

#[derive(Deserialize)]
struct SkillResponse {
    id: String,
    display_title: Option<String>,
    name: Option<String>,
    description: Option<String>,
    latest_version: Option<String>,
    source: String,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(rename = "type")]
    _kind: String,
    #[serde(rename = "skill_id")]
    _skill_id: String,
    name: Option<String>,
    description: Option<String>,
}

/// The provider's skill upload interface.
pub struct AnthropicSkills {
    config: AnthropicModelConfig,
}

impl AnthropicSkills {
    pub(crate) fn from_config(config: AnthropicModelConfig) -> Self {
        Self { config }
    }
}

// Dot segments need double encoding because URL parsing normalizes even
// percent-encoded dots. Otherwise this is encodeURIComponent's byte encoding.
fn encode_path_segment(value: &str) -> String {
    if value == "." {
        return "%252E".to_string();
    }
    if value == ".." {
        return "%252E%252E".to_string();
    }
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    encoded
}

#[async_trait]
impl Skills for AnthropicSkills {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    async fn upload_skill(
        &self,
        options: &UploadSkillCallOptions,
    ) -> Result<UploadSkillResult, AiMuxError> {
        let boundary = format!("----{}", aimux_provider_utils::generate_id());
        let mut body = Vec::new();
        if let Some(title) = &options.display_title {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"display_title\"\r\n\r\n{title}\r\n").as_bytes());
        }
        for file in &options.files {
            // FormData encodes these characters in multipart filenames.
            let path = file
                .path
                .replace('\r', "%0D")
                .replace('\n', "%0A")
                .replace('"', "%22");
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"files[]\"; filename=\"{path}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
            match &file.data {
                UploadFileData::Text { text } => body.extend_from_slice(text.as_bytes()),
                UploadFileData::Data {
                    data: FileBytes::Binary(bytes),
                } => body.extend_from_slice(bytes),
                UploadFileData::Data {
                    data: FileBytes::Base64(data),
                } => {
                    use base64::Engine;
                    let normalized = data
                        .replace('-', "+")
                        .replace('_', "/")
                        .chars()
                        .filter(|character| {
                            !matches!(character, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
                        })
                        .collect::<String>();
                    let engine = base64::engine::GeneralPurpose::new(
                        &base64::alphabet::STANDARD,
                        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(
                            base64::engine::DecodePaddingMode::Indifferent,
                        ),
                    );
                    body.extend_from_slice(&engine.decode(normalized).map_err(|error| {
                        AiMuxError::InvalidArgument(format!("invalid base64: {error}"))
                    })?);
                }
            }
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let config = self.config.resolved().await?;
        let mut headers = config
            .request_headers(None, &BTreeSet::from(["skills-2025-10-02".to_string()]))
            .await?;
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("anthropic-beta"));
        headers.push((
            "anthropic-beta".to_string(),
            "skills-2025-10-02".to_string(),
        ));
        let response = aimux_provider_utils::post_to_api(
            config.with_transport(HttpRequest {
                url: config.url("/skills"),
                headers: headers.clone(),
                ..HttpRequest::default()
            }),
            HttpBody::Bytes(body, format!("multipart/form-data; boundary={boundary}")),
            aimux_provider_utils::create_json_response_handler(),
            config.failed_response_handler(),
        )
        .await?;
        let response: SkillResponse = response.value;
        let version: Option<VersionResponse> = if let Some(version) = &response.latest_version {
            let result = aimux_provider_utils::get_from_api(
                config.with_transport(HttpRequest {
                    url: config.url(&format!(
                        "/skills/{}/versions/{}",
                        encode_path_segment(&response.id),
                        encode_path_segment(version)
                    )),
                    headers,
                    ..HttpRequest::default()
                }),
                aimux_provider_utils::create_json_response_handler(),
                config.failed_response_handler(),
            )
            .await?;
            Some(result.value)
        } else {
            None
        };
        Ok(UploadSkillResult {
            provider_reference: HashMap::from([(CANONICAL.to_string(), response.id)]),
            display_title: response.display_title,
            name: version
                .as_ref()
                .and_then(|version| version.name.clone())
                .or(response.name),
            description: version
                .and_then(|version| version.description)
                .or(response.description),
            latest_version: response.latest_version,
            provider_metadata: Some(aimux_core::shared::provider_namespace(
                CANONICAL,
                json!({
                    "source": response.source,
                    "createdAt": response.created_at,
                    "updatedAt": response.updated_at,
                }),
            )?),
            warnings: Vec::new(),
        })
    }
}
