//! Google internal exports (`@ai-sdk/google/internal`, `src/internal/index.ts`),
//! consumed by the sibling `vertex` package.

pub(crate) use super::error::google_failed_response_handler;
pub(crate) use super::language_model::build_vertex_request_body;
pub(crate) use super::options::{GOOGLE, Namespace, VERTEX_VIDEO_METADATA_KEYS};
pub(crate) use super::provider::list_models_once;
pub(crate) use super::transcription::{TranscriptionOptions, parse_offset_seconds};
#[cfg(feature = "realtime")]
pub(crate) use super::transcription::{live_stream, validate_live_input_audio_format};
pub(crate) use super::{GoogleModel, GoogleSpeechModel};
