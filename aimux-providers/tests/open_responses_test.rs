//! Open Responses provider tests, translated from the Vercel AI SDK TypeScript suite.
//!
//! Translation sources:
//! - `packages/open-responses/src/responses/map-open-responses-finish-reason.test.ts` (8 tests)
//! - `packages/open-responses/src/responses/convert-to-open-responses-input.test.ts` (31 tests)
//! - `packages/open-responses/src/responses/open-responses-language-model.test.ts` (33 tests)
//!
//! HTTP is mocked with `wiremock` (a real loopback server), replacing the TS
//! `createTestServer`. Each test starts its own `MockServer` so parallel
//! `#[tokio::test]` runs do not collide.

use aimux_core::tool::RawToolCall;

use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, Tool, ToolChoice};
use aimux_core::result::{GenerateContent, ReasoningOutput};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::FunctionTool;
use aimux_core::types::{FinishReasonUnified, ReasoningEffort};

use aimux_providers::open_responses::{
    OpenResponsesModel, OpenResponsesProviderSettings, convert_to_open_responses_input,
    create_open_responses, map_open_responses_finish_reason,
};

// ============================================================================
// Shared helpers
// ============================================================================

/// The TS `TEST_PROMPT`: a single user text message "Hello".
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `CallOptions` with only `prompt` set (everything else default/None).
fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build Open Responses settings whose base URL points at the mock server.
fn make_settings(server: &MockServer) -> OpenResponsesProviderSettings {
    OpenResponsesProviderSettings::new("lmstudio", server.uri())
}

/// Create a model pointing at the mock server.
fn make_model(server: &MockServer, model_id: &str) -> OpenResponsesModel {
    let provider = create_open_responses(make_settings(server)).expect("valid settings");
    provider.responses(model_id)
}

/// Mount a JSON response on any path.
async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on any path.
async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Build a single SSE `data: <json>\n\n` event string.
fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(_) => break,
        }
    }
    parts
}

// ============================================================================
// Fixture data
// ============================================================================

/// The `lmstudio-basic.1.json` fixture (simplified to the essential fields).
fn lmstudio_basic_json() -> Value {
    json!({
        "id": "resp_551daeb1a02e4fcaf9ab76ed29f821a6db2df1883e55652c",
        "object": "response",
        "created_at": 1768900049,
        "completed_at": 1768900162,
        "status": "completed",
        "incomplete_details": null,
        "model": "mistralai/ministral-3-14b-reasoning",
        "output": [
            {
                "id": "rs_3l1z5wpifxkwxhj459ya7",
                "type": "reasoning",
                "status": "completed",
                "summary": [],
                "content": [
                    {
                        "type": "reasoning_text",
                        "text": "reasoning content"
                    }
                ]
            },
            {
                "id": "msg_p1y190hl7hj1xyfqr1cir",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "text": "text content",
                        "annotations": [],
                        "logprobs": []
                    }
                ]
            }
        ],
        "error": null,
        "usage": {
            "input_tokens": 136,
            "output_tokens": 3677,
            "total_tokens": 3813,
            "input_tokens_details": {
                "cached_tokens": 0
            },
            "output_tokens_details": {
                "reasoning_tokens": 2456
            }
        }
    })
}

/// The `lmstudio-tool-call.1.json` fixture.
fn lmstudio_tool_call_json() -> Value {
    json!({
        "id": "resp_930de53bd4b5933673481fa630f3dc5f58027a2c67598a2a",
        "object": "response",
        "created_at": 1769005553,
        "status": "completed",
        "incomplete_details": null,
        "model": "mistralai/ministral-3-14b-reasoning",
        "output": [
            {
                "id": "fc_ru0kcno9erlzp8573yub",
                "call_id": "call_2866856768160095",
                "type": "function_call",
                "name": "weather",
                "arguments": "{\"location\":\"San Francisco\"}",
                "status": "completed"
            }
        ],
        "error": null,
        "usage": {
            "input_tokens": 1189,
            "output_tokens": 11,
            "total_tokens": 1200,
            "input_tokens_details": {
                "cached_tokens": 891
            },
            "output_tokens_details": {
                "reasoning_tokens": 0
            }
        }
    })
}

/// The `openai-pdf-input-file.1.json` fixture.
fn openai_pdf_json() -> Value {
    json!({
        "id": "resp_048edf44633e41ae0069d4fe9fabf48194957da2d8582b1c4a",
        "object": "response",
        "created_at": 1775566496,
        "status": "completed",
        "incomplete_details": null,
        "model": lmstudio_basic_json()["model"],
        "output": [
            {
                "id": "msg_048edf44633e41ae0069d4fea0d1a08194af1e491c093df1d9",
                "type": "message",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "annotations": [],
                        "logprobs": [],
                        "text": "Dummy PDF file"
                    }
                ],
                "role": "assistant"
            }
        ],
        "error": null,
        "usage": {
            "input_tokens": 44,
            "input_tokens_details": {
                "cached_tokens": 0
            },
            "output_tokens": 4,
            "output_tokens_details": {
                "reasoning_tokens": 0
            },
            "total_tokens": 48
        }
    })
}

