//! The `TranscriptionModel` trait — the provider-facing interface for
//! speech-to-text.
//!
//! Aligned with Vercel AI SDK `TranscriptionModelV4`
//! (`reference/ai/packages/provider/src/transcription-model/v4/`).
//!
//! This is the only non-chat model type with an optional streaming method,
//! [`TranscriptionModel::do_stream`]. The default implementation returns
//! [`AiMuxError::UnsupportedFunctionality`]; providers override it and users
//! enter through [`stream_transcribe`].

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::{SharedHeaders, SharedProviderMetadata, SharedProviderOptions, Warning};
use crate::{AbortSignal, retry, timeout};

/// Audio input: raw bytes or a base64-encoded string.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum AudioInput {
    /// Raw binary bytes.
    Binary(Vec<u8>),
    /// A base64-encoded string.
    Base64(String),
}

/// A chunk of audio in a streaming transcription request.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum AudioChunk {
    /// Raw binary bytes.
    Binary(Vec<u8>),
    /// A base64-encoded string.
    Base64(String),
}

/// A transcript segment with timing information.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TranscriptionCallOptions {
    /// Audio data to transcribe (raw bytes or a base64-encoded string).
    pub audio: AudioInput,

    /// The IANA media type of the audio data, e.g. `"audio/mp3"`.
    pub media_type: String,

    /// Additional provider-specific options, keyed by provider name.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<SharedProviderOptions>,

    /// Abort signal for cancelling the operation.
    #[serde(skip)]
    #[ts(skip)]
    pub abort_signal: Option<AbortSignal>,

    /// Per-call retry override. `None` uses the model default.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,

    /// Per-call operation timeout.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<crate::options::TimeoutConfiguration>,

    /// Additional HTTP headers to send with the request.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
            max_retries: None,
            timeout: None,
            headers: None,
        }
    }
}

/// The result of [`TranscriptionModel::do_generate`].
///
/// Aligned with V4 `TranscriptionModelV4Result`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TranscriptionResult {
    /// The complete transcribed text from the audio.
    pub text: String,

    /// Transcript segments with timing information.
    pub segments: Vec<TranscriptionSegment>,

    /// The detected language (ISO 639-1 code, e.g. `"en"`), if detected.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// The total duration of the audio in seconds, if determined.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_in_seconds: Option<f64>,

    /// Warnings for the call, e.g. unsupported settings.
    pub warnings: Vec<Warning>,

    /// Optional request information for telemetry and debugging.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<TranscriptionRequest>,

    /// Response information for telemetry and debugging.
    pub response: TranscriptionResponse,

    /// Additional provider-specific metadata, keyed by provider name.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// Optional request information for a transcription call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TranscriptionRequest {
    /// Raw request HTTP body that was sent (JSON stringified).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Response information for a transcription call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TranscriptionResponse {
    /// Timestamp for the start of the generated response (ISO 8601 string).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// The ID of the model that was used to generate the response.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Response headers.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<SharedHeaders>,
    /// Response body (opaque JSON).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// The input audio format for a streaming transcription request.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InputAudioFormat {
    /// Audio format type, e.g. `"audio/pcm"`, `"audio/pcmu"`, `"audio/pcma"`.
    pub format_type: String,
    /// Sample rate in Hz. Only applicable for formats that require a rate.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

    /// Timeout configuration (RFC-0028): `first_chunk_ms` bounds connect +
    /// session establishment, `chunk_ms` the gap between events, `total_ms`
    /// the whole stream. `None` = no timeouts (waits are unbounded apart from
    /// abort).
    pub timeout: Option<crate::options::TimeoutConfiguration>,
}

impl std::fmt::Debug for TranscriptionStreamOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptionStreamOptions")
            .field("input_audio_format", &self.input_audio_format)
            .field("provider_options", &self.provider_options)
            .field("abort_signal", &self.abort_signal)
            .field("headers", &self.headers)
            .field("include_raw_chunks", &self.include_raw_chunks)
            .field("timeout", &self.timeout)
            .field("audio", &"<stream>")
            .finish()
    }
}

/// A single chunk in the stream returned by `do_stream`.
///
/// Aligned with V4 `TranscriptionModelV4StreamPart`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TranscriptionStreamPart {
    /// Stream start event, carrying warnings.
    StreamStart { warnings: Vec<Warning> },
    /// Append-only transcript delta.
    TranscriptDelta {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        delta: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Non-final transcript text (may be revised by later parts).
    TranscriptPartial {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        text: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_second: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_in_seconds: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_index: Option<u32>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Final transcript text for a provider-defined segment or utterance.
    TranscriptFinal {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        text: String,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_second: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_second: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_index: Option<u32>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<SharedProviderMetadata>,
    },
    /// Response metadata, emitted once available.
    ResponseMetadata {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<SharedHeaders>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<serde_json::Value>,
    },
    /// Metadata available after the stream finishes.
    Finish {
        text: String,
        segments: Vec<TranscriptionSegment>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_in_seconds: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// Default implementation returns [`AiMuxError::UnsupportedFunctionality`]; providers
    /// override it as needed. This mirrors the optional `doStream?` in the TS
    /// spec.
    async fn do_stream(
        &self,
        _options: TranscriptionStreamOptions,
    ) -> Result<TranscriptionStreamResult, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "transcription streaming is not supported by provider `{}`",
            self.provider()
        )))
    }
}

/// User-facing non-streaming transcription with Core-owned retry and timeout.
///
/// # Errors
///
/// Returns the provider failure, retry exhaustion, timeout, or caller abort.
pub async fn transcribe(
    model: &dyn TranscriptionModel,
    options: TranscriptionCallOptions,
) -> Result<TranscriptionResult, AiMuxError> {
    let timeout = timeout::OperationTimeout::new(options.timeout.unwrap_or_default())?;
    let abort_signal = options.abort_signal.clone();
    let retries = retry::prepare_retries(options.max_retries, abort_signal.clone());
    timeout::run(
        retries.retry(|| model.do_generate(&options)),
        abort_signal.as_ref(),
        timeout,
    )
    .await
}

/// Start user-facing live transcription.
///
/// The live audio stream cannot be replayed, so session setup is attempted
/// once. The provider session applies the supplied abort signal and streaming
/// timeouts throughout connect, send, and receive.
///
/// # Errors
///
/// Returns the provider's session-establishment error.
pub async fn stream_transcribe(
    model: &dyn TranscriptionModel,
    options: TranscriptionStreamOptions,
) -> Result<TranscriptionStreamResult, AiMuxError> {
    model.do_stream(options).await
}
