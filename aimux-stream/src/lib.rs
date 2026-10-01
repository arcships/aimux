//! # aimux-stream
//!
//! Low-level streaming primitives for SSE (Server-Sent Events) and NDJSON parsing,
//! used by provider implementations to decode model API response streams.

pub mod lines;
pub mod ndjson;
pub mod sse;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use ndjson::{NdjsonError, NdjsonStream};
pub use sse::{SseError, SseEvent, SseStream};