// ============================================================================
// mapOpenResponsesFinishReason tests (8 tests)
// ============================================================================

#[cfg(test)]
mod finish_reason_tests {
    use super::*;

    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_undefined() {
        assert_eq!(
            map_open_responses_finish_reason(None, true),
            FinishReasonUnified::ToolCalls
        );
    }

    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_null() {
        // In Rust, null and undefined are both represented as None.
        assert_eq!(
            map_open_responses_finish_reason(None, true),
            FinishReasonUnified::ToolCalls
        );
    }

    #[test]
    fn stop_when_no_tool_calls_and_finish_reason_undefined() {
        assert_eq!(
            map_open_responses_finish_reason(None, false),
            FinishReasonUnified::Stop
        );
    }

    #[test]
    fn stop_when_no_tool_calls_and_finish_reason_null() {
        assert_eq!(
            map_open_responses_finish_reason(None, false),
            FinishReasonUnified::Stop
        );
    }

    #[test]
    fn length_when_finish_reason_max_output_tokens() {
        assert_eq!(
            map_open_responses_finish_reason(Some("max_output_tokens"), false),
            FinishReasonUnified::Length
        );
    }

    #[test]
    fn content_filter_when_finish_reason_content_filter() {
        assert_eq!(
            map_open_responses_finish_reason(Some("content_filter"), false),
            FinishReasonUnified::ContentFilter
        );
    }

    #[test]
    fn tool_calls_when_has_tool_calls_and_finish_reason_unknown() {
        assert_eq!(
            map_open_responses_finish_reason(Some("completed"), true),
            FinishReasonUnified::ToolCalls
        );
    }

    #[test]
    fn other_when_no_tool_calls_and_finish_reason_unknown() {
        assert_eq!(
            map_open_responses_finish_reason(Some("completed"), false),
            FinishReasonUnified::Other
        );
    }
}

// ============================================================================
// convertToOpenResponsesInput tests
// ============================================================================

#[cfg(test)]
mod convert_tests {
    use super::*;

    // -- System messages --

