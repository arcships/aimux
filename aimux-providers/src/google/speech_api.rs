//! `google-speech-api.ts`: the `generateContent` response of an audio
//! request.

use serde::Deserialize as DeriveDeserialize;
use serde_json::Value;

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InlineData {
    pub(super) mime_type: Option<String>,
    pub(super) data: Option<String>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResponsePart {
    pub(super) inline_data: Option<InlineData>,
}

#[derive(DeriveDeserialize)]
pub(super) struct ResponseContent {
    pub(super) parts: Option<Vec<ResponsePart>>,
}

#[derive(DeriveDeserialize)]
pub(super) struct ResponseCandidate {
    pub(super) content: Option<ResponseContent>,
}

#[derive(DeriveDeserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SpeechResponseBody {
    pub(super) candidates: Option<Vec<ResponseCandidate>>,
    /// Passed through as the result's usage
    /// (`@ai-sdk/google@4.0.87 src/google-speech-api.ts:35`).
    pub(super) usage_metadata: Option<serde_json::Map<String, Value>>,
}
