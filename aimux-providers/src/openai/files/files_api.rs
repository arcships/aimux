//! Response shape of the OpenAI Files endpoint (openai-files-api.ts).

use serde::Deserialize;

/// The response from the OpenAI `/files` endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIFilesResponse {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) bytes: Option<u64>,
    #[serde(default, rename = "created_at")]
    pub(crate) created_at: Option<u64>,
    #[serde(default)]
    pub(crate) filename: Option<String>,
    #[serde(default)]
    pub(crate) purpose: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default, rename = "expires_at")]
    pub(crate) expires_at: Option<u64>,
}
