//! Tool registration and management.

use aimux_core::tool::FunctionTool;
use std::collections::HashMap;

/// A collection of tools that can be offered to the model.
#[derive(Debug, Clone, Default)]
pub struct ToolSet {
    tools: HashMap<String, FunctionTool>,
}

impl ToolSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: FunctionTool) {
        self.tools.insert(tool.name.clone(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&FunctionTool> {
        self.tools.get(name)
    }

    pub fn to_vec(&self) -> Vec<FunctionTool> {
        self.tools.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl FromIterator<FunctionTool> for ToolSet {
    fn from_iter<I: IntoIterator<Item = FunctionTool>>(iter: I) -> Self {
        let tools = iter.into_iter().map(|t| (t.name.clone(), t)).collect();
        Self { tools }
    }
}
