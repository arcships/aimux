//! `transcription/`: the OpenAI transcription (STT) model, REST and realtime.

pub(super) mod transcription_api;
pub(super) mod transcription_model;
pub(super) mod transcription_model_options;

pub use transcription_model::OpenAITranscriptionModel;
