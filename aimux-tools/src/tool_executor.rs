//! Runtime dispatch of tool calls.

use async_trait::async_trait;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::tool::{ToolCall, ToolResult};

/// Trait for objects that can execute a specific tool call.
#[async_trait]
pub trait ToolFn: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, args: &Value) -> Result<Value, AiMuxError>;
}

/// Dispatches `ToolCall`s to the correct `ToolFn` implementation.
pub struct ToolExecutor {
    handlers: std::collections::HashMap<String, Box<dyn ToolFn>>,
}

impl ToolExecutor {
    pub fn new() -> Self {
        Self {
            handlers: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, handler: impl ToolFn + 'static) {
        let name = handler.name().to_string();
        self.handlers.insert(name, Box::new(handler));
    }

    pub async fn execute(&self, call: &ToolCall) -> Result<ToolResult, AiMuxError> {
        let handler = self.handlers.get(&call.tool_name).ok_or_else(|| {
            AiMuxError::Tool(format!(
                "no handler registered for tool '{}'",
                call.tool_name
            ))
        })?;
        let output = handler.execute(&call.input).await?;
        Ok(ToolResult {
            tool_call_id: call.tool_call_id.clone(),
            output,
        })
    }

    pub async fn execute_all(&self, calls: &[ToolCall]) -> Vec<Result<ToolResult, AiMuxError>> {
        let futures = calls.iter().map(|call| self.execute(call));
        futures::future::join_all(futures).await
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}
