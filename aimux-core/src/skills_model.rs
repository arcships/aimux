//! Provider-facing skill upload interface, aligned with SkillsV4.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::files_model::UploadFileData;
use crate::shared::{
    SharedProviderMetadata, SharedProviderOptions, SharedProviderReference, Warning,
};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillFile {
    pub path: String,
    pub data: UploadFileData,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UploadSkillCallOptions {
    pub files: Vec<SkillFile>,
    pub display_title: Option<String>,
    pub provider_options: Option<SharedProviderOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UploadSkillResult {
    pub provider_reference: SharedProviderReference,
    pub display_title: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub latest_version: Option<String>,
    pub provider_metadata: Option<SharedProviderMetadata>,
    pub warnings: Vec<Warning>,
}

#[async_trait]
pub trait Skills: Send + Sync {
    fn provider(&self) -> &str;
    async fn upload_skill(
        &self,
        options: &UploadSkillCallOptions,
    ) -> Result<UploadSkillResult, AiMuxError>;
}
