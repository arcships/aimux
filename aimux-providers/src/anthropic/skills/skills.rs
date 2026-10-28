//! Skill uploads and version metadata from the Anthropic Skills API.

use indexmap::IndexSet;

use async_trait::async_trait;
use serde_json::json;

use aimux_core::AiMuxError;
use aimux_core::skills_model::{Skills, UploadSkillCallOptions, UploadSkillResult};
use aimux_provider_utils::{HttpBody, HttpRequest};

use crate::anthropic::config::AnthropicConfig;
use crate::anthropic::language_model_options::CANONICAL;

use super::skills_api::{SkillResponse, VersionResponse};

/// The provider's skill upload interface.
pub struct AnthropicSkills {
    config: AnthropicConfig,
}

impl AnthropicSkills {
    pub(crate) fn from_config(config: AnthropicConfig) -> Self {
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
            body.extend_from_slice(
                &aimux_provider_utils::convert_inline_file_data_to_uint8_array(&file.data)?,
            );
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let config = self.config.resolved().await?;
        let mut headers = config
            .request_headers(
                None,
                &IndexSet::from(["skills-2025-10-02".to_string()]),
                &[],
            )
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
            provider_reference: aimux_core::shared::SharedProviderReference::from([(
                CANONICAL.to_string(),
                response.id,
            )]),
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
