//! # aimux-stream
//!
//! Low-level streaming primitives for SSE (Server-Sent Events) parsing and
//! streamed tool-call tracking,
//! used by provider implementations to decode model API response streams.

pub mod lines;
pub mod sse;
pub mod streaming_tool_call_argument_state;
pub mod streaming_tool_call_tracker;

// Re-export the most commonly used items.
pub use lines::extract_lines;
pub use sse::{SseError, SseEvent, SseStream};
pub use streaming_tool_call_argument_state::{
    StreamingToolCallArgumentState, starts_with_structured_value,
};
pub use streaming_tool_call_tracker::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker,
    ToolCallStreamPart, TrackerError, TypeValidation,
};
