//! The core `ContentPart` a text result carries, against the AI SDK
//! (`ai@7.0.127`): `generate-text/content-part.ts:12-33`, built by
//! `convert-language-model-content.ts`, replayed by
//! `to-response-messages.ts`.

use aimux_core::content::{AssistantContentPart, ToolContentPart, ToolResultOutput};
use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, generate_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::message::{ModelMessage, ModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::result::{
    ContentPart, GenerateContent, GenerateResult, LanguageModelFile, StreamResult, TypedToolError,
    TypedToolResult,
};
use aimux_core::shared::{FileBytes, GeneratedFileData};
use aimux_core::tool::{FunctionTool, ProviderTool, RawToolCall, Tool, ToolResult};
use aimux_core::types::{FinishReason, FinishReasonUnified, NamespacedKind, Usage};
use async_trait::async_trait;
use serde_json::{Value, json};

struct ScriptedModel {
    content: Vec<GenerateContent>,
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
            finish_reason: FinishReason {
                unified: FinishReasonUnified::ToolCalls,
                raw: None,
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: None,
            request: None,
        })
    }
    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        unimplemented!("generate only")
    }
}

fn call(id: &str, name: &str, input: &str, provider_executed: bool) -> GenerateContent {
    GenerateContent::ToolCall(RawToolCall {
        tool_call_id: id.into(),
        tool_name: name.into(),
        input: input.into(),
        provider_executed: provider_executed.then_some(true),
        dynamic: provider_executed.then_some(true),
        provider_metadata: None,
    })
}

fn provider_result(id: &str, result: Value, is_error: bool) -> GenerateContent {
    GenerateContent::ToolResult(ToolResult {
        tool_call_id: id.into(),
        tool_name: "web_search".into(),
        result,
        is_error: is_error.then_some(true),
        preliminary: None,
        dynamic: None,
        provider_metadata: None,
    })
}

async fn generate(content: Vec<GenerateContent>) -> aimux_core::generate::GenerateTextResult {
    generate_text(
        &ScriptedModel { content },
        ModelPrompt::Messages(vec![ModelMessage::user("go")]),
        GenerateTextOptions {
            tools: Some(vec![
                FunctionTool::new("weather", json!({ "type": "object" })).into(),
                // The provider tool whose late result `c0` answers
                // (`convert-language-model-content.ts:87-104`).
                Tool::Provider(ProviderTool {
                    id: NamespacedKind::new("openai.web_search").unwrap(),
                    name: "web_search".into(),
                    args: serde_json::Map::new(),
                }),
            ]),
            ..Default::default()
        },
    )
    .await
    .expect("generates")
}

fn tags(content: &[ContentPart]) -> Vec<String> {
    content
        .iter()
        .map(|part| match serde_json::to_value(part).unwrap() {
            Value::Object(map) => map["type"].as_str().unwrap().to_owned(),
            other => panic!("{other}"),
        })
        .collect()
}

// `convert-language-model-content.ts:55-73`: a generated file is wrapped as
// `{ file, providerMetadata }`, like a reasoning file, and the file is the
// AI SDK's `GeneratedFile` (`generated-file.ts:10-32`).
#[tokio::test]
async fn a_generated_file_is_wrapped() {
    let file = LanguageModelFile {
        data: GeneratedFileData::Data {
            data: FileBytes::Base64("aGk=".into()),
        },
        media_type: "text/plain".into(),
        provider_metadata: None,
    };
    let result = generate(vec![
        GenerateContent::File(file.clone()),
        GenerateContent::ReasoningFile(file),
    ])
    .await;
    assert_eq!(
        tags(&result.final_step().content),
        ["file", "reasoning-file"]
    );
    let wire = serde_json::to_value(&result.final_step().content).unwrap();
    assert_eq!(
        wire[0],
        json!({ "type": "file", "file": { "base64": "aGk=", "mediaType": "text/plain" } })
    );
    assert_eq!(wire[0]["file"], wire[1]["file"]);
}

