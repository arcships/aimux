//! Model identifier helpers.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A model identifier in `"provider/model-name"` format, e.g. `"openai/gpt-4o"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId {
    provider: String,
    model: String,
}

impl ModelId {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

impl std::str::FromStr for ModelId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (provider, model) = s
            .split_once('/')
            .ok_or_else(|| format!("expected 'provider/model', got '{s}'"))?;
        Ok(Self::new(provider, model))
    }
}
