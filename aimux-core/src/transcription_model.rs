//! The `TranscriptionModel` trait — the provider-facing interface for
//! speech-to-text.
//!
//! Aligned with Vercel AI SDK `TranscriptionModelV4`
//! (`reference/ai/packages/provider/src/transcription-model/v4/`).
//!
//! This is the only non-chat model type with an optional streaming method,
//! [`TranscriptionModel::do_stream`]. The default implementation returns
//! [`AiMuxError::Unsupported`]; providers override it as needed.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::error::AiMuxError;
use crate::shared::{
    AbortSignal, SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning,
};

/// Audio input: raw bytes or a base64-encoded string.
#[derive(Debug, Clone)]
pub enum AudioInput {
    /// Raw binary bytes.
    Binary(Vec<u8>),
    /// A base64-encoded string.
    Base64(String),
}

/// A chunk of audio in a streaming transcription request.
#[derive(Debug, Clone)]
pub enum AudioChunk {
    /// Raw binary bytes.
    Binary(Vec<u8>),
    /// A base64-encoded string.
    Base64(String),
}

/// A transcript segment with timing information.
#[derive(Debug, Clone)]
pub struct TranscriptionSegment {
    /// The text content of this segment.
    pub text: String,
    /// The start time of this segment in seconds.
    pub start_second: f64,
    /// The end time of this segment in seconds.
    pub end_second: f64,
}

/// Options passed to [`TranscriptionModel::do_generate`].
///
/// Aligned with V4 `TranscriptionModelV4CallOptions`.
#[derive(Debug, Clone)]
pub struct TranscriptionCallOptions {
    /// Audio data to transcribe (raw bytes or a base64-encoded string).
    pub audio: AudioInput,

    /// The IANA media type of the audio data, e.g. `"audio/mp3"`.
    pub media_type: String,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl TranscriptionCallOptions {
    /// Create options with the given audio and media type.
    pub fn new(audio: AudioInput, media_type: impl Into<String>) -> Self {
        Self {
            audio,
            media_type: media_type.into(),
            provider_options: None,
            abort_signal: None,
            headers: None,
        }
    }
}

/// The result of [`TranscriptionModel::do_generate`].
///
/// Aligned with V4 `TranscriptionModelV4Result`.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// The complete transcribed text from the audio.
    pub text: String,

    /// Transcript segments with timing information.
    pub segments: Vec<TranscriptionSegment>,

    /// The detected language (ISO 639-1 code, e.g. `"en"`), if detected.
    pub language: Option<String>,

    /// The total duration of the audio in seconds, if determined.
    pub duration_in_seconds: Option<f64>,

    /// Warnings for the call, e.g. unsupported settings.
    pub warnings: Vec<Warning>,

    /// Optional request information for telemetry and debugging.
    pub request: Option<TranscriptionRequest>,

    /// Response information for telemetry and debugging.
    pub response: TranscriptionResponse,

    /// Additional provider-specific metadata, keyed by provider name.
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// Optional request information for a transcription call.
#[derive(Debug, Clone, Default)]
pub struct TranscriptionRequest {
    /// Raw request HTTP body that was sent (JSON stringified).
    pub body: Option<String>,
}

/// Response information for a transcription call.
#[derive(Debug, Clone, Default)]
pub struct TranscriptionResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    pub model_id: Option<String>,
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// Response body (opaque JSON).
    pub body: Option<serde_json::Value>,
}

/// The input audio format for a streaming transcription request.
#[derive(Debug, Clone)]
pub struct InputAudioFormat {
    /// Audio format type, e.g. `"audio/pcm"`, `"audio/pcmu"`, `"audio/pcma"`.
    pub format_type: String,
    /// Sample rate in Hz. Only applicable for formats that require a rate.
    pub rate: Option<u32>,
}

/// Options passed to [`TranscriptionModel::do_stream`].
///
/// Aligned with V4 `TranscriptionModelV4StreamOptions`.
pub struct TranscriptionStreamOptions {
    /// Audio chunks to transcribe (raw bytes or base64-encoded strings).
    pub audio: Pin<Box<dyn Stream<Item = AudioChunk> + Send>>,

