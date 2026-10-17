//! Checks `generate_text` / `stream_text` make around the model call,
//! against the AI SDK (`ai@7.0.127`): the prompt standardization
//! (`prompt/standardize-prompt.ts`) and the enforced tool choice
//! (`generate-text/generate-text.ts:1155-1176`,
//! `generate-text/stream-language-model-call.ts:644-687`).

use aimux_core::error::{AiMuxError, ToolChoiceViolationError};
use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::{ModelMessage, ModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::{StreamPart, TextStreamPart};
use aimux_core::tool::{FunctionTool, LanguageModelV4ToolChoice, RawToolCall, Tool, ToolChoice};
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

/// A model that answers with fixed content: `do_generate` returns it as is,
/// `do_stream` as stream parts.
struct ScriptedModel {
    content: Vec<GenerateContent>,
}

fn finish_reason() -> FinishReason {
    FinishReason {
        unified: FinishReasonUnified::Stop,
        raw: Some("end_turn".into()),
    }
}

#[async_trait]
impl LanguageModel for ScriptedModel {
    fn provider(&self) -> &str {
        "scripted.chat"
    }
    fn model_id(&self) -> &str {
        "scripted-1"
    }
    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: self.content.clone(),
            finish_reason: finish_reason(),
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: None,
            request: None,
        })
    }
    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let mut parts = vec![StreamPart::StreamStart { warnings: vec![] }];
        for (index, content) in self.content.iter().enumerate() {
            let id = format!("b{index}");
            match content {
                GenerateContent::Text { text, .. } => {
                    parts.push(StreamPart::TextStart {
                        id: id.clone(),
                        provider_metadata: None,
                    });
                    for delta in [&text[..1], &text[1..]] {
                        parts.push(StreamPart::TextDelta {
                            id: id.clone(),
                            delta: delta.into(),
                            provider_metadata: None,
                        });
                    }
                    parts.push(StreamPart::TextEnd {
                        id,
                        provider_metadata: None,
                    });
                }
                GenerateContent::Reasoning(reasoning) => {
                    parts.push(StreamPart::ReasoningStart {
                        id: id.clone(),
                        provider_metadata: None,
                    });
                    parts.push(StreamPart::ReasoningDelta {
                        id: id.clone(),
                        delta: reasoning.text.clone(),
                        provider_metadata: None,
                    });
                    parts.push(StreamPart::ReasoningEnd {
                        id,
                        provider_metadata: reasoning.provider_metadata.clone(),
                    });
                }
                GenerateContent::ToolCall(call) => parts.push(StreamPart::ToolCall(call.clone())),
                other => panic!("not scripted: {other:?}"),
            }
        }
        parts.push(StreamPart::Finish {
            finish_reason: finish_reason(),
            usage: Usage::default(),
            provider_metadata: None,
        });
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts.into_iter().map(Ok))),
            request: None,
            response: None,
        })
    }
}

fn text(text: &str) -> GenerateContent {
    GenerateContent::Text {
        text: text.into(),
        provider_metadata: None,
    }
}

fn call(tool_name: &str) -> GenerateContent {
    GenerateContent::ToolCall(RawToolCall {
        tool_call_id: "call-1".into(),
        tool_name: tool_name.into(),
        input: r#"{"city":"Paris"}"#.into(),
        provider_executed: None,
        dynamic: None,
        provider_metadata: None,
    })
}

fn options(tool_choice: ToolChoice) -> GenerateTextOptions {
    let tool = |name: &str| -> Tool { FunctionTool::new(name, json!({ "type": "object" })).into() };
    GenerateTextOptions {
        tools: Some(vec![tool("weather"), tool("search")]),
        tool_choice: Some(tool_choice),
        ..Default::default()
    }
}

fn violation(error: &AiMuxError) -> &ToolChoiceViolationError {
    match error {
        AiMuxError::ToolChoiceViolation(violation) => violation,
        other => panic!("not a tool choice violation: {other:?}"),
    }
}

fn weather() -> ToolChoice {
    ToolChoice::Tool {
        tool_name: "weather".into(),
    }
}

// `standardize-prompt.ts:78-83`.
#[tokio::test]
async fn empty_messages_are_an_invalid_prompt() {
    let model = ScriptedModel {
        content: vec![text("hi")],
    };
    let error = generate_text(
        &model,
        ModelPrompt::Messages(vec![]),
        GenerateTextOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidPrompt(message) if message == "messages must not be empty"),
        "{error:?}"
    );
    let error = stream_text(
        &model,
        ModelPrompt::Messages(vec![]),
        GenerateTextOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AiMuxError::InvalidPrompt(_)), "{error:?}");
}

