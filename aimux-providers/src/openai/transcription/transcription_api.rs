//! Response shape of the OpenAI transcription endpoint (openai-transcription-api.ts).

use serde::Deserialize;

/// A word with timing information in the OpenAI transcription response.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAIWord {
    pub(crate) word: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
}

/// A segment with timing information in the OpenAI transcription response.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAISegment {
    pub(crate) text: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
    /// Set on `diarized_json` segments (`transcript.text.segment`).
    #[serde(default)]
    pub(crate) speaker: Option<String>,
}

/// The OpenAI transcription API response body.
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAITranscriptionResponse {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) duration: Option<f64>,
    #[serde(default)]
    pub(crate) words: Option<Vec<OpenAIWord>>,
    #[serde(default)]
    pub(crate) segments: Option<Vec<OpenAISegment>>,
}
