//! JSON Schema sanitization for Anthropic's constrained decoder.
//!
//! Removes JSON Schema keywords that Anthropic rejects in
//! `output_config.format.schema`. The full original schema is still used by
//! the AI SDK result validation; this only relaxes the schema sent to
//! Anthropic's constrained decoder.
//!
//! Ported from `packages/anthropic/src/sanitize-json-schema.ts`.

use serde_json::{Map, Value, json};

/// String formats that Anthropic's constrained decoder accepts.
const SUPPORTED_STRING_FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "uri",
    "ipv4",
    "ipv6",
    "uuid",
];

/// Constraint keys that are stripped from the schema but surfaced in a
/// human-readable description.
const DESCRIPTION_CONSTRAINT_KEYS: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minProperties",
    "maxProperties",
    "not",
];

/// Sanitize a JSON Schema, removing keywords Anthropic rejects and adding
/// readable descriptions for stripped constraints.
pub fn sanitize_json_schema(schema: &Value) -> Value {
    sanitize_schema(schema)
}

fn sanitize_definition(definition: &Value) -> Value {
    if !definition.is_object() {
        // Booleans and non-objects pass through unchanged.
        return definition.clone();
    }
    sanitize_schema(definition)
}

fn sanitize_schema(schema: &Value) -> Value {
    let obj = match schema.as_object() {
        Some(o) => o,
        None => return schema.clone(),
    };

    let mut result = Map::new();

    if let Some(r) = obj.get("$ref")
        && !r.is_null()
    {
        return json!({ "$ref": r.clone() });
    }

    copy_if_present(&mut result, obj, "$schema");
    copy_if_present(&mut result, obj, "$id");
    copy_if_present(&mut result, obj, "title");
    copy_if_present(&mut result, obj, "description");
    copy_if_present(&mut result, obj, "default");
    copy_if_present(&mut result, obj, "const");
    copy_if_present(&mut result, obj, "enum");
    copy_if_present(&mut result, obj, "type");

    // anyOf, or oneOf converted to anyOf.
    if let Some(any_of) = obj.get("anyOf").and_then(|v| v.as_array()) {
        result.insert(
            "anyOf".to_string(),
            Value::Array(any_of.iter().map(sanitize_definition).collect()),
        );
    } else if let Some(one_of) = obj.get("oneOf").and_then(|v| v.as_array()) {
        result.insert(
            "anyOf".to_string(),
            Value::Array(one_of.iter().map(sanitize_definition).collect()),
        );
    }

    if let Some(all_of) = obj.get("allOf").and_then(|v| v.as_array()) {
        result.insert(
            "allOf".to_string(),
            Value::Array(all_of.iter().map(sanitize_definition).collect()),
        );
    }

    if let Some(defs) = obj.get("definitions").and_then(|v| v.as_object()) {
        let mut mapped = Map::new();
        for (name, def) in defs {
            mapped.insert(name.clone(), sanitize_definition(def));
        }
        result.insert("definitions".to_string(), Value::Object(mapped));
    }

    if let Some(defs) = obj.get("$defs").and_then(|v| v.as_object()) {
        let mut mapped = Map::new();
        for (name, def) in defs {
            mapped.insert(name.clone(), sanitize_definition(def));
        }
        result.insert("$defs".to_string(), Value::Object(mapped));
    }

    let is_object = obj.get("type").and_then(|t| t.as_str()) == Some("object")
        || obj.get("properties").is_some();
    if is_object {
        if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
            let mut mapped = Map::new();
            for (name, def) in props {
                mapped.insert(name.clone(), sanitize_definition(def));
            }
            result.insert("properties".to_string(), Value::Object(mapped));
        }
        result.insert("additionalProperties".to_string(), Value::Bool(false));
        if let Some(req) = obj.get("required") {
            result.insert("required".to_string(), req.clone());
        }
    }

    if let Some(items) = obj.get("items") {
        let sanitized = match items.as_array() {
            Some(arr) => Value::Array(arr.iter().map(sanitize_definition).collect()),
            None => sanitize_definition(items),
        };
        result.insert("items".to_string(), sanitized);
    }

    if let Some(format) = obj.get("format").and_then(|v| v.as_str())
        && SUPPORTED_STRING_FORMATS.contains(&format)
    {
        result.insert("format".to_string(), Value::String(format.to_string()));
    }

    if let Some(desc) = get_constraint_description(obj) {
        let existing = result.get("description").and_then(|v| v.as_str());
        let merged = match existing {
            Some(prev) => format!("{prev}\n{desc}"),
            None => desc,
        };
        result.insert("description".to_string(), Value::String(merged));
    }

    Value::Object(result)
}

fn copy_if_present(result: &mut Map<String, Value>, obj: &Map<String, Value>, key: &str) {
    // Mirrors the TS `!== undefined` / `!= null` guards. `$schema`, `$id`,
    // `title`, `description`, `enum`, `type` use `!= null` (skip null); `default`
    // and `const` use `!== undefined` (copy even null, but JSON has no
    // undefined so always copy when present).
    if let Some(v) = obj.get(key) {
        if key != "default" && key != "const" && v.is_null() {
            return;
        }
        result.insert(key.to_string(), v.clone());
    }
}

fn get_constraint_description(obj: &Map<String, Value>) -> Option<String> {
    let mut descriptions: Vec<String> = Vec::new();

    for key in DESCRIPTION_CONSTRAINT_KEYS {
        let value = obj.get(*key);
        if value.is_none() {
            continue;
        }
        let value = value.unwrap();
        if value.is_null() || value.is_boolean() && !value.as_bool().unwrap_or(false) {
            continue;
        }
        descriptions.push(format!(
            "{}: {}",
            format_constraint_name(key),
            format_constraint_value(value)
        ));
    }

    if let Some(format) = obj.get("format").and_then(|v| v.as_str())
        && !SUPPORTED_STRING_FORMATS.contains(&format)
    {
        descriptions.push(format!("format: {format}"));
    }

    if descriptions.is_empty() {
        None
    } else {
        Some(format!("{}.", descriptions.join("; ")))
    }
}

fn format_constraint_name(key: &str) -> String {
    let mut out = String::new();
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push(' ');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn format_constraint_value(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        s.to_string()
    } else {
        // JSON.stringify the value (numbers/booleans/null).
        match value {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Null => "null".to_string(),
            other => other.to_string(),
        }
    }
}
