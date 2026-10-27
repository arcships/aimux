//! Older ports of `sanitize-json-schema.test.ts` (previously in
//! `tests/anthropic_remaining_test.rs`).

use crate::anthropic::sanitize_json_schema::sanitize_json_schema;
use serde_json::json;

mod sanitize_json_schema {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › strips unsupported number constraints and adds readable descriptions
    #[test]
    fn sanitize_strips_unsupported_number_constraints() {
        let schema = json!({
            "type": "object",
            "properties": {
                "recurringIntervalMinutes": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "minimum": 1,
                    "maximum": 60,
                    "exclusiveMaximum": 120
                }
            },
            "required": ["recurringIntervalMinutes"],
            "additionalProperties": false
        });

        let expected = json!({
            "additionalProperties": false,
            "properties": {
                "recurringIntervalMinutes": {
                    "description":
                        "minimum: 1; maximum: 60; exclusive minimum: 0; exclusive maximum: 120.",
                    "type": "number"
                }
            },
            "required": ["recurringIntervalMinutes"],
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › strips unsupported string constraints and unsupported formats
    #[test]
    fn sanitize_strips_unsupported_string_constraints_and_formats() {
        let schema = json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "A URL slug",
                    "minLength": 1,
                    "maxLength": 20,
                    "pattern": "^[a-z0-9-]+$",
                    "format": "regex"
                }
            }
        });

        let expected = json!({
            "additionalProperties": false,
            "properties": {
                "slug": {
                    "description":
                        "A URL slug\nmin length: 1; max length: 20; pattern: ^[a-z0-9-]+$; format: regex.",
                    "type": "string"
                }
            },
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › recursively sanitizes arrays, definitions, and composition schemas
    #[test]
    fn sanitize_recursively_handles_arrays_defs_and_composition() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "PositiveInteger": { "type": "integer", "minimum": 1 }
            },
            "properties": {
                "count": { "$ref": "#/$defs/PositiveInteger" },
                "tags": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 4,
                    "uniqueItems": true,
                    "items": {
                        "anyOf": [
                            { "type": "string", "minLength": 1 },
                            { "type": "number", "maximum": 10 }
                        ]
                    }
                }
            }
        });

        let expected = json!({
            "$defs": {
                "PositiveInteger": {
                    "description": "minimum: 1.",
                    "type": "integer"
                }
            },
            "additionalProperties": false,
            "properties": {
                "count": { "$ref": "#/$defs/PositiveInteger" },
                "tags": {
                    "description": "min items: 2; max items: 4; unique items: true.",
                    "items": {
                        "anyOf": [
                            { "description": "min length: 1.", "type": "string" },
                            { "description": "maximum: 10.", "type": "number" }
                        ]
                    },
                    "type": "array"
                }
            },
            "type": "object"
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › converts oneOf to anyOf
    #[test]
    fn sanitize_converts_one_of_to_any_of() {
        let schema = json!({
            "oneOf": [
                { "type": "string", "minLength": 1 },
                { "type": "number", "minimum": 0 }
            ]
        });

        let expected = json!({
            "anyOf": [
                { "description": "min length: 1.", "type": "string" },
                { "description": "minimum: 0.", "type": "number" }
            ]
        });

        assert_eq!(sanitize_json_schema(&schema), expected);
    }

    /// upstream: @ai-sdk/anthropic src/sanitize-json-schema.test.ts › sanitizeJsonSchema › does not mutate the input schema
    #[test]
    fn sanitize_does_not_mutate_input_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "value": { "type": "number", "exclusiveMinimum": 0 }
            }
        });

        let snapshot = schema.clone();
        let _ = sanitize_json_schema(&schema);
        assert_eq!(schema, snapshot);
    }
}
