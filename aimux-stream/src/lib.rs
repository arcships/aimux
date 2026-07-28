//! # aimux-stream
//!
//! Low-level streaming primitives for SSE (Server-Sent Events) and NDJSON parsing,
//! used by provider implementations to decode model API response streams.

pub mod lines;
pub mod ndjson;
pub mod sse;
pub mod streaming_tool_call_tracker;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use ndjson::{NdjsonError, NdjsonStream};
pub use sse::{SseError, SseEvent, SseStream};
pub use streaming_tool_call_tracker::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker,
    ToolCallStreamPart, TrackerError, TypeValidation,
};
