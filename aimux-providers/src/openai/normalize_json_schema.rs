//! Normalization of a JSON schema for OpenAI strict mode (normalize-openai-json-schema.ts).

use aimux_core::error::AiMuxError;
use aimux_core::types::Warning;
use serde_json::Value;

/// Apply the upstream structured-output compatibility rules only at schema positions.
pub(crate) fn normalize_json_schema(
    schema: &Value,
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    fn walk(value: &mut Value, removed: &mut [bool; 2], is_root: bool) -> Result<(), AiMuxError> {
        let Some(obj) = value.as_object_mut() else {
            return Ok(());
        };
        if let Some(names) = obj.get("propertyNames").filter(|v| !v.is_null()) {
            if names.get("type").and_then(Value::as_str) != Some("string") {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "JSON Schema propertyNames that does not use a string schema".into(),
                ));
            }
            obj.remove("propertyNames");
            removed[0] = true;
        }
        if obj
            .get("pattern")
            .and_then(Value::as_str)
            .is_some_and(contains_lookaround)
        {
            obj.remove("pattern");
            removed[1] = true;
        }
        for key in [
            "properties",
            "patternProperties",
            "definitions",
            "$defs",
            "dependencies",
        ] {
            if let Some(Value::Object(record)) = obj.get_mut(key) {
                for child in record.values_mut().filter(|v| !v.is_array()) {
                    walk(child, removed, false)?;
                }
            }
        }
        for key in [
            "additionalProperties",
            "additionalItems",
            "items",
            "contains",
            "not",
            "allOf",
            "anyOf",
            "oneOf",
            "if",
            "then",
            "else",
        ] {
            if let Some(child) = obj.get_mut(key) {
                if let Value::Array(children) = child {
                    for child in children {
                        walk(child, removed, false)?;
                    }
                } else {
                    walk(child, removed, false)?;
                }
            }
        }
        // Zod 4 writes recursive references as `allOf: [{ $ref }]`; OpenAI
        // has no `allOf`, so a singleton local reference becomes a direct
        // `$ref`, expanded at the root where OpenAI requires an object.
        let reference = match obj.get("allOf") {
            Some(Value::Array(all_of)) if all_of.len() == 1 => all_of[0]
                .as_object()
                .filter(|item| item.len() == 1)
                .and_then(|item| item.get("$ref"))
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        };
        if let Some(reference) = reference {
            if !is_root {
                obj.remove("allOf");
                obj.insert("$ref".into(), Value::String(reference));
            } else if let Some(Value::Object(referenced)) = local_reference(&reference, obj) {
                let mut expanded = referenced.clone();
                obj.remove("allOf");
                expanded.extend(std::mem::take(obj));
                *obj = expanded;
            }
        }
        Ok(())
    }
    /// `#/definitions/<name>` or `#/$defs/<name>` in `schema`.
    fn local_reference<'a>(
        reference: &str,
        schema: &'a serde_json::Map<String, Value>,
    ) -> Option<&'a Value> {
        let (keyword, name) = reference
            .strip_prefix("#/definitions/")
            .map(|name| ("definitions", name))
            .or_else(|| {
                reference
                    .strip_prefix("#/$defs/")
                    .map(|name| ("$defs", name))
            })?;
        if name.is_empty() {
            return None;
        }
        let name = name.replace("~1", "/").replace("~0", "~");
        schema.get(keyword)?.get(&name)
    }
    let mut result = schema.clone();
    let mut removed = [false; 2];
    walk(&mut result, &mut removed, true)?;
    for (index, feature, details) in [
        (
            0,
            "JSON Schema propertyNames",
            "OpenAI does not support JSON Schema propertyNames. It was removed before sending the schema, so OpenAI will not enforce property-name constraints.",
        ),
        (
            1,
            "JSON Schema pattern with regex lookaround",
            "OpenAI does not support regex lookaround in JSON Schema patterns. The pattern was removed before sending the schema, so OpenAI will not enforce that constraint.",
        ),
    ] {
        if removed[index] {
            warnings.push(Warning::Compatibility {
                feature: feature.into(),
                details: Some(details.into()),
            });
        }
    }
    Ok(result)
}

fn contains_lookaround(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let (mut escaped, mut in_class) = (false, false);
    for (i, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'(' if !in_class
                && bytes.get(i + 1) == Some(&b'?')
                && (matches!(bytes.get(i + 2), Some(b'=' | b'!'))
                    || (bytes.get(i + 2) == Some(&b'<')
                        && matches!(bytes.get(i + 3), Some(b'=' | b'!')))) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}