// `generate-text.ts:1155-1176`: a `required` choice needs some tool call, a
// `tool` choice a call to that tool; the error carries the choice, the
// unified finish reason, the model and the provider content.
#[tokio::test]
async fn generate_text_rejects_a_response_that_breaks_the_tool_choice() {
    let prompt = || ModelPrompt::Messages(vec![ModelMessage::user("weather?")]);

    let model = ScriptedModel {
        content: vec![text("It is sunny.")],
    };
    let error = generate_text(&model, prompt(), options(ToolChoice::Required))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model response did not contain a tool call even though tool choice was required."
    );
    let details = violation(&error);
    assert_eq!(details.tool_choice, LanguageModelV4ToolChoice::Required);
    assert_eq!(details.finish_reason, FinishReasonUnified::Stop);
    assert_eq!(details.provider, "scripted.chat");
    assert_eq!(details.model_id, "scripted-1");
    assert_eq!(details.content, vec![text("It is sunny.")]);

    let model = ScriptedModel {
        content: vec![call("search")],
    };
    let error = generate_text(&model, prompt(), options(weather()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model response did not contain a call to the required tool 'weather'."
    );
    assert_eq!(violation(&error).content, vec![call("search")]);

    // A call that satisfies the choice, or a choice that enforces nothing.
    let model = ScriptedModel {
        content: vec![call("weather")],
    };
    generate_text(&model, prompt(), options(weather()))
        .await
        .unwrap();
    generate_text(&model, prompt(), options(ToolChoice::Required))
        .await
        .unwrap();
    let model = ScriptedModel {
        content: vec![text("no tools")],
    };
    generate_text(&model, prompt(), options(ToolChoice::Auto))
        .await
        .unwrap();
}

// `stream-language-model-call.ts:644-687` and `stream-text.ts:2984-2989`:
// the stream ends with the violation as an error part and an `error` finish
// reason, the raw reason kept; the error carries the streamed provider
// content (text blocks joined by id, `:892-953`).
#[tokio::test]
async fn stream_text_ends_with_the_violation() {
    let model = ScriptedModel {
        content: vec![
            GenerateContent::Reasoning(ReasoningOutput {
                text: "think".into(),
                provider_metadata: None,
            }),
            text("It is sunny."),
        ],
    };
    let result = stream_text(
        &model,
        ModelPrompt::Messages(vec![ModelMessage::user("weather?")]),
        options(weather()),
    )
    .await
    .unwrap();
    let parts: Vec<_> = result.stream.collect().await;
    let parts: Vec<_> = parts.into_iter().map(Result::unwrap).collect();
    let [
        ..,
        TextStreamPart::Error { error },
        TextStreamPart::FinishStep {
            finish_reason: step_finish_reason,
            raw_finish_reason: step_raw_finish_reason,
            ..
        },
        TextStreamPart::Finish {
            finish_reason,
            raw_finish_reason,
            ..
        },
    ] = &parts[..]
    else {
        panic!("{parts:#?}")
    };
    assert_eq!(*step_finish_reason, FinishReasonUnified::Error);
    assert_eq!(step_raw_finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(*finish_reason, FinishReasonUnified::Error);
    assert_eq!(raw_finish_reason.as_deref(), Some("end_turn"));
    let details = violation(error);
    assert_eq!(
        details.tool_choice,
        LanguageModelV4ToolChoice::Tool {
            tool_name: "weather".into()
        }
    );
    assert_eq!(details.finish_reason, FinishReasonUnified::Stop);
    assert_eq!(
        details.content,
        vec![
            GenerateContent::Reasoning(ReasoningOutput {
                text: "think".into(),
                provider_metadata: None,
            }),
            text("It is sunny."),
        ]
    );

    // A satisfied choice finishes normally.
    let model = ScriptedModel {
        content: vec![call("weather")],
    };
    let result = stream_text(
        &model,
        ModelPrompt::Messages(vec![ModelMessage::user("weather?")]),
        options(weather()),
    )
    .await
    .unwrap();
    let parts: Vec<_> = result.stream.collect().await;
    assert!(
        parts
            .iter()
            .all(|part| !matches!(part, Ok(TextStreamPart::Error { .. }))),
        "{parts:#?}"
    );
    let Some(Ok(TextStreamPart::Finish { finish_reason, .. })) = parts.last() else {
        panic!("{parts:#?}")
    };
    assert_eq!(*finish_reason, FinishReasonUnified::Stop);
}
