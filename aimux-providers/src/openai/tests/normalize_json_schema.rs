use aimux_core::error::AiMuxError;
use serde_json::{Value, json};

use crate::openai::normalize_json_schema::normalize_json_schema;

/// `normalizeOpenAIJsonSchema(schema)` as `{ schema, warnings }`.
fn normalize(schema: &Value) -> Result<Value, AiMuxError> {
    let mut warnings = Vec::new();
    let schema = normalize_json_schema(schema, &mut warnings)?;
    Ok(json!({ "schema": schema, "warnings": warnings }))
}

mod normalize_open_ai_json_schema {
    use super::*;

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › removes string propertyNames recursively and warns
    #[test]
    fn removes_string_property_names_recursively_and_warns() {
        let schema = json!({
            "type": "object",
            "properties": {
                "variables": {
                    "type": "object",
                    "propertyNames": {"type": "string", "format": "uuid"},
                    "additionalProperties": {
                        "type": "object",
                        "propertyNames": {"type": "string", "pattern": "^[A-Z_]+$"},
                    },
                },
            },
            "definitions": {
                "variable": {"type": "object", "propertyNames": {"type": "string"}},
            },
            "$defs": {
                "conditional": {
                    "if": {"type": "object", "propertyNames": {"type": "string"}},
                },
            },
        });

        assert_eq!(
            normalize(&schema).unwrap(),
            json!({
                "schema": {
                    "type": "object",
                    "properties": {
                        "variables": {
                            "type": "object",
                            "additionalProperties": {"type": "object"},
                        },
                    },
                    "definitions": {"variable": {"type": "object"}},
                    "$defs": {"conditional": {"if": {"type": "object"}}},
                },
                "warnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema propertyNames",
                    "details": "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
                }],
            })
        );

        assert!(
            schema["properties"]["variables"]
                .get("propertyNames")
                .is_some()
        );
    }

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › rejects non-string propertyNames schemas
    #[test]
    fn rejects_non_string_property_names_schemas() {
        let result = normalize(&json!({
            "type": "object",
            "propertyNames": {"type": "number"},
        }));
        assert!(
            matches!(result, Err(AiMuxError::UnsupportedFunctionality(_))),
            "{result:?}"
        );
    }

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › removes regex lookaround patterns recursively and warns
    #[test]
    fn removes_regex_lookaround_patterns_recursively_and_warns() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "format": "email",
                    "pattern": "^(?!\\.)(?!.*\\.\\.).+@.+$",
                },
                "username": {"type": "string", "pattern": "^@[a-zA-Z0-9_]+$"},
                "contacts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "value": {"type": "string", "pattern": "(?<=prefix)value"},
                        },
                    },
                },
            },
            "$defs": {"value": {"type": "string", "pattern": "value(?=suffix)"}},
        });

        assert_eq!(
            normalize(&schema).unwrap(),
            json!({
                "schema": {
                    "type": "object",
                    "properties": {
                        "email": {"type": "string", "format": "email"},
                        "username": {"type": "string", "pattern": "^@[a-zA-Z0-9_]+$"},
                        "contacts": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {"value": {"type": "string"}},
                            },
                        },
                    },
                    "$defs": {"value": {"type": "string"}},
                },
                "warnings": [{
                    "type": "compatibility",
                    "feature": "JSON Schema pattern with regex lookaround",
                    "details": "OpenAI does not support regex lookaround in JSON Schema patterns. The pattern was removed before sending the schema, so OpenAI will not enforce that constraint.",
                }],
            })
        );

        assert!(schema["properties"]["email"].get("pattern").is_some());
    }

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › preserves escaped and character-class lookaround-like text
    #[test]
    fn preserves_escaped_and_character_class_lookaround_like_text() {
        let schema = json!({
            "type": "object",
            "properties": {
                "escaped": {"type": "string", "pattern": "\\(\\?=literal\\)"},
                "characterClass": {"type": "string", "pattern": "[(?=!)]"},
            },
        });

        assert_eq!(
            normalize(&schema).unwrap(),
            json!({ "schema": schema, "warnings": [] })
        );
    }

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › unwraps singleton reference allOf schemas from recursive Zod schemas
    #[test]
    fn unwraps_singleton_reference_all_of_schemas_from_recursive_zod_schemas() {
        let schema = json!({
            "type": "object",
            "properties": {
                "relatives": {
                    "type": "array",
                    "items": {"allOf": [{"$ref": "#/definitions/person"}]},
                },
            },
            "definitions": {
                "person": {
                    "type": "object",
                    "properties": {"firstName": {"type": "string"}},
                    "required": ["firstName"],
                    "additionalProperties": false,
                },
            },
            "required": ["relatives"],
            "additionalProperties": false,
        });

        assert_eq!(
            normalize(&schema).unwrap(),
            json!({
                "schema": {
                    "type": "object",
                    "properties": {
                        "relatives": {
                            "type": "array",
                            "items": {"$ref": "#/definitions/person"},
                        },
                    },
                    "definitions": schema["definitions"],
                    "required": ["relatives"],
                    "additionalProperties": false,
                },
                "warnings": [],
            })
        );
    }

    /// upstream: @ai-sdk/openai src/normalize-openai-json-schema.test.ts › normalizeOpenAIJsonSchema › expands a singleton local reference allOf at the root
    #[test]
    fn expands_a_singleton_local_reference_all_of_at_the_root() {
        let schema = json!({
            "default": {"firstName": "John"},
            "allOf": [{"$ref": "#/definitions/person"}],
            "definitions": {
                "person": {
                    "type": "object",
                    "properties": {
                        "firstName": {"type": "string"},
                        "relatives": {
                            "type": "array",
                            "items": {"allOf": [{"$ref": "#/definitions/person"}]},
                        },
                    },
                    "required": ["firstName", "relatives"],
                    "additionalProperties": false,
                },
            },
        });

        assert_eq!(
            normalize(&schema).unwrap(),
            json!({
                "schema": {
                    "type": "object",
                    "properties": {
                        "firstName": {"type": "string"},
                        "relatives": {
                            "type": "array",
                            "items": {"$ref": "#/definitions/person"},
                        },
                    },
                    "required": ["firstName", "relatives"],
                    "additionalProperties": false,
                    "default": {"firstName": "John"},
                    "definitions": {
                        "person": {
                            "type": "object",
                            "properties": {
                                "firstName": {"type": "string"},
                                "relatives": {
                                    "type": "array",
                                    "items": {"$ref": "#/definitions/person"},
                                },
                            },
                            "required": ["firstName", "relatives"],
                            "additionalProperties": false,
                        },
                    },
                },
                "warnings": [],
            })
        );
    }
}
