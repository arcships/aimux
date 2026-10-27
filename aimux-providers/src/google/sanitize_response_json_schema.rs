//! `sanitize-response-json-schema.ts`.

use serde_json::{Value, json};

/// Preserve JSON Schema while replacing unsupported `const` constraints.
#[must_use]
pub fn sanitize_response_json_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema.clone();
    };
    let mut result = object.clone();
    if let Some(value) = result.remove("const") {
        result.insert("enum".into(), json!([value]));
    }
    for key in ["properties", "$defs"] {
        if let Some(Value::Object(definitions)) = result.get_mut(key) {
            for value in definitions.values_mut() {
                *value = sanitize_response_json_schema(value);
            }
        }
    }
    for key in ["items", "additionalProperties", "anyOf", "oneOf"] {
        if let Some(value) = result.get_mut(key) {
            if let Value::Array(array) = value {
                for item in array {
                    *item = sanitize_response_json_schema(item);
                }
            } else {
                *value = sanitize_response_json_schema(value);
            }
        }
    }
    Value::Object(result)
}
