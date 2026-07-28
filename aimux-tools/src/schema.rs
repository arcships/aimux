//! Helpers for building JSON Schema for tool parameters.

use serde_json::{Value, json};

/// Builder for a JSON Schema object.
#[derive(Debug, Clone)]
pub struct JsonSchemaBuilder {
    properties: Vec<(String, Value)>,
    required: Vec<String>,
}

impl JsonSchemaBuilder {
    pub fn new() -> Self {
        Self {
            properties: Vec::new(),
            required: Vec::new(),
        }
    }

    /// Add a required string property.
    pub fn required_string(mut self, name: &str, description: &str) -> Self {
        self.properties.push((
            name.to_string(),
            json!({
                "type": "string",
                "description": description,
            }),
        ));
        self.required.push(name.to_string());
        self
    }

    /// Add an optional string property.
    pub fn optional_string(mut self, name: &str, description: &str) -> Self {
        self.properties.push((
            name.to_string(),
            json!({
                "type": "string",
                "description": description,
            }),
        ));
        self
    }

    /// Add a required number property.
    pub fn required_number(mut self, name: &str, description: &str) -> Self {
        self.properties.push((
            name.to_string(),
            json!({
                "type": "number",
                "description": description,
            }),
        ));
        self.required.push(name.to_string());
        self
    }

    /// Add a required boolean property.
    pub fn required_bool(mut self, name: &str, description: &str) -> Self {
        self.properties.push((
            name.to_string(),
            json!({
                "type": "boolean",
                "description": description,
            }),
        ));
        self.required.push(name.to_string());
        self
    }

    /// Build the final JSON Schema.
    pub fn build(self) -> Value {
        let properties: Value = self
            .properties
            .into_iter()
            .collect::<serde_json::Map<String, Value>>()
            .into();

        json!({
            "type": "object",
            "properties": properties,
            "required": self.required,
            "additionalProperties": false,
        })
    }
}

impl Default for JsonSchemaBuilder {
    fn default() -> Self {
        Self::new()
    }
}
