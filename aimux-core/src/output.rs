//! Structured output of a text generation: the AI SDK's `Output`
//! specifications (ai@7.0.127 `generate-text/output.ts`).
//!
//! An [`Output`] decides the `response_format` sent to the model and how the
//! final step's text is parsed into the result's `output`. `generate_text` and
//! `stream_text` take it as [`crate::generate::GenerateTextOptions::output`];
//! without one the text output applies and `output` is the text.
//!
//! Parse failures follow the AI SDK's `NoObjectGeneratedError` messages
//! ("No object generated: could not parse the response." and "No object
//! generated: response did not match schema."), carried by the Aimux variants
//! for the same two cases: [`AiMuxError::JsonParse`] for text that is not
//! JSON and [`AiMuxError::InvalidResponseData`] for JSON of the wrong shape.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::options::ResponseFormat;

/// What the final text of a generation is parsed into.
///
/// Upstream's `Output` is a JavaScript object with functions and has no JSON
/// form; aimux's JSON follows the public JSON rules: tagged by `type` with the
/// output kind (upstream's mode `name`, `output.ts:68`, `:114`, `:241`, `:520`, `:657`;
/// the field `name` stays the schema name), and the arguments of
/// `Output.object()` / `array()` / `choice()` / `json()` camelCase
/// (`{"type":"array","element":{...},"minItems":1}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Output {
    /// The text itself (`Output.text()`).
    Text {},
    /// A JSON object matching `schema` (`Output.object()`).
    Object {
        schema: Value,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// An array of elements matching `element` (`Output.array()`). The model
    /// is asked for `{"elements": [...]}`.
    Array {
        element: Value,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_items: Option<u32>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// One of `options` (`Output.choice()`). The model is asked for
    /// `{"result": "<option>"}`.
    Choice {
        options: Vec<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// Any JSON value, unconstrained (`Output.json()`).
    Json {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

const JSON_SCHEMA_DRAFT_07: &str = "http://json-schema.org/draft-07/schema#";
const COULD_NOT_PARSE: &str = "No object generated: could not parse the response.";
const DID_NOT_MATCH: &str = "No object generated: response did not match schema.";

impl Output {
    /// Check the specification before any request is made: `min_items` must
    /// not exceed `max_items` (`output.ts` `array()`).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an array whose `min_items` exceeds `max_items`.
    pub fn validate(&self) -> Result<(), AiMuxError> {
        if let Self::Array {
            min_items: Some(min_items),
            max_items: Some(max_items),
            ..
        } = self
            && min_items > max_items
        {
            return Err(AiMuxError::InvalidArgument(
                "minItems must be less than or equal to maxItems".into(),
            ));
        }
        Ok(())
    }

    /// The `response_format` the model is called with.
    #[must_use]
    pub fn response_format(&self) -> ResponseFormat {
        match self {
            Self::Text {} => ResponseFormat::Text,
            Self::Object {
                schema,
                name,
                description,
            } => ResponseFormat::Json {
                schema: Some(schema.clone()),
                name: name.clone(),
                description: description.clone(),
            },
            Self::Array {
                element,
                min_items,
                max_items,
                name,
                description,
            } => {
                // The element schema loses `$schema`; its `definitions` /
                // `$defs` move up to the wrapper (output.ts `array()`).
                let mut item_schema = element.as_object().cloned().unwrap_or_default();
                item_schema.remove("$schema");
                let definitions = item_schema.remove("definitions");
                let defs = item_schema.remove("$defs");
                let mut elements = Map::new();
                elements.insert("type".into(), json!("array"));
                elements.insert("items".into(), Value::Object(item_schema));
                if let Some(min_items) = min_items {
                    elements.insert("minItems".into(), json!(min_items));
                }
                if let Some(max_items) = max_items {
                    elements.insert("maxItems".into(), json!(max_items));
                }
                let mut schema = Map::new();
                schema.insert("$schema".into(), json!(JSON_SCHEMA_DRAFT_07));
                if let Some(definitions) = definitions {
                    schema.insert("definitions".into(), definitions);
                }
                if let Some(defs) = defs {
                    schema.insert("$defs".into(), defs);
                }
                schema.insert("type".into(), json!("object"));
                schema.insert("properties".into(), json!({ "elements": elements }));
                schema.insert("required".into(), json!(["elements"]));
                schema.insert("additionalProperties".into(), json!(false));
                ResponseFormat::Json {
                    schema: Some(Value::Object(schema)),
                    name: name.clone(),
                    description: description.clone(),
                }
            }
            Self::Choice {
                options,
                name,
                description,
            } => ResponseFormat::Json {
                schema: Some(json!({
                    "$schema": JSON_SCHEMA_DRAFT_07,
                    "type": "object",
                    "properties": { "result": { "type": "string", "enum": options } },
                    "required": ["result"],
                    "additionalProperties": false,
                })),
                name: name.clone(),
                description: description.clone(),
            },
            Self::Json { name, description } => ResponseFormat::Json {
                schema: None,
                name: name.clone(),
                description: description.clone(),
            },
        }
    }

    /// Parse the final text into the output value (`parseCompleteOutput`).
    ///
    /// # Errors
    ///
    /// `JsonParse` when the text is not JSON, `InvalidResponseData` when the
    /// JSON does not have the requested shape; both messages start with the
    /// AI SDK's `NoObjectGeneratedError` message.
    pub fn parse_complete_output(&self, text: &str) -> Result<Value, AiMuxError> {
        let schema_mismatch =
            |cause: String| AiMuxError::InvalidResponseData(format!("{DID_NOT_MATCH} {cause}"));
        match self {
            Self::Text {} => Ok(Value::String(text.to_owned())),
            Self::Json { .. } => parse_json(text),
            Self::Object { schema, .. } => {
                let value = parse_json(text)?;
                validate(&value, schema).map_err(schema_mismatch)?;
                Ok(value)
            }
            Self::Array {
                element,
                min_items,
                max_items,
                ..
            } => {
                let value = parse_json(text)?;
                let Some(elements) = value.get("elements").and_then(Value::as_array) else {
                    return Err(schema_mismatch(type_validation_failed(
                        &value,
                        "response must be an object with an elements array",
                    )));
                };
                if let Some(min_items) = min_items
                    && elements.len() < *min_items as usize
                {
                    return Err(schema_mismatch(type_validation_failed(
                        &Value::Array(elements.clone()),
                        &format!("elements array must contain at least {min_items} items"),
                    )));
                }
                if let Some(max_items) = max_items
                    && elements.len() > *max_items as usize
                {
                    return Err(schema_mismatch(type_validation_failed(
                        &Value::Array(elements.clone()),
                        &format!("elements array must contain at most {max_items} items"),
                    )));
                }
                for item in elements {
                    validate(item, element).map_err(schema_mismatch)?;
                }
                Ok(Value::Array(elements.clone()))
            }
            Self::Choice { options, .. } => {
                let value = parse_json(text)?;
                match value.get("result").and_then(Value::as_str) {
                    Some(result) if options.iter().any(|option| option == result) => {
                        Ok(Value::String(result.to_owned()))
                    }
                    _ => Err(schema_mismatch(type_validation_failed(
                        &value,
                        "response must be an object that contains a choice value.",
                    ))),
                }
            }
        }
    }
}

/// The AI SDK's `safeParseJSON` (`@ai-sdk/provider-utils` `parse-json.ts`):
/// JSON with the prototype-poisoning guard of `secure-json-parse`.
fn parse_json(text: &str) -> Result<Value, AiMuxError> {
    let parse_failed = |message: String| {
        AiMuxError::JsonParse(format!(
            "{COULD_NOT_PARSE} JSON parsing failed: Text: {text}.\nError message: {message}"
        ))
    };
    let value: Value =
        serde_json::from_str(text).map_err(|error| parse_failed(error.to_string()))?;
    if crate::parse_tool_call::contains_forbidden_prototype(&value) {
        return Err(parse_failed(
            "Object contains forbidden prototype property".into(),
        ));
    }
    Ok(value)
}

/// The AI SDK's `TypeValidationError` message
/// (`@ai-sdk/provider` `errors/type-validation-error.ts`).
fn type_validation_failed(value: &Value, cause: &str) -> String {
    format!("Type validation failed: Value: {value}.\nError message: {cause}")
}

/// Validate a value against a JSON schema, as tool inputs are validated
/// (`parse_tool_call`).
fn validate(value: &Value, schema: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| type_validation_failed(value, &format!("schema is invalid: {error}")))?;
    validator
        .validate(value)
        .map_err(|error| type_validation_failed(value, &error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_output() -> Output {
        Output::Object {
            schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
            name: Some("person".into()),
            description: None,
        }
    }

    /// output.ts `text()`: response format `{type: 'text'}`, output is the text.
    #[test]
    fn text_output_is_the_text() {
        let output = Output::Text {};
        assert!(matches!(output.response_format(), ResponseFormat::Text));
        assert_eq!(output.parse_complete_output("hi").unwrap(), json!("hi"));
    }

    /// output.ts `object()`: the schema, name and description go into the
    /// response format; the text must parse and match the schema.
    #[test]
    fn object_output_parses_and_validates() {
        let output = object_output();
        let ResponseFormat::Json { schema, name, .. } = output.response_format() else {
            panic!("object output asks for JSON");
        };
        assert_eq!(schema.unwrap()["required"], json!(["name"]));
        assert_eq!(name.as_deref(), Some("person"));

        assert_eq!(
            output.parse_complete_output(r#"{"name":"Ada"}"#).unwrap(),
            json!({"name": "Ada"})
        );
        let error = output.parse_complete_output("{not json").unwrap_err();
        assert!(
            matches!(&error, AiMuxError::JsonParse(message)
                if message.starts_with("No object generated: could not parse the response.")),
            "{error:?}"
        );
        let error = output.parse_complete_output(r#"{"age":3}"#).unwrap_err();
        assert!(
            matches!(&error, AiMuxError::InvalidResponseData(message)
                if message.starts_with("No object generated: response did not match schema.")),
            "{error:?}"
        );
    }

    /// output.ts `array()`: the element schema is wrapped in
    /// `{elements: {type: array, items, minItems?, maxItems?}}`; `$schema` is
    /// dropped from the items and `$defs` move to the wrapper.
    #[test]
    fn array_output_wraps_the_element_schema() {
        let output = Output::Array {
            element: json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "$defs": { "n": { "type": "number" } },
                "type": "number",
            }),
            min_items: Some(1),
            max_items: Some(2),
            name: None,
            description: None,
        };
        let ResponseFormat::Json { schema, .. } = output.response_format() else {
            panic!("array output asks for JSON");
        };
        assert_eq!(
            schema.unwrap(),
            json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "$defs": { "n": { "type": "number" } },
                "type": "object",
                "properties": {
                    "elements": {
                        "type": "array",
                        "items": { "type": "number" },
                        "minItems": 1,
                        "maxItems": 2,
                    }
                },
                "required": ["elements"],
                "additionalProperties": false,
            })
        );
        assert_eq!(
            output
                .parse_complete_output(r#"{"elements":[1,2]}"#)
                .unwrap(),
            json!([1, 2])
        );
        for text in [
            r#"{"elements":[]}"#,
            r#"{"elements":[1,2,3]}"#,
            r#"[1]"#,
            r#"{"elements":["x"]}"#,
        ] {
            assert!(
                matches!(
                    output.parse_complete_output(text),
                    Err(AiMuxError::InvalidResponseData(_))
                ),
                "{text}"
            );
        }
    }

    /// output.ts `array()` rejects `minItems > maxItems` before the call.
    #[test]
    fn array_bounds_are_checked_up_front() {
        let output = Output::Array {
            element: json!({}),
            min_items: Some(3),
            max_items: Some(2),
            name: None,
            description: None,
        };
        assert!(matches!(
            output.validate(),
            Err(AiMuxError::InvalidArgument(message))
                if message == "minItems must be less than or equal to maxItems"
        ));
    }

    /// output.ts `choice()`: `{result: enum}`; the result must be one of the
    /// options.
    #[test]
    fn choice_output_returns_the_chosen_option() {
        let output = Output::Choice {
            options: vec!["yes".into(), "no".into()],
            name: None,
            description: None,
        };
        let ResponseFormat::Json { schema, .. } = output.response_format() else {
            panic!("choice output asks for JSON");
        };
        assert_eq!(
            schema.unwrap()["properties"]["result"],
            json!({"type": "string", "enum": ["yes", "no"]})
        );
        assert_eq!(
            output.parse_complete_output(r#"{"result":"no"}"#).unwrap(),
            json!("no")
        );
        assert!(matches!(
            output.parse_complete_output(r#"{"result":"maybe"}"#),
            Err(AiMuxError::InvalidResponseData(_))
        ));
    }

    /// output.ts `json()`: no schema in the response format; any JSON parses.
    #[test]
    fn json_output_accepts_any_json() {
        let output = Output::Json {
            name: None,
            description: None,
        };
        assert!(matches!(
            output.response_format(),
            ResponseFormat::Json { schema: None, .. }
        ));
        assert_eq!(
            output.parse_complete_output("[1,null]").unwrap(),
            json!([1, null])
        );
        assert!(matches!(
            output.parse_complete_output(r#"{"__proto__":{}}"#),
            Err(AiMuxError::JsonParse(_))
        ));
    }
}
