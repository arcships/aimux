//! Provider-facing skill upload interface, aligned with SkillsV4.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::AiMuxError;
use crate::files_model::UploadFileData;
use crate::shared::{
    SharedProviderMetadata, SharedProviderOptions, SharedProviderReference, Warning,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillFile {
    pub path: String,
    pub data: UploadFileData,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadSkillCallOptions {
    pub files: Vec<SkillFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UploadSkillResult {
    pub provider_reference: SharedProviderReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
    pub warnings: Vec<Warning>,
}

#[async_trait]
pub trait Skills: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }
    fn provider(&self) -> &str;
    async fn upload_skill(
        &self,
        options: &UploadSkillCallOptions,
    ) -> Result<UploadSkillResult, AiMuxError>;
}
