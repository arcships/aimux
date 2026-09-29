//! # aimux-stream
//!
//! Low-level streaming primitives for SSE (Server-Sent Events) parsing and
//! streamed tool-call tracking,
//! used by provider implementations to decode model API response streams.

pub mod lines;
pub mod sse;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use sse::{SseError, SseEvent, SseStream};
