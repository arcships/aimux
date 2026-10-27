//! The prompt of a text completion (`convert-to-openai-compatible-completion-prompt.ts`).

use aimux_core::AiMuxError;
use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, UserPart};
use aimux_core::options::CallOptions;

pub(super) fn completion_prompt(options: &CallOptions) -> Result<String, AiMuxError> {
    let mut text = String::new();
    for (index, message) in options.prompt.iter().enumerate() {
        match message {
            LanguageModelMessage::System { content, .. } if index == 0 => {
                text.push_str(content);
                text.push_str("\n\n");
            }
            LanguageModelMessage::System { .. } => {
                return Err(AiMuxError::invalid_prompt(
                    "Unexpected system message in prompt",
                ));
            }
            LanguageModelMessage::User { content, .. } => {
                text.push_str("user:\n");
                for part in content {
                    if let UserPart::Text(part) = part {
                        text.push_str(&part.text);
                    }
                }
                text.push_str("\n\n");
            }
            LanguageModelMessage::Assistant { content, .. } => {
                text.push_str("assistant:\n");
                for part in content {
                    match part {
                        AssistantPart::Text(part) => text.push_str(&part.text),
                        AssistantPart::ToolCall(_) => {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "tool-call messages".into(),
                            ));
                        }
                        _ => {}
                    }
                }
                text.push_str("\n\n");
            }
            LanguageModelMessage::Tool { .. } => {
                return Err(AiMuxError::UnsupportedFunctionality("tool messages".into()));
            }
        }
    }
    text.push_str("assistant:\n");
    Ok(text)
}