    #[test]
    fn convert_single_system_message_to_instructions() {
        let prompt = vec![LanguageModelMessage::System {
            content: "You are a helpful assistant.".into(),
            provider_options: None,
        }];
        let (input, instructions, warnings) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            instructions,
            Some("You are a helpful assistant.".to_string())
        );
        assert_eq!(input, json!([]));
        assert!(warnings.is_empty());
    }

    #[test]
    fn convert_multiple_system_messages_joined_with_newlines() {
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::System {
                content: "Always be concise.".into(),
                provider_options: None,
            },
        ];
        let (_, instructions, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            instructions,
            Some("You are a helpful assistant.\nAlways be concise.".to_string())
        );
    }

    #[test]
    fn convert_no_system_messages_returns_none_instructions() {
        let prompt = test_prompt();
        let (_, instructions, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(instructions, None);
    }

    #[test]
    fn convert_system_message_with_user_and_assistant() {
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: "Hi there!".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        let (input, instructions, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            instructions,
            Some("You are a helpful assistant.".to_string())
        );
        assert_eq!(
            input,
            json!([
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Hi there!"}]
                }
            ])
        );
    }

    // -- User messages --

    #[test]
    fn convert_user_text_part() {
        let prompt = test_prompt();
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Hello"}]
            }])
        );
    }

    #[test]
    fn convert_image_file_base64_to_input_image() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("ZmFrZS1kYXRh".into()),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_image", "image_url": "data:image/png;base64,ZmFrZS1kYXRh"}]
            }])
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn convert_image_file_url_to_input_image() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Url {
                    original_url: None,
                    url: "https://example.com/image.png".into(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_image", "image_url": "https://example.com/image.png"}]
            }])
        );
    }

    #[test]
    fn convert_pdf_file_base64_to_input_file() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: "What does this PDF say?".into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("UERGREFUQQ==".into()),
                    },
                    media_type: "application/pdf".into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What does this PDF say?"},
                    {"type": "input_file", "filename": "data", "file_data": "data:application/pdf;base64,UERGREFUQQ=="}
                ]
            }])
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn convert_pdf_file_url_to_input_file() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Url {
                    original_url: None,
                    url: "https://example.com/document.pdf".into(),
                },
                media_type: "application/pdf".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_file", "file_url": "https://example.com/document.pdf"}]
            }])
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn convert_file_with_custom_filename() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("UERGREFUQQ==".to_string()),
                },
                media_type: "application/pdf".to_string(),
                filename: Some("report.pdf".to_string()),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_file",
                    "filename": "report.pdf",
                    "file_data": "data:application/pdf;base64,UERGREFUQQ=="
                }]
            }])
        );
    }

    // -- Assistant messages --

    #[test]
    fn convert_assistant_text_part() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: "Hello from assistant".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello from assistant"}]
            }])
        );
    }

    #[test]
    fn convert_assistant_multiple_text_parts() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::Text(TextPart {
                    text: "First part".into(),
                    provider_options: None,
                }),
                AssistantPart::Text(TextPart {
                    text: "Second part".into(),
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "First part"},
                    {"type": "output_text", "text": "Second part"}
                ]
            }])
        );
    }

    // -- Assistant messages with tool calls --

    #[test]
    fn convert_assistant_single_tool_call() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::ToolCall(ToolCallPart {
                tool_call_id: "call_123".into(),
                tool_name: "get_weather".into(),
                input: json!({"location": "San Francisco"}),
                provider_executed: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call",
                "call_id": "call_123",
                "name": "get_weather",
                "arguments": "{\"location\":\"San Francisco\"}"
            }])
        );
    }

    #[test]
    fn convert_assistant_tool_call_string_input() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::ToolCall(ToolCallPart {
                tool_call_id: "call_124".into(),
                tool_name: "get_weather".into(),
                input: Value::String("{\"location\":\"Berlin\"}".to_string()),
                provider_executed: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call",
                "call_id": "call_124",
                "name": "get_weather",
                "arguments": "{\"location\":\"Berlin\"}"
            }])
        );
    }

    #[test]
    fn convert_assistant_text_and_tool_call() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::Text(TextPart {
                    text: "Let me check the weather for you.".into(),
                    provider_options: None,
                }),
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_456".into(),
                    tool_name: "get_weather".into(),
                    input: json!({"location": "New York"}),
                    provider_executed: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Let me check the weather for you."}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_456",
                    "name": "get_weather",
                    "arguments": "{\"location\":\"New York\"}"
                }
            ])
        );
    }

    #[test]
    fn convert_assistant_multiple_tool_calls() {
        let prompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_001".into(),
                    tool_name: "get_weather".into(),
                    input: json!({"location": "Paris"}),
                    provider_executed: None,
                    provider_options: None,
                }),
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_002".into(),
                    tool_name: "get_time".into(),
                    input: json!({"timezone": "Europe/Paris"}),
                    provider_executed: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {
                    "type": "function_call",
                    "call_id": "call_001",
                    "name": "get_weather",
                    "arguments": "{\"location\":\"Paris\"}"
                },
                {
                    "type": "function_call",
                    "call_id": "call_002",
                    "name": "get_time",
                    "arguments": "{\"timezone\":\"Europe/Paris\"}"
                }
            ])
        );
    }

    // -- Tool messages --

    #[test]
    fn convert_tool_message_json_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_123".into(),
                output: serde_json::from_value(
                    json!({"type": "json", "value": {"temperature": 72, "condition": "sunny"}}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_123",
                "output": "{\"temperature\":72,\"condition\":\"sunny\"}"
            }])
        );
    }

    #[test]
    fn convert_tool_message_text_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_456".into(),
                output: serde_json::from_value(
                    json!({"type": "text", "value": "Search results: Found 5 items"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_456",
                "output": "Search results: Found 5 items"
            }])
        );
    }

    #[test]
    fn convert_tool_message_error_text_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_789".into(),
                output: serde_json::from_value(
                    json!({"type": "error_text", "value": "API request failed: timeout"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_789",
                "output": "API request failed: timeout"
            }])
        );
    }

    #[test]
    fn convert_tool_message_execution_denied_output() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_denied".into(),
                output: serde_json::from_value(
                    json!({"type": "execution_denied", "reason": "User declined the action"}),
                )
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_denied",
                "output": "User declined the action"
            }])
        );
    }

    #[test]
    fn convert_tool_message_content_output_text() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![ToolPart::ToolResult(ToolResultPart {
                tool_call_id: "call_content".into(),
                output: serde_json::from_value(json!({
                    "type": "content",
                    "value": [
                        {"type": "text", "text": "First result"},
                        {"type": "text", "text": "Second result"}
                    ]
                }))
                .unwrap(),
                tool_name: "get_weather".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([{
                "type": "function_call_output",
                "call_id": "call_content",
                "output": [
                    {"type": "input_text", "text": "First result"},
                    {"type": "input_text", "text": "Second result"}
                ]
            }])
        );
    }

    #[test]
    fn convert_tool_message_multiple_results() {
        let prompt = vec![LanguageModelMessage::Tool {
            content: vec![
                ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_001".into(),
                    output: serde_json::from_value(json!({"type": "json", "value": {"temp": 72}}))
                        .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                }),
                ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_002".into(),
                    output: serde_json::from_value(json!({"type": "text", "value": "3:00 PM"}))
                        .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {
                    "type": "function_call_output",
                    "call_id": "call_001",
                    "output": "{\"temp\":72}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_002",
                    "output": "3:00 PM"
                }
            ])
        );
    }

    // -- Message chains --

    #[test]
    fn convert_user_assistant_user_chain() {
        let prompt = vec![
            LanguageModelMessage::user_text("What is the capital of France?"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: "The capital of France is Paris.".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::user_text("And what about Germany?"),
        ];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the capital of France?"}]},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "The capital of France is Paris."}]},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "And what about Germany?"}]}
            ])
        );
    }

    #[test]
    fn convert_user_assistant_tool_tool_chain() {
        let prompt = vec![
            LanguageModelMessage::user_text("What is the weather in Tokyo?"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_weather".into(),
                    tool_name: "get_weather".into(),
                    input: json!({"location": "Tokyo"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_weather".into(),
                    output: serde_json::from_value(json!({"type": "json", "value": {"temperature": 25, "condition": "cloudy"}})).unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                {"type": "function_call", "call_id": "call_weather", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                {"type": "function_call_output", "call_id": "call_weather", "output": "{\"temperature\":25,\"condition\":\"cloudy\"}"}
            ])
        );
    }

    #[test]
    fn convert_tool_roundtrip_with_followup_assistant() {
        let prompt = vec![
            LanguageModelMessage::user_text("What is the weather in Tokyo?"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_weather".into(),
                    tool_name: "get_weather".into(),
                    input: Value::String("{\"location\":\"Tokyo\"}".to_string()),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_weather".into(),
                    output: serde_json::from_value(json!({"type": "json", "value": {"temperature": 25, "condition": "cloudy"}})).unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::Text(TextPart {
                    text: "It is 25 C and cloudy in Tokyo.".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        let (input, _, _) = convert_to_open_responses_input(&prompt);
        assert_eq!(
            input,
            json!([
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                {"type": "function_call", "call_id": "call_weather", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                {"type": "function_call_output", "call_id": "call_weather", "output": "{\"temperature\":25,\"condition\":\"cloudy\"}"},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "It is 25 C and cloudy in Tokyo."}]}
            ])
        );
    }

    // -- Provider reference (warning, not error in Rust) --

    #[test]
    fn convert_file_reference_produces_warning() {
        let prompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Reference {
                    reference: [("openResponses".into(), "file-ref-123".into())]
                        .into_iter()
                        .collect(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        let (input, _, warnings) = convert_to_open_responses_input(&prompt);
        // The reference part is skipped, so user content is empty array.
        assert_eq!(
            input,
            json!([{
                "type": "message",
                "role": "user",
                "content": []
            }])
        );
        assert!(!warnings.is_empty());
    }
}

// ============================================================================
// OpenResponsesModel doGenerate tests
// ============================================================================

#[cfg(test)]
mod do_generate_tests {
    use super::*;

    #[tokio::test]
    async fn throws_when_response_has_no_output() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "resp_no_output",
                "created_at": 1741257730,
                "model": "gemma-7b-it",
                "status": "incomplete",
                "incomplete_details": {"reason": "content_filter"},
                "usage": {"input_tokens": 10, "output_tokens": 0}
            }),
        )
        .await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model.do_generate(&default_options(test_prompt())).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Responses API returned no output (content_filter)"),
            "got: {msg}"
        );
    }

    #[tokio::test]
    async fn surfaces_response_error_before_no_output() {
        let server = MockServer::start().await;
        mock_json(
            &server,
            json!({
                "id": "resp_error",
                "created_at": 1741257730,
                "model": "gemma-7b-it",
                "status": "failed",
                "error": {
                    "code": "server_error",
                    "message": "The upstream provider failed to generate a response."
                },
                "usage": {"input_tokens": 10, "output_tokens": 0}
            }),
        )
        .await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model.do_generate(&default_options(test_prompt())).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("The upstream provider failed to generate a response."),
            "got: {msg}"
        );
    }

    #[tokio::test]
    async fn basic_generation_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gemma-7b-it",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                }]
            })
        );
    }

    #[tokio::test]
    async fn basic_generation_content() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 2);
        assert_eq!(
            result.content[0],
            GenerateContent::Reasoning(ReasoningOutput {
                text: "reasoning content".to_string(),
                provider_metadata: None,
            })
        );
        assert_eq!(
            result.content[1],
            GenerateContent::Text {
                text: "text content".to_string(),
                provider_metadata: None,
            }
        );
    }

    #[tokio::test]
    async fn basic_generation_usage() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(136));
        assert_eq!(result.usage.input_tokens.no_cache, Some(136));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(3677));
        assert_eq!(result.usage.output_tokens.reasoning, Some(2456));
        assert_eq!(result.usage.output_tokens.text, Some(1221));
    }

    #[tokio::test]
    async fn request_parameters_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            max_output_tokens: Some(100),
            temperature: Some(0.5),
            top_p: Some(0.9),
            presence_penalty: Some(0.1),
            frequency_penalty: Some(0.2),
            response_format: Some(ResponseFormat::Json {
                schema: Some(json!({
                    "type": "object",
                    "properties": {"status": {"type": "string"}},
                    "required": ["status"]
                })),
                name: Some("response".to_string()),
                description: Some("Example response schema".to_string()),
            }),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gemma-7b-it",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                }],
                "max_output_tokens": 100,
                "temperature": 0.5,
                "top_p": 0.9,
                "presence_penalty": 0.1,
                "frequency_penalty": 0.2,
                "text": {
                    "format": {
                        "type": "json_schema",
                        "strict": true,
                        "name": "response",
                        "description": "Example response schema",
                        "schema": {
                            "type": "object",
                            "properties": {"status": {"type": "string"}},
                            "required": ["status"]
                        }
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn tools_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![
                Tool::from(
                    FunctionTool::new(
                        "get_weather",
                        json!({
                            "type": "object",
                            "properties": {
                                "location": {"type": "string", "description": "The city and state"}
                            },
                            "required": ["location"]
                        }),
                    )
                    .with_description("Get the current weather for a location"),
                ),
                Tool::from(
                    FunctionTool::new(
                        "search",
                        json!({
                            "type": "object",
                            "properties": {"query": {"type": "string"}},
                            "required": ["query"]
                        }),
                    )
                    .with_description("Search for information")
                    .with_strict(true),
                ),
            ]),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gemma-7b-it",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Hello"}]
                }],
                "tools": [
                    {
                        "type": "function",
                        "name": "get_weather",
                        "parameters": {
                            "type": "object",
                            "properties": {"location": {"type": "string", "description": "The city and state"}},
                            "required": ["location"]
                        },
                        "description": "Get the current weather for a location"
                    },
                    {
                        "type": "function",
                        "name": "search",
                        "parameters": {
                            "type": "object",
                            "properties": {"query": {"type": "string"}},
                            "required": ["query"]
                        },
                        "description": "Search for information",
                        "strict": true
                    }
                ]
            })
        );
    }

    // -- Top-level reasoning tests --

    #[tokio::test]
    async fn reasoning_high_maps_to_effort() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::High),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"effort": "high"}));
    }

    #[tokio::test]
    async fn reasoning_minimal_coerced_to_low() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::Minimal),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"effort": "low"}));
    }

    #[tokio::test]
    async fn reasoning_none_maps_to_none() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"effort": "none"}));
    }

    #[tokio::test]
    async fn reasoning_xhigh_passed_directly() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::Xhigh),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"effort": "xhigh"}));
    }

    #[tokio::test]
    async fn reasoning_not_set_when_not_specified() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(body.get("reasoning").is_none());
    }

    // -- ProviderOptions reasoning tests --

    fn lmstudio_opts(value: Value) -> Option<aimux_core::shared::SharedProviderOptions> {
        Some(aimux_core::shared::provider_namespace("lmstudio", value).unwrap())
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:594`
    #[tokio::test]
    async fn provider_options_reasoning_summary_detailed() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({"reasoningSummary": "detailed"})),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"summary": "detailed"}));
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:623`
    #[tokio::test]
    async fn provider_options_combines_effort_with_summary() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            reasoning: Some(ReasoningEffort::High),
            provider_options: lmstudio_opts(json!({"reasoningSummary": "auto"})),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["reasoning"],
            json!({"effort": "high", "summary": "auto"})
        );
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:655`
    #[tokio::test]
    async fn provider_options_reasoning_summary_concise() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({"reasoningSummary": "concise"})),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reasoning"], json!({"summary": "concise"}));
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:685`
    #[tokio::test]
    async fn provider_options_no_reasoning_fields() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            provider_options: lmstudio_opts(json!({})),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(body.get("reasoning").is_none());
    }

    // -- Tool call parsing tests --

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:742`
    #[tokio::test]
    async fn parse_tool_call_from_response() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new(
                    "weather",
                    json!({
                        "type": "object",
                        "properties": {
                            "location": {"type": "string", "description": "The location to get the weather for"}
                        },
                        "required": ["location"]
                    }),
                )
                .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(ToolChoice::Required),
            ..CallOptions::new(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            GenerateContent::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => {
                assert_eq!(tool_call_id, "call_2866856768160095");
                assert_eq!(tool_name, "weather");
                assert_eq!(
                    input,
                    &Value::String(r#"{"location":"San Francisco"}"#.into())
                );
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:746`
    #[tokio::test]
    async fn tool_call_finish_reason() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new("weather", json!({"type": "object"}))
                    .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(ToolChoice::Required),
            ..CallOptions::new(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(result.finish_reason.raw, None);
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:753`
    #[tokio::test]
    async fn tool_call_usage() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_tool_call_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new("weather", json!({"type": "object"}))
                    .with_description("Get the weather in a location"),
            )]),
            tool_choice: Some(ToolChoice::Required),
            ..CallOptions::new(test_prompt())
        };
        let result = model.do_generate(&options).await.unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(1189));
        assert_eq!(result.usage.input_tokens.cache_read, Some(891));
        assert_eq!(result.usage.input_tokens.no_cache, Some(298));
        assert_eq!(result.usage.output_tokens.total, Some(11));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(11));
    }

    // -- Tool choice tests --

    fn test_tool() -> Tool {
        Tool::from(
            FunctionTool::new(
                "get_weather",
                json!({
                    "type": "object",
                    "properties": {"location": {"type": "string"}},
                    "required": ["location"]
                }),
            )
            .with_description("Get the current weather"),
        )
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:772`
    #[tokio::test]
    async fn tool_choice_auto() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(ToolChoice::Auto),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("auto"));
        assert!(body.get("tools").is_some());
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:784`
    #[tokio::test]
    async fn tool_choice_none() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(ToolChoice::None),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("none"));
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:796`
    #[tokio::test]
    async fn tool_choice_required() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(ToolChoice::Required),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tool_choice"], json!("required"));
    }

    /// TS: `packages/open-responses/src/responses/open-responses-language-model.test.ts:808`
    #[tokio::test]
    async fn tool_choice_specific_tool() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let options = CallOptions {
            tools: Some(vec![test_tool()]),
            tool_choice: Some(ToolChoice::Tool {
                tool_name: "get_weather".to_string(),
            }),
            ..CallOptions::new(test_prompt())
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["tool_choice"],
            json!({"type": "function", "name": "get_weather"})
        );
    }

    // -- System messages tests --

    #[tokio::test]
    async fn system_message_instructions() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["instructions"], json!("You are a helpful assistant."));
    }

    #[tokio::test]
    async fn multiple_system_messages_joined() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let prompt = vec![
            LanguageModelMessage::System {
                content: "You are a helpful assistant.".into(),
                provider_options: None,
            },
            LanguageModelMessage::System {
                content: "Always be concise.".into(),
                provider_options: None,
            },
            LanguageModelMessage::user_text("Hello"),
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["instructions"],
            json!("You are a helpful assistant.\nAlways be concise.")
        );
    }

    // -- Multi-turn tool conversation --

    #[tokio::test]
    async fn multi_turn_tool_conversation_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, lmstudio_basic_json()).await;

        let model = make_model(&server, "gemma-7b-it");
        let prompt = vec![
            LanguageModelMessage::user_text("What is the weather in Tokyo?"),
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "call_weather_123".into(),
                    tool_name: "get_weather".into(),
                    input: json!({"location": "Tokyo"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "call_weather_123".into(),
                    output: serde_json::from_value(json!({
                        "type": "json",
                        "value": {"temperature": 22, "condition": "sunny", "humidity": 65}
                    }))
                    .unwrap(),
                    tool_name: "get_weather".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        let options = CallOptions {
            tools: Some(vec![Tool::from(
                FunctionTool::new(
                    "get_weather",
                    json!({
                        "type": "object",
                        "properties": {"location": {"type": "string"}},
                        "required": ["location"]
                    }),
                )
                .with_description("Get the current weather for a location"),
            )]),
            ..CallOptions::new(prompt)
        };
        model.do_generate(&options).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gemma-7b-it",
                "input": [
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "What is the weather in Tokyo?"}]},
                    {"type": "function_call", "call_id": "call_weather_123", "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"},
                    {"type": "function_call_output", "call_id": "call_weather_123", "output": "{\"temperature\":22,\"condition\":\"sunny\",\"humidity\":65}"}
                ],
                "tools": [{
                    "type": "function",
                    "name": "get_weather",
                    "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]},
                    "description": "Get the current weather for a location"
                }]
            })
        );
    }

    // -- PDF input file --

    #[tokio::test]
    async fn pdf_input_file_request_body() {
        let server = MockServer::start().await;
        mock_json(&server, openai_pdf_json()).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain? Reply with just the text content, nothing else.".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let request = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-4.1-nano",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "What text does this PDF contain? Reply with just the text content, nothing else."},
                        {"type": "input_file", "file_url": "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf"}
                    ]
                }]
            })
        );
    }

    #[tokio::test]
    async fn pdf_input_file_content() {
        let server = MockServer::start().await;
        mock_json(&server, openai_pdf_json()).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_generate(&default_options(prompt)).await.unwrap();

        assert_eq!(result.content.len(), 1);
        assert_eq!(
            result.content[0],
            GenerateContent::Text {
                text: "Dummy PDF file".to_string(),
                provider_metadata: None,
            }
        );
    }

    #[tokio::test]
    async fn pdf_input_file_usage() {
        let server = MockServer::start().await;
        mock_json(&server, openai_pdf_json()).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_generate(&default_options(prompt)).await.unwrap();

        assert_eq!(result.usage.input_tokens.total, Some(44));
        assert_eq!(result.usage.input_tokens.no_cache, Some(44));
        assert_eq!(result.usage.input_tokens.cache_read, Some(0));
        assert_eq!(result.usage.output_tokens.total, Some(4));
        assert_eq!(result.usage.output_tokens.reasoning, Some(0));
        assert_eq!(result.usage.output_tokens.text, Some(4));
    }
}

