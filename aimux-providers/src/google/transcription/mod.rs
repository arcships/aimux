//! `transcription/`: Gemini speech-to-text, unary and live.

mod transcription_model;
mod transcription_model_options;

pub use transcription_model::GoogleTranscriptionModel;
#[cfg(feature = "realtime")]
pub(crate) use transcription_model::{live_stream, validate_live_input_audio_format};
pub(crate) use transcription_model_options::{TranscriptionOptions, parse_offset_seconds};