    /// The input audio format for the raw audio chunks.
    pub input_audio_format: InputAudioFormat,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional HTTP headers for HTTP/WebSocket-based providers.
    pub headers: Option<SharedHeaders>,

    /// When `true`, providers should include raw provider chunks in the
    /// stream.
    pub include_raw_chunks: bool,
}

impl std::fmt::Debug for TranscriptionStreamOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptionStreamOptions")
            .field("input_audio_format", &self.input_audio_format)
            .field("provider_options", &self.provider_options)
            .field("abort_signal", &self.abort_signal)
            .field("headers", &self.headers)
            .field("include_raw_chunks", &self.include_raw_chunks)
            .field("audio", &"<stream>")
            .finish()
    }
}

/// A single chunk in the stream returned by `do_stream`.
///
/// Aligned with V4 `TranscriptionModelV4StreamPart`.
#[derive(Debug)]
pub enum TranscriptionStreamPart {
    /// Stream start event, carrying warnings.
    StreamStart { warnings: Vec<Warning> },
    /// Append-only transcript delta.
    TranscriptDelta {
        id: Option<String>,
        delta: String,
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Non-final transcript text (may be revised by later parts).
    TranscriptPartial {
        id: Option<String>,
        text: String,
        start_second: Option<f64>,
        duration_in_seconds: Option<f64>,
        channel_index: Option<u32>,
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Final transcript text for a provider-defined segment or utterance.
    TranscriptFinal {
        id: Option<String>,
        text: String,
        start_second: Option<f64>,
        end_second: Option<f64>,
        channel_index: Option<u32>,
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Response metadata, emitted once available.
    ResponseMetadata {
        timestamp: Option<String>,
        model_id: Option<String>,
        headers: Option<SharedHeaders>,
        body: Option<serde_json::Value>,
    },
    /// Metadata available after the stream finishes.
    Finish {
        text: String,
        segments: Vec<TranscriptionSegment>,
        language: Option<String>,
        duration_in_seconds: Option<f64>,
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Raw provider chunk (only when `include_raw_chunks` is `true`).
    Raw { raw_value: serde_json::Value },
    /// An error occurred mid-stream.
    Error { error: serde_json::Value },
}

/// The result of [`TranscriptionModel::do_stream`].
///
/// Aligned with V4 `TranscriptionModelV4StreamResult`.
pub struct TranscriptionStreamResult {
    /// The stream of [`TranscriptionStreamPart`] items.
    pub stream: Pin<Box<dyn Stream<Item = Result<TranscriptionStreamPart, AiMuxError>> + Send>>,
    /// Optional request information for debugging.
    pub request: Option<TranscriptionRequest>,
    /// Optional response information.
    pub response: Option<TranscriptionResponse>,
}

impl std::fmt::Debug for TranscriptionStreamResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptionStreamResult")
            .field("request", &self.request)
            .field("response", &self.response)
            .field("stream", &"<stream>")
            .finish()
    }
}

/// The unified transcription (STT) model trait (provider-facing).
///
/// Aligned with V4 `TranscriptionModelV4`.
#[async_trait]
pub trait TranscriptionModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID, e.g. `"whisper-1"`.
    fn model_id(&self) -> &str;

    /// Generate a transcript.
    ///
    /// Naming: the `do_` prefix prevents accidental direct usage by users.
    async fn do_generate(
        &self,
        options: &TranscriptionCallOptions,
    ) -> Result<TranscriptionResult, AiMuxError>;

    /// Stream a transcript for live audio.
    ///
    /// Default implementation returns [`AiMuxError::Unsupported`]; providers
    /// override it as needed. This mirrors the optional `doStream?` in the TS
    /// spec.
    async fn do_stream(
        &self,
        _options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        Err(AiMuxError::Unsupported(format!(
            "transcription streaming is not supported by provider `{}`",
            self.provider()
        )))
    }
}
