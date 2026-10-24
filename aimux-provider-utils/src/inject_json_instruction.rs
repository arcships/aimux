//! The JSON instruction providers without native structured output add to
//! the system prompt (`@ai-sdk/provider-utils` `inject-json-instruction.ts`).

use aimux_core::language_model_message::LanguageModelMessage;
use serde_json::Value;

const DEFAULT_SCHEMA_PREFIX: &str = "JSON schema:";
const DEFAULT_SCHEMA_SUFFIX: &str =
    "You MUST answer with a JSON object that matches the JSON schema above.";
const DEFAULT_GENERIC_SUFFIX: &str = "You MUST answer with JSON.";

/// `prompt`, a blank line, then the schema between `schema_prefix` and
/// `schema_suffix`. A `None` prefix or suffix takes the AI SDK's default: with
/// a schema `JSON schema:` and "You MUST answer with a JSON object that matches
/// the JSON schema above.", without one no prefix and "You MUST answer with
/// JSON.". An empty prompt adds no blank line.
#[must_use]
pub fn inject_json_instruction(
    prompt: Option<&str>,
    schema: Option<&Value>,
    schema_prefix: Option<&str>,
    schema_suffix: Option<&str>,
) -> String {
    let prompt = prompt.filter(|prompt| !prompt.is_empty());
    let (default_prefix, default_suffix) = match schema {
        Some(_) => (Some(DEFAULT_SCHEMA_PREFIX), DEFAULT_SCHEMA_SUFFIX),
        None => (None, DEFAULT_GENERIC_SUFFIX),
    };
    let schema = schema.map(Value::to_string);
    [
        prompt,
        prompt.map(|_| ""),
        schema_prefix.or(default_prefix),
        schema.as_deref(),
        Some(schema_suffix.unwrap_or(default_suffix)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
}

/// `messages` with the instruction of [`inject_json_instruction`] appended to
/// the leading system message, or in a new system message put first when
/// there is none (`injectJsonInstructionIntoMessages`).
#[must_use]
pub fn inject_json_instruction_into_messages(
    messages: &[LanguageModelMessage],
    schema: Option<&Value>,
    schema_prefix: Option<&str>,
    schema_suffix: Option<&str>,
) -> Vec<LanguageModelMessage> {
    let (content, provider_options, rest) = match messages.split_first() {
        Some((
            LanguageModelMessage::System {
                content,
                provider_options,
            },
            rest,
        )) => (Some(content.as_str()), provider_options.clone(), rest),
        _ => (None, None, messages),
    };
    let system = LanguageModelMessage::System {
        content: inject_json_instruction(content, schema, schema_prefix, schema_suffix),
        provider_options,
    };
    std::iter::once(system)
        .chain(rest.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    //! Ports of `inject-json-instruction.test.ts` (vercel/ai fce9210); the
    //! `null as any` case is TypeScript-only.

    use super::*;
    use aimux_core::language_model_message::{AssistantPart, TextPart};
    use serde_json::json;

    fn basic_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}, "age": {"type": "number"}},
            "required": ["name", "age"]
        })
    }

    const BASIC: &str = "JSON schema:\n\
        {\"type\":\"object\",\"properties\":{\"name\":{\"type\":\"string\"},\"age\":{\"type\":\"number\"}},\"required\":[\"name\",\"age\"]}\n\
        You MUST answer with a JSON object that matches the JSON schema above.";

    fn assistant(text: &str) -> LanguageModelMessage {
        LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: text.into(),
                provider_options: None,
            })],
            provider_options: None,
        }
    }

    fn system(content: &str) -> LanguageModelMessage {
        LanguageModelMessage::System {
            content: content.into(),
            provider_options: None,
        }
    }

    #[test]
    fn prompt_and_schema() {
        let result =
            inject_json_instruction(Some("Generate a person"), Some(&basic_schema()), None, None);
        assert_eq!(result, format!("Generate a person\n\n{BASIC}"));
    }

    #[test]
    fn only_prompt() {
        let result = inject_json_instruction(Some("Generate a person"), None, None, None);
        assert_eq!(result, "Generate a person\n\nYou MUST answer with JSON.");
    }

    #[test]
    fn only_schema() {
        let result = inject_json_instruction(None, Some(&basic_schema()), None, None);
        assert_eq!(result, BASIC);
    }

    #[test]
    fn no_prompt_no_schema() {
        assert_eq!(
            inject_json_instruction(None, None, None, None),
            "You MUST answer with JSON."
        );
    }

    #[test]
    fn custom_prefix_and_suffix() {
        let result = inject_json_instruction(
            Some("Generate a person"),
            Some(&basic_schema()),
            Some("Custom prefix:"),
            Some("Custom suffix"),
        );
        assert_eq!(
            result,
            format!(
                "Generate a person\n\nCustom prefix:\n{}\nCustom suffix",
                basic_schema()
            )
        );
    }

    #[test]
    fn empty_prompt() {
        let result = inject_json_instruction(Some(""), Some(&basic_schema()), None, None);
        assert_eq!(result, BASIC);
    }

    #[test]
    fn empty_object_schema() {
        let result =
            inject_json_instruction(Some("Generate a person"), Some(&json!({})), None, None);
        assert_eq!(
            result,
            "Generate a person\n\nJSON schema:\n{}\nYou MUST answer with a JSON object that matches the JSON schema above."
        );
    }

    #[test]
    fn nested_schema_keeps_key_order_and_escapes() {
        let schema = json!({
            "type": "object",
            "properties": {
                "person": {"type": "object", "properties": {"name": {"type": "string"}, "address": {"type": "object", "properties": {"street": {"type": "string"}, "city": {"type": "string"}}}}},
                "special": {"type": "string", "description": "Quotes \" and newlines \n"}
            }
        });
        let result = inject_json_instruction(Some("Generate"), Some(&schema), None, None);
        assert_eq!(
            result,
            format!(
                "Generate\n\nJSON schema:\n{}\n{DEFAULT_SCHEMA_SUFFIX}",
                serde_json::to_string(&schema).unwrap()
            )
        );
        assert!(result.contains(r#"{"type":"object","properties":{"person""#));
        assert!(result.contains(r#"Quotes \" and newlines \n"#));
    }

    #[test]
    fn into_messages_appends_to_the_system_message() {
        let result = inject_json_instruction_into_messages(
            &[system("Generate a person")],
            Some(&basic_schema()),
            None,
            None,
        );
        assert_eq!(
            result,
            vec![system(&format!("Generate a person\n\n{BASIC}"))]
        );
    }

    #[test]
    fn into_messages_does_not_change_the_input() {
        let messages = vec![system("Generate a person")];
        let _ = inject_json_instruction_into_messages(&messages, Some(&basic_schema()), None, None);
        assert_eq!(messages, vec![system("Generate a person")]);
    }

    #[test]
    fn into_messages_with_no_messages() {
        let result = inject_json_instruction_into_messages(&[], Some(&basic_schema()), None, None);
        assert_eq!(result, vec![system(BASIC)]);
    }

    #[test]
    fn into_messages_puts_a_new_system_message_first() {
        let messages = vec![
            LanguageModelMessage::user_text("Hello"),
            assistant("Hi there"),
        ];
        let result =
            inject_json_instruction_into_messages(&messages, Some(&basic_schema()), None, None);
        assert_eq!(result[0], system(BASIC));
        assert_eq!(result[1..], messages[..]);
    }

    #[test]
    fn into_messages_with_an_empty_system_message() {
        let messages = vec![system(""), LanguageModelMessage::user_text("Generate data")];
        let result =
            inject_json_instruction_into_messages(&messages, Some(&basic_schema()), None, None);
        assert_eq!(result, vec![system(BASIC), messages[1].clone()]);
    }

    #[test]
    fn into_messages_keeps_the_other_messages() {
        let messages = vec![
            system("You are helpful"),
            LanguageModelMessage::user_text("Hello"),
            assistant("Hi"),
            LanguageModelMessage::user_text("Generate person"),
        ];
        let result =
            inject_json_instruction_into_messages(&messages, Some(&basic_schema()), None, None);
        assert_eq!(result[0], system(&format!("You are helpful\n\n{BASIC}")));
        assert_eq!(result[1..], messages[1..]);
    }

    #[test]
    fn into_messages_without_a_schema() {
        let result =
            inject_json_instruction_into_messages(&[system("Generate data")], None, None, None);
        assert_eq!(
            result,
            vec![system("Generate data\n\nYou MUST answer with JSON.")]
        );
    }

    #[test]
    fn into_messages_with_custom_prefix_and_suffix() {
        let result = inject_json_instruction_into_messages(
            &[system("Generate data")],
            Some(&basic_schema()),
            Some("Custom schema:"),
            Some("Follow this format exactly."),
        );
        assert_eq!(
            result,
            vec![system(&format!(
                "Generate data\n\nCustom schema:\n{}\nFollow this format exactly.",
                basic_schema()
            ))]
        );
    }

    #[test]
    fn into_messages_keeps_the_system_provider_options() {
        let options: aimux_core::shared::SharedProviderOptions =
            serde_json::from_value(json!({"anthropic": {"cacheControl": {"type": "ephemeral"}}}))
                .unwrap();
        let messages = vec![LanguageModelMessage::System {
            content: "Be brief".into(),
            provider_options: Some(options.clone()),
        }];
        let result = inject_json_instruction_into_messages(&messages, None, None, None);
        assert!(matches!(
            &result[0],
            LanguageModelMessage::System { provider_options: Some(kept), .. } if *kept == options
        ));
    }
}