// `convert-language-model-content.ts:87-176`: a provider tool result is a
// tool result, or a tool error when marked as one, with the input of its
// call; a result whose call is not in the response has no input.
// `generate-text.ts:1303-1323` with `:208-214`: a client tool call that
// could not be parsed gets a tool error after the model's content.
#[tokio::test]
async fn tool_results_errors_and_invalid_calls_become_their_own_parts() {
    let result = generate(vec![
        call("c1", "web_search", r#"{"q":"rust"}"#, true),
        provider_result("c1", json!({ "hits": 3 }), false),
        call("c2", "web_search", r#"{"q":"go"}"#, true),
        provider_result("c2", json!("quota exceeded"), true),
        provider_result("c0", json!("late"), false),
        call("c3", "unknown_tool", r#"{"x":1}"#, false),
        GenerateContent::Text {
            text: "done".into(),
            provider_metadata: None,
        },
    ])
    .await;
    assert_eq!(
        tags(&result.final_step().content),
        [
            "tool-call",
            "tool-result",
            "tool-call",
            "tool-error",
            "tool-result",
            "tool-call",
            "text",
            "tool-error"
        ]
    );
    let ContentPart::ToolResult(found) = &result.final_step().content[1] else {
        panic!()
    };
    assert_eq!(
        found,
        &TypedToolResult {
            tool_call_id: "c1".into(),
            tool_name: "web_search".into(),
            input: Some(json!({ "q": "rust" })),
            output: json!({ "hits": 3 }),
            provider_executed: Some(true),
            provider_metadata: None,
            // The call is to a declared provider tool, so it is not dynamic
            // (`parse-tool-call.ts:267`).
            dynamic: None,
            preliminary: None,
        }
    );
    let ContentPart::ToolError(error) = &result.final_step().content[3] else {
        panic!()
    };
    assert_eq!(error.error, json!("quota exceeded"));
    assert_eq!(error.input, Some(json!({ "q": "go" })));
    let ContentPart::ToolResult(deferred) = &result.final_step().content[4] else {
        panic!()
    };
    assert_eq!(deferred.input, None);
    assert!(
        serde_json::to_value(deferred)
            .unwrap()
            .get("input")
            .is_none(),
        "a deferred result has no input key"
    );
    let ContentPart::ToolError(invalid) = &result.final_step().content[7] else {
        panic!()
    };
    assert_eq!(
        invalid,
        &TypedToolError {
            tool_call_id: "c3".into(),
            tool_name: "unknown_tool".into(),
            input: Some(json!({ "x": 1 })),
            error: json!(
                "Model tried to call unavailable tool 'unknown_tool'. Available tools: weather, web_search."
            ),
            provider_executed: None,
            provider_metadata: None,
            dynamic: Some(true),
        }
    );
}

// `to-response-messages.ts:17-228`: provider-executed results replay in the
// assistant message (an error as an error-json output), the other tool
// errors in a tool message as error text, ordered by tool call.
#[tokio::test]
async fn tool_errors_replay_where_upstream_puts_them() {
    let result = generate(vec![
        call("c3", "unknown_tool", "{", false),
        call("c2", "web_search", r#"{"q":"go"}"#, true),
        provider_result("c2", json!("quota exceeded"), true),
        call("c1", "other_tool", "{}", false),
    ])
    .await;
    let [
        ModelMessage::Assistant { content, .. },
        ModelMessage::Tool {
            content: tool_content,
            ..
        },
    ] = &result.final_step().response.messages[..]
    else {
        panic!("{:#?}", result.final_step().response.messages)
    };
    let aimux_core::message::AssistantContent::Parts(parts) = content else {
        panic!()
    };
    let AssistantContentPart::ToolResult(provider) = &parts[2] else {
        panic!("{parts:#?}")
    };
    assert_eq!(
        provider.output,
        ToolResultOutput::ErrorJson {
            value: json!("quota exceeded"),
            provider_options: None
        }
    );
    // The invalid input text is replayed as `{}` (`:93-94`).
    let AssistantContentPart::ToolCall { input, .. } = &parts[0] else {
        panic!()
    };
    assert_eq!(input, &json!({}));
    let ids: Vec<_> = tool_content
        .iter()
        .map(|part| match part {
            ToolContentPart::ToolResult(result) => {
                let ToolResultOutput::ErrorText { value, .. } = &result.output else {
                    panic!("{result:#?}")
                };
                assert!(value.starts_with("Model tried to call unavailable tool"));
                result.tool_call_id.as_str()
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ids, ["c3", "c1"]);
}
