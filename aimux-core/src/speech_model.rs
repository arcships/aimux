//! The `SpeechModel` trait — the provider-facing interface for text-to-speech.
//!
//! Aligned with Vercel AI SDK `SpeechModelV4`
//! (`reference/ai/packages/provider/src/speech-model/v4/`).

use async_trait::async_trait;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};

/// Generated audio: a base64-encoded string or raw binary bytes.
///
/// The TS result type is `string | Uint8Array`. Providers should return data
/// without unnecessary conversion.
#[derive(Debug, Clone)]
pub enum AudioData {
    /// Audio as a base64-encoded string.
    Base64(String),
    /// Audio as raw binary bytes.
    Binary(Vec<u8>),
}

/// Options passed to [`SpeechModel::do_generate`].
///
/// Aligned with V4 `SpeechModelV4CallOptions`.
#[derive(Debug, Clone)]
pub struct SpeechCallOptions {
    /// Text to convert to speech.
    pub text: String,

    /// The voice to use (provider-specific voice ID, name, or other
    /// identifier).
    pub voice: Option<String>,

    /// The desired output format for the audio, e.g. `"mp3"`, `"wav"`.
    pub output_format: Option<String>,

    /// Instructions for the speech generation, e.g.
    /// `"Speak in a slow and steady tone"`.
    pub instructions: Option<String>,

    /// The speed of the speech generation.
    pub speed: Option<f64>,

    /// The language for speech generation (ISO 639-1 code, e.g. `"en"`, or
    /// `"auto"` for automatic detection). Provider support varies.
    pub language: Option<String>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl SpeechCallOptions {
    /// Create options with the given text and all other fields unset.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice: None,
            output_format: None,
            instructions: None,
            speed: None,
            language: None,
            provider_options: None,
            abort_signal: None,
            headers: None,
        }
    }
}

/// The result of [`SpeechModel::do_generate`].
///
/// Aligned with V4 `SpeechModelV4Result`.
#[derive(Debug, Clone)]
pub struct SpeechResult {
    /// Generated audio (base64 string or binary data).
    pub audio: AudioData,

    /// Warnings for the call, e.g. unsupported settings.
    pub warnings: Vec<Warning>,

    /// Optional request information for telemetry and debugging.
    pub request: Option<SpeechRequest>,

    /// Response information for telemetry and debugging.
    pub response: SpeechResponse,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// Optional request information for a speech call.
#[derive(Debug, Clone, Default)]
pub struct SpeechRequest {
    /// Response body (HTTP providers only).
    pub body: Option<serde_json::Value>,
}

/// Response information for a speech call.
#[derive(Debug, Clone, Default)]
pub struct SpeechResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// Response body (opaque JSON).
    pub body: Option<serde_json::Value>,
}

/// The unified speech (TTS) model trait (provider-facing).
///
/// Aligned with V4 `SpeechModelV4`.
#[async_trait]
pub trait SpeechModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"tts-1"`.
    fn model_id(&self) -> &str;

    /// Generate speech audio from text.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_generate(&self, options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError>;
}
