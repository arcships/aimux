//! # aimux-stream
//!
//! Low-level SSE (Server-Sent Events) parsing, used by provider
//! implementations to decode model API response streams.

pub mod lines;
pub mod sse;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use sse::{SseError, SseEvent, SseStream};