// ============================================================================
// OpenResponsesModel doStream tests
// ============================================================================

#[cfg(test)]
mod do_stream_tests {
    use super::*;

    /// Build SSE chunks for a basic text streaming response (simplified).
    fn basic_stream_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            sse_event(
                r#"{"type":"response.in_progress","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":1}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","content":[],"role":"assistant"},"sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"Hello","sequence_number":3}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":" world","sequence_number":4}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","content":[{"type":"output_text","text":"Hello world"}],"role":"assistant"},"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}},"sequence_number":6}"#,
            ),
        ]
    }

    #[tokio::test]
    async fn stream_basic_generation() {
        let server = MockServer::start().await;
        let chunks = basic_stream_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        // StreamStart
        assert!(matches!(
            &parts[0],
            StreamPart::StreamStart { warnings } if warnings.is_empty()
        ));

        // TextStart
        assert!(matches!(
            &parts[1],
            StreamPart::TextStart { id, .. } if id == "msg_1"
        ));

        // TextDelta "Hello"
        assert!(matches!(
            &parts[2],
            StreamPart::TextDelta { id, delta, .. } if id == "msg_1" && delta == "Hello"
        ));

        // TextDelta " world"
        assert!(matches!(
            &parts[3],
            StreamPart::TextDelta { id, delta, .. } if id == "msg_1" && delta == " world"
        ));

        // TextEnd
        assert!(matches!(
            &parts[4],
            StreamPart::TextEnd { id, .. } if id == "msg_1"
        ));

        // Finish
        assert!(
            matches!(&parts[5], StreamPart::Finish { finish_reason, usage, .. } if
                finish_reason.unified == FinishReasonUnified::Stop &&
                usage.input_tokens.total == Some(10) &&
                usage.output_tokens.total == Some(5)
            )
        );
    }

    /// Build SSE chunks for a reasoning + tool call streaming response.
    fn reasoning_tool_call_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            // Reasoning item added
            sse_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","status":"in_progress","summary":[],"content":[]},"sequence_number":1}"#,
            ),
            // Reasoning text deltas
            sse_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"content_index":0,"delta":"Thinking","sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"content_index":0,"delta":" about weather","sequence_number":3}"#,
            ),
            // Reasoning item done
            sse_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","status":"completed","summary":[],"content":[{"type":"reasoning_text","text":"Thinking about weather"}]},"sequence_number":4}"#,
            ),
            // Message item added
            sse_event(
                r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","status":"in_progress","content":[],"role":"assistant"},"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":0,"delta":"I'll check.","sequence_number":6}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","status":"completed","content":[{"type":"output_text","text":"I'll check."}],"role":"assistant"},"sequence_number":7}"#,
            ),
            // Function call
            sse_event(
                r#"{"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"in_progress","arguments":"","call_id":"call_1","name":"weather"},"sequence_number":8}"#,
            ),
            sse_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":2,"arguments":"{\"location\":\"SF\"}","sequence_number":9}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","status":"completed","arguments":"{\"location\":\"SF\"}","call_id":"call_1","name":"weather"},"sequence_number":10}"#,
            ),
            // Response completed
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150,"input_tokens_details":{"cached_tokens":20},"output_tokens_details":{"reasoning_tokens":10}}},"sequence_number":11}"#,
            ),
        ]
    }

    #[tokio::test]
    async fn stream_reasoning_with_tool_call() {
        let server = MockServer::start().await;
        let chunks = reasoning_tool_call_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();
        let parts = collect_stream(result).await;

        // Find the key parts.
        let has_reasoning_start = parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningStart { id, .. } if id == "rs_1"));
        assert!(has_reasoning_start, "should have ReasoningStart");

        let reasoning_deltas: Vec<&str> = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::ReasoningDelta { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reasoning_deltas, vec!["Thinking", " about weather"]);

        let has_reasoning_end = parts
            .iter()
            .any(|p| matches!(p, StreamPart::ReasoningEnd { id, .. } if id == "rs_1"));
        assert!(has_reasoning_end, "should have ReasoningEnd");

        // ToolCall
        let tool_call = parts.iter().find_map(|p| match p {
            StreamPart::ToolCall(RawToolCall {
                tool_call_id,
                tool_name,
                input,
                ..
            }) => Some((tool_call_id, tool_name, input)),
            _ => None,
        });
        assert!(tool_call.is_some(), "should have ToolCall");
        let (tc_id, tc_name, tc_input) = tool_call.unwrap();
        assert_eq!(tc_id, "call_1");
        assert_eq!(tc_name, "weather");
        assert_eq!(tc_input, &Value::String(r#"{"location":"SF"}"#.into()));

        // Finish with tool-calls reason
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some((finish_reason, usage)),
            _ => None,
        });
        assert!(finish.is_some(), "should have Finish");
        let (fr, usage) = finish.unwrap();
        assert_eq!(fr.unified, FinishReasonUnified::ToolCalls);
        assert_eq!(usage.input_tokens.total, Some(100));
        assert_eq!(usage.input_tokens.cache_read, Some(20));
        assert_eq!(usage.output_tokens.total, Some(50));
        assert_eq!(usage.output_tokens.reasoning, Some(10));
    }

    /// Build SSE chunks for a PDF input streaming response.
    fn pdf_stream_chunks() -> Vec<String> {
        vec![
            sse_event(
                r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress","incomplete_details":null,"usage":null},"sequence_number":0}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.added","item":{"id":"msg_051","type":"message","status":"in_progress","content":[],"role":"assistant"},"output_index":0,"sequence_number":2}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":"Dummy","item_id":"msg_051","output_index":0,"sequence_number":4}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":" PDF","item_id":"msg_051","output_index":0,"sequence_number":5}"#,
            ),
            sse_event(
                r#"{"type":"response.output_text.delta","content_index":0,"delta":" file","item_id":"msg_051","output_index":0,"sequence_number":6}"#,
            ),
            sse_event(
                r#"{"type":"response.output_item.done","item":{"id":"msg_051","type":"message","status":"completed","content":[{"type":"output_text","text":"Dummy PDF file"}],"role":"assistant"},"output_index":0,"sequence_number":9}"#,
            ),
            sse_event(
                r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","incomplete_details":null,"usage":{"input_tokens":44,"input_tokens_details":{"cached_tokens":0},"output_tokens":4,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":48}},"sequence_number":10}"#,
            ),
        ]
    }

    #[tokio::test]
    async fn stream_pdf_input() {
        let server = MockServer::start().await;
        let chunks = pdf_stream_chunks();
        let body = sse_body(
            &chunks
                .iter()
                .map(std::string::String::as_str)
                .collect::<Vec<_>>(),
        );
        mock_sse(&server, body).await;

        let model = make_model(&server, "gpt-4.1-nano");
        let prompt = vec![LanguageModelMessage::User {
            content: vec![
UserPart::Text(TextPart {
text: "What text does this PDF contain?".into(),
provider_options: None
}),
UserPart::File(FilePart {
data: FileData::Url {
original_url: None,
url: "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf".into()
},
media_type: "application/pdf".into(),
filename: None,
provider_options: None
})
],
            provider_options: None,
        }];
        let result = model.do_stream(&default_options(prompt)).await.unwrap();
        let parts = collect_stream(result).await;

        // Collect all text deltas
        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                StreamPart::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Dummy PDF file");

        // Verify finish
        let finish = parts.iter().find_map(|p| match p {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some((finish_reason, usage)),
            _ => None,
        });
        assert!(finish.is_some());
        let (fr, usage) = finish.unwrap();
        assert_eq!(fr.unified, FinishReasonUnified::Stop);
        assert_eq!(usage.input_tokens.total, Some(44));
        assert_eq!(usage.output_tokens.total, Some(4));
    }

    #[tokio::test]
    async fn stream_tool_call_with_proto_item_id() {
        // Test that "__proto__" as item_id doesn't cause issues (Rust HashMap is safe).
        let server = MockServer::start().await;
        let body = sse_body(&[
            &sse_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"__proto__","output_index":0,"arguments":"polluted","sequence_number":0}"#,
            ),
            &sse_event(
                r#"{"type":"response.completed","response":{"incomplete_details":null,"status":"completed","usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":0}},"sequence_number":1}"#,
            ),
        ]);
        mock_sse(&server, body).await;

        let model = make_model(&server, "gemma-7b-it");
        let result = model
            .do_stream(&default_options(test_prompt()))
            .await
            .unwrap();

        // Should not panic or error.
        let _parts = collect_stream(result).await;
    }
}
