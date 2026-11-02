//! Google Gemini provider (`@ai-sdk/google`). One file per upstream file,
//! named after it without the `google-` prefix; this file is `index.ts`.
//!
//! [`create_google`] is the Rust form of the AI SDK's `createGoogle`: it takes
//! [`GoogleProviderSettings`], validates the base URL, fixes the provider name
//! and returns a [`GoogleProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `GOOGLE_GENERATIVE_AI_API_KEY`. [`google()`] is the default instance.
//!
//! The models speak Google's Generative Language API
//! (`generativelanguage.googleapis.com/v1beta`). providerOptions are read from
//! `providerOptions.google` and response metadata is written under `google`
//! (see `options`). Vertex AI serves the same request format and shares the
//! conversion in `convert_to_messages`.
//!
//! Gemini's API shape is fundamentally different from OpenAI/Anthropic:
//! - The model id is part of the URL path, not the request body.
//! - Authentication is via the `x-goog-api-key` header (or `?key=…`).
//! - System messages become a top-level `systemInstruction` field.
//! - The model role is `"model"` (not `"assistant"`).
//! - Tool calls are `functionCall` parts; tool results are
//!   `functionResponse` parts.
//! - Streaming is SSE with one JSON object per `data:` line.

mod api_types;
mod convert_to_messages;
mod convert_usage;
mod embedding_model;
mod error;
pub mod files;
mod get_model_path;
#[cfg(feature = "realtime")]
mod get_realtime_base_url;
mod image_model;
mod image_settings;
pub(crate) mod internal;
mod json_accumulator;
mod language_model;
mod language_model_options;
mod map_finish_reason;
mod model_capabilities;
mod options;
mod prepare_tools;
mod prompt;
mod provider;
mod sanitize_response_json_schema;
mod speech_api;
mod speech_input;
mod speech_model;
mod speech_model_options;
mod supported_file_url;
pub mod transcription;
mod video_model;

pub use embedding_model::GoogleEmbeddingModel;
pub use files::GoogleFiles;
pub use image_model::GoogleImageModel;
pub use image_settings::GoogleImageSettings;
pub use language_model::GoogleModel;
pub use provider::{GoogleProvider, GoogleProviderSettings, create_google, google};
pub use speech_model::GoogleSpeechModel;
pub use transcription::GoogleTranscriptionModel;
pub use video_model::GoogleVideoModel;

use error::google_failed_response_handler;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};

#[cfg(test)]
mod tests;
