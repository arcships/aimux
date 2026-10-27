use serde_json::json;

use crate::google::sanitize_response_json_schema::sanitize_response_json_schema;

/// upstream: @ai-sdk/google src/sanitize-response-json-schema.test.ts › replaces const with enum while preserving JSON Schema
#[test]
fn replaces_const_with_enum_while_preserving_json_schema() {
    let schema = json!({
        "type": "object",
        "properties": {
            "response": {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "type": { "type": "string", "const": "fruit" },
                        },
                        "required": ["type"],
                        "additionalProperties": false,
                    },
                ],
            },
        },
        "required": ["response"],
        "additionalProperties": false,
        "$defs": {
            "label": { "type": "string", "const": "produce" },
        },
    });

    assert_eq!(
        sanitize_response_json_schema(&schema),
        json!({
            "type": "object",
            "properties": {
                "response": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "enum": ["fruit"] },
                            },
                            "required": ["type"],
                            "additionalProperties": false,
                        },
                    ],
                },
            },
            "required": ["response"],
            "additionalProperties": false,
            "$defs": {
                "label": { "type": "string", "enum": ["produce"] },
            },
        })
    );

    // toMatchObject: the input schema is left unchanged.
    assert_eq!(
        schema["properties"]["response"]["oneOf"][0]["properties"]["type"],
        json!({ "type": "string", "const": "fruit" })
    );
    assert_eq!(
        schema["properties"]["response"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}
