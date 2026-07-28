//! # aimux-tools
//!
//! Tool / function-calling support for aimux.

pub mod schema;
pub mod tool_executor;
pub mod tool_set;

pub use tool_executor::{ToolExecutor, ToolFn};
pub use tool_set::ToolSet;
