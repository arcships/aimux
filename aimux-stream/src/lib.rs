//! # aimux-stream
//!
//! Low-level SSE (Server-Sent Events) decoding, used by
//! `aimux-provider-utils` to decode model API response streams. The crate
//! plays the role `eventsource-parser` plays for the AI SDK: it knows nothing
//! about providers, models or stream parts.

pub mod lines;
pub mod sse;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use sse::{SseError, SseEvent, SseStream};
