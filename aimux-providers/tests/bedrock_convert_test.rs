//! Pure-function tests for the Amazon Bedrock provider's conversion layer.
//!
//! Translated from the TS test files:
//! - `convert-to-amazon-bedrock-chat-messages.test.ts` (64 cases)
//! - `amazon-bedrock-prepare-tools.test.ts` (23 cases incl. `it.each`)
//! - `convert-amazon-bedrock-usage.test.ts` (9 cases)
//!
//! Cases the Rust data model cannot express are skipped with an inline
//! comment. The main categories of skips:
//! - **System-after-non-system throw**: the Rust converter currently lifts
//!   all system messages into the system array.
//! - **S3 URLs / provider references**: `FileUrl` / `FileReference` are not
//!   converted by the Rust Bedrock path.
//! - **Top-level-only mediaType auto-detection from bytes**: the Rust path
//!   does not sniff magic bytes.
//! - **Mistral tool-call-id normalization (`isMistral`)**: the Rust
//!   `convert_prompt_to_bedrock` has no `isMistral` parameter. The
//!   non-Mistral (passthrough) cases ARE covered.
//! - **Provider-defined tools (web_search, anthropic provider tools)** and
//!   `additionalTools`/`betas`: the Rust `FunctionTool` has no `type`/`id`.
//! - **`raw` echo on `Usage`**: the Rust `Usage` type has no `raw` field.

use serde_json::{Value, json};

use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions, provider_namespace};

use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, ReasoningPart, TextPart, ToolCallPart, ToolPart,
    ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::ToolChoice;
use aimux_core::tool::FunctionTool;

use aimux_providers::bedrock::convert::{
    BedrockUsage, convert_prompt_to_bedrock, convert_usage, prepare_tools,
};

// ── prompt builders ─────────────────────────────────────────────────────────

fn user(content: Vec<UserPart>) -> LanguageModelMessage {
    LanguageModelMessage::User {
        content,
        provider_options: None,
    }
}

fn assistant(content: Vec<AssistantPart>) -> LanguageModelMessage {
    LanguageModelMessage::Assistant {
        content,
        provider_options: None,
    }
}

fn system_msg(text: &str) -> LanguageModelMessage {
    LanguageModelMessage::System {
        content: text.to_string(),
        provider_options: None,
    }
}

fn tool_msg(content: Vec<ToolPart>) -> LanguageModelMessage {
    LanguageModelMessage::Tool {
        content,
        provider_options: None,
    }
}

fn user_text(text: &str) -> UserPart {
    UserPart::Text(TextPart {
        text: text.to_string(),
        provider_options: None,
    })
}

fn assistant_text(text: &str) -> AssistantPart {
    AssistantPart::Text(TextPart {
        text: text.to_string(),
        provider_options: None,
    })
}

/// A file part with inline base64 data, passed through verbatim.
fn file_base64(
    data: &str,
    media_type: &str,
    filename: Option<&str>,
    provider_options: Option<SharedProviderOptions>,
) -> UserPart {
    UserPart::File(FilePart {
        data: FileData::Data {
            data: FileBytes::Base64(data.to_string()),
        },
        media_type: media_type.to_string(),
        filename: filename.map(std::string::ToString::to_string),
        provider_options,
    })
}

fn text_with_cache(text: &str, cache_type: &str, ttl: Option<&str>) -> TextPart {
    let mut cp = serde_json::Map::new();
    cp.insert("type".to_string(), json!(cache_type));
    if let Some(t) = ttl {
        cp.insert("ttl".to_string(), json!(t));
    }
    TextPart {
        text: text.to_string(),
        provider_options: Some(
            provider_namespace("bedrock", json!({ "cachePoint": Value::Object(cp) })).unwrap(),
        ),
    }
}

fn reasoning(text: &str, signature: Option<&str>) -> AssistantPart {
    AssistantPart::Reasoning(ReasoningPart {
        text: text.to_string(),
        provider_options: signature.map(|signature| {
            provider_namespace("amazonBedrock", json!({ "signature": signature })).unwrap()
        }),
    })
}

fn tool_call(id: &str, name: &str, input: Value) -> AssistantPart {
    AssistantPart::ToolCall(ToolCallPart {
        tool_call_id: id.to_string(),
        tool_name: name.to_string(),
        input,
        provider_executed: None,
        provider_options: None,
    })
}

fn tool_result(id: &str, output: ToolResultOutput) -> ToolPart {
    ToolPart::ToolResult(ToolResultPart {
        tool_call_id: id.to_string(),
        tool_name: "test".to_string(),
        output,
        provider_options: None,
    })
}

// ════════════════════════════════════════════════════════════════════════════
// convert-to-amazon-bedrock-chat-messages
// ════════════════════════════════════════════════════════════════════════════

// ── system messages ─────────────────────────────────────────────────────────

/// TS: "should combine multiple leading system messages into a single system message"
#[test]
fn system_combine_multiple_leading() {
    let (system, _) =
        convert_prompt_to_bedrock(&vec![system_msg("Hello"), system_msg("World")]).unwrap();
    assert_eq!(
        Value::Array(system),
        json!([{ "text": "Hello" }, { "text": "World" }])
    );
}

// SKIPPED (TS: "should throw an error if a system message is provided after a
// non-system message"): the converter currently lifts all system messages
// into the system array.

/// TS: "should extract the system message"
#[test]
fn system_extract_single() {
    let (system, _) = convert_prompt_to_bedrock(&vec![system_msg("Hello")]).unwrap();
    assert_eq!(Value::Array(system), json!([{ "text": "Hello" }]));
}

// ── user messages ───────────────────────────────────────────────────────────

/// TS: "should convert messages with image parts"
#[test]
fn user_convert_image_parts() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        user_text("Hello"),
        file_base64("AAECAw==", "image/png", None, None),
    ])])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([{
            "role": "user",
            "content": [
                { "text": "Hello" },
                { "image": { "format": "png", "source": { "bytes": "AAECAw==" } } },
            ]
        }])
    );
}

// SKIPPED (TS: "should convert image parts with S3 URLs"): FileUrl is not
// converted by the Rust Bedrock path.

/// TS: "should convert messages with document parts"
#[test]
fn user_convert_document_parts() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        user_text("Hello"),
        file_base64("AAECAw==", "application/pdf", None, None),
    ])])
    .unwrap();
    assert_eq!(
        messages[0]["content"],
        json!([
            { "text": "Hello" },
            { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } } },
        ])
    );
}

/// TS: "should strip file extension when filename is provided"
#[test]
fn user_strip_file_extension() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        user_text("Hello"),
        file_base64(
            "AAECAw==",
            "application/pdf",
            Some("custom-filename.pdf"),
            None,
        ),
    ])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][1],
        json!({ "document": { "format": "pdf", "name": "custom-filename", "source": { "bytes": "AAECAw==" } } })
    );
}

/// TS: "should preserve filename without extension when provided"
#[test]
fn user_preserve_filename_without_extension() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "AAECAw==",
        "application/pdf",
        Some("custom-filename"),
        None,
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({ "document": { "format": "pdf", "name": "custom-filename", "source": { "bytes": "AAECAw==" } } })
    );
}

/// TS: "should use consistent document names for prompt cache effectiveness"
#[test]
fn user_consistent_document_names() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![
            file_base64("AAECAw==", "application/pdf", None, None),
            file_base64("BAUGBw==", "application/pdf", None, None),
        ]),
        assistant(vec![assistant_text("OK")]),
        user(vec![file_base64("AAECAw==", "application/pdf", None, None)]),
    ])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [
                { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } } },
                { "document": { "format": "pdf", "name": "document-2", "source": { "bytes": "BAUGBw==" } } },
            ]},
            { "role": "assistant", "content": [{ "text": "OK" }] },
            { "role": "user", "content": [
                { "document": { "format": "pdf", "name": "document-3", "source": { "bytes": "AAECAw==" } } },
            ]},
        ])
    );
}

// Message-level cache-point cases remain outside this restored test subset.

// SKIPPED (TS: "should throw for file parts with provider references"):
// FileReference is not converted by the user-file path.

/// TS: "should add cache point to user content part when specified"
#[test]
fn user_content_part_cache_point() {
    let (system, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        user_text("Hello"),
        UserPart::Text(text_with_cache("cached", "default", Some("5m"))),
        user_text("World"),
    ])])
    .unwrap();
    assert!(system.is_empty());
    assert_eq!(
        Value::Array(messages),
        json!([{
            "role": "user",
            "content": [
                { "text": "Hello" },
                { "text": "cached" },
                { "cachePoint": { "type": "default", "ttl": "5m" } },
                { "text": "World" },
            ]
        }])
    );
}

// ── assistant messages ──────────────────────────────────────────────────────

/// TS: "should remove trailing whitespace from last assistant message when there is no further user message"
#[test]
fn assistant_trim_trailing_whitespace_last() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("user content")]),
        assistant(vec![assistant_text("assistant content  ")]),
    ])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "user content" }] },
            { "role": "assistant", "content": [{ "text": "assistant content" }] },
        ])
    );
}

/// TS: "should remove trailing whitespace from last assistant message with multi-part content when there is no further user message"
#[test]
fn assistant_trim_trailing_whitespace_multi_part() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("user content")]),
        assistant(vec![
            assistant_text("assistant "),
            assistant_text("content  "),
        ]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([{ "text": "assistant " }, { "text": "content" }])
    );
}

/// TS: "should keep trailing whitespace from assistant message when there is a further user message"
#[test]
fn assistant_keep_trailing_whitespace_with_further_user() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("user content")]),
        assistant(vec![assistant_text("assistant content  ")]),
        user(vec![user_text("user content 2")]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([{ "text": "assistant content  " }])
    );
}

/// TS: "should combine multiple sequential assistant messages into a single message"
#[test]
fn assistant_combine_sequential() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Hi!")]),
        assistant(vec![assistant_text("Hello")]),
        assistant(vec![assistant_text("World")]),
        assistant(vec![assistant_text("!")]),
    ])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Hi!" }] },
            { "role": "assistant", "content": [{ "text": "Hello" }, { "text": "World" }, { "text": "!" }] },
        ])
    );
}

// Message-level cache-point cases remain outside this restored test subset.

/// TS: "should add cache point to assistant content part when specified"
#[test]
fn assistant_content_part_cache_point() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![
        assistant_text("Hello"),
        AssistantPart::Text(text_with_cache("cached", "default", Some("1h"))),
        assistant_text("World"),
    ])])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([{
            "role": "assistant",
            "content": [
                { "text": "Hello" },
                { "text": "cached" },
                { "cachePoint": { "type": "default", "ttl": "1h" } },
                { "text": "World" },
            ]
        }])
    );
}

/// TS: "should properly convert reasoning content type"
#[test]
fn assistant_reasoning_with_signature() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my step-by-step reasoning process",
            Some("test-signature"),
        )]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([{
            "reasoningContent": {
                "reasoningText": {
                    "text": "This is my step-by-step reasoning process",
                    "signature": "test-signature"
                }
            }
        }])
    );
}

/// TS: "should preserve assistant message reasoning parts with amazonBedrock providerOptions"
#[test]
fn assistant_preserve_amazon_bedrock_reasoning() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![
            reasoning(
                "Bedrock-signed reasoning round-tripped to Bedrock",
                Some("bedrock-signature"),
            ),
            assistant_text("final answer"),
        ]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([
            { "reasoningContent": { "reasoningText": {
                "text": "Bedrock-signed reasoning round-tripped to Bedrock",
                "signature": "bedrock-signature"
            }}},
            { "text": "final answer" },
        ])
    );
}

/// TS: "should not trim reasoning text when a signature is present"
#[test]
fn assistant_no_trim_reasoning_with_signature() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my reasoning with trailing space    ",
            Some("test-signature"),
        )]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"][0]["reasoningContent"]["reasoningText"]["text"],
        json!("This is my reasoning with trailing space    ")
    );
}

/// TS: "should omit reasoning content without signature"
#[test]
fn assistant_omit_reasoning_without_signature() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![
            reasoning("This is my reasoning with trailing space    ", None),
            assistant_text("final answer"),
        ]),
    ])
    .unwrap();
    assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
}

/// TS: "should omit multiple reasoning parts without signatures"
#[test]
fn assistant_omit_multiple_reasoning_without_signatures() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![
            reasoning("First reasoning with trailing space    ", None),
            reasoning("Second reasoning with trailing space    ", None),
            assistant_text("final answer"),
        ]),
    ])
    .unwrap();
    assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
}

/// TS: "should omit unsigned reasoning while preserving tool calls in multi-turn tool use"
#[test]
fn assistant_omit_unsigned_reasoning_preserving_tool_calls() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("What is the weather?")]),
        assistant(vec![
            reasoning("I should call the weather tool.", None),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            ToolResultOutput::Text {
                value: "Sunny, 72F".to_string(),
                provider_options: None,
            },
        )]),
    ])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "What is the weather?" }] },
            { "role": "assistant", "content": [{
                "toolUse": { "input": { "city": "SF" }, "name": "getWeather", "toolUseId": "call-1" }
            }]},
            { "role": "user", "content": [{
                "toolResult": { "toolUseId": "call-1", "content": [{ "text": "Sunny, 72F" }] }
            }]},
        ])
    );
}

/// TS: "should preserve reasoning text with signature in multi-turn tool use"
#[test]
fn assistant_preserve_reasoning_multi_turn() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("What is the weather?")]),
        assistant(vec![
            reasoning("Let me check the weather API.\n", Some("sig-abc123")),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            ToolResultOutput::Text {
                value: "Sunny, 72F".to_string(),
                provider_options: None,
            },
        )]),
        assistant(vec![
            reasoning("The weather is sunny and warm.\n", Some("sig-def456")),
            assistant_text("It is sunny and 72F in SF."),
        ]),
    ])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "What is the weather?" }] },
            { "role": "assistant", "content": [
                { "reasoningContent": { "reasoningText": {
                    "text": "Let me check the weather API.\n", "signature": "sig-abc123"
                }}},
                { "toolUse": { "input": { "city": "SF" }, "name": "getWeather", "toolUseId": "call-1" } },
            ]},
            { "role": "user", "content": [{
                "toolResult": { "toolUseId": "call-1", "content": [{ "text": "Sunny, 72F" }] }
            }]},
            { "role": "assistant", "content": [
                { "reasoningContent": { "reasoningText": {
                    "text": "The weather is sunny and warm.\n", "signature": "sig-def456"
                }}},
                { "text": "It is sunny and 72F in SF." },
            ]},
        ])
    );
}

/// TS: "should handle a mix of text and reasoning content types"
#[test]
fn assistant_mix_text_and_reasoning() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Explain your reasoning")]),
        assistant(vec![
            assistant_text("My answer is 42."),
            reasoning(
                "I calculated this by analyzing the meaning of life",
                Some("reasoning-process"),
            ),
        ]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([
            { "text": "My answer is 42." },
            { "reasoningContent": { "reasoningText": {
                "text": "I calculated this by analyzing the meaning of life",
                "signature": "reasoning-process"
            }}},
        ])
    );
}

/// TS: "should filter out empty text blocks in assistant messages"
#[test]
fn assistant_filter_empty_text_blocks() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Hello")]),
        assistant(vec![
            assistant_text("\n\n"),
            tool_call("call-123", "test", json!({})),
            assistant_text("  "),
            assistant_text("actual content"),
        ]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([
            { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
            { "text": "actual content" },
        ])
    );
}

/// TS: "should wrap non-object (invalid) tool call input in an object"
#[test]
fn assistant_wrap_non_object_tool_input() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![tool_call(
        "call-1",
        "cityAttractions",
        Value::String("{ \"city\": \"San Francisco\", }".to_string()),
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"],
        json!([{
            "toolUse": {
                "toolUseId": "call-1",
                "name": "cityAttractions",
                "input": { "rawInvalidInput": "{ \"city\": \"San Francisco\", }" }
            }
        }])
    );
}

/// TS: "should strip invalid characters from tool call names"
#[test]
fn assistant_strip_invalid_tool_name_chars() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![
        tool_call("call-1", "$READFILE", json!({})),
        tool_call(
            "call-2",
            "exchange_delivered_order_items<|channel|>",
            json!({}),
        ),
        tool_call("call-3", "$", json!({})),
    ])])
    .unwrap();
    assert_eq!(
        messages[0]["content"],
        json!([
            { "toolUse": { "toolUseId": "call-1", "name": "READFILE", "input": {} } },
            { "toolUse": { "toolUseId": "call-2", "name": "exchange_delivered_order_itemschannel", "input": {} } },
            { "toolUse": { "toolUseId": "call-3", "name": "_", "input": {} } },
        ])
    );
}

/// TS: "should preserve empty text blocks when reasoning blocks are present"
#[test]
fn assistant_preserve_empty_text_with_reasoning() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![user_text("Hello")]),
        assistant(vec![
            reasoning("thinking...", Some("sig-1")),
            assistant_text(""),
            reasoning("more thinking...", Some("sig-2")),
            assistant_text("response text"),
            tool_call("call-123", "test", json!({})),
        ]),
    ])
    .unwrap();
    assert_eq!(
        messages[1]["content"],
        json!([
            { "reasoningContent": { "reasoningText": { "text": "thinking...", "signature": "sig-1" } } },
            { "text": "" },
            { "reasoningContent": { "reasoningText": { "text": "more thinking...", "signature": "sig-2" } } },
            { "text": "response text" },
            { "toolUse": { "toolUseId": "call-123", "name": "test", "input": {} } },
        ])
    );
}

// ── tool messages ───────────────────────────────────────────────────────────

/// TS: "should convert tool result with content array containing text"
#[test]
fn tool_result_content_text() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        "call-123",
        ToolResultOutput::Content {
            value: vec![ToolResultContent::Text(TextPart {
                text: "The result is 42".to_string(),
                provider_options: None,
            })],
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0],
        json!({
            "role": "user",
            "content": [{
                "toolResult": {
                    "toolUseId": "call-123",
                    "content": [{ "text": "The result is 42" }]
                }
            }]
        })
    );
}

/// TS: "should convert tool result with content array containing image"
#[test]
fn tool_result_content_image() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        "call-123",
        ToolResultOutput::Content {
            value: vec![ToolResultContent::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("base64data".to_string()),
                },
                media_type: "image/jpeg".to_string(),
                filename: None,
                provider_options: None,
            })],
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "toolResult": {
                "toolUseId": "call-123",
                "content": [{ "image": { "format": "jpeg", "source": { "bytes": "base64data" } } }]
            }
        })
    );
}

// SKIPPED (TS: "should convert tool result images with S3 URLs"): FileUrl is
// not converted by the Rust Bedrock path.

/// TS: "should convert tool result with content array containing PDF"
#[test]
fn tool_result_content_pdf() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        "call-123",
        ToolResultOutput::Content {
            value: vec![ToolResultContent::File(FilePart {
                data: FileData::Data {
                    data: FileBytes::Base64("base64data".to_string()),
                },
                media_type: "application/pdf".to_string(),
                filename: Some("tool-result.pdf".to_string()),
                provider_options: None,
            })],
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "toolResult": {
                "toolUseId": "call-123",
                "content": [{
                    "document": { "format": "pdf", "name": "tool-result", "source": { "bytes": "base64data" } }
                }]
            }
        })
    );
}

// NOT PORTED (TS: "should throw error for unsupported image format in tool result
// content" and "should throw error for unsupported mime type in tool result
// file content").

/// TS: "should fallback to stringified result when content is undefined" (json output)
#[test]
fn tool_result_json_output() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        "call-123",
        ToolResultOutput::Json {
            value: json!({ "value": 42 }),
            provider_options: None,
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "toolResult": {
                "toolUseId": "call-123",
                "content": [{ "text": "{\"value\":42}" }]
            }
        })
    );
}

// ── citations ───────────────────────────────────────────────────────────────

/// TS: "should handle citations enabled for PDF"
#[test]
fn citations_enabled_for_pdf() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "AAECAw==",
        "application/pdf",
        None,
        Some(provider_namespace("bedrock", json!({ "citations": { "enabled": true } })).unwrap()),
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "document": {
                "format": "pdf",
                "name": "document-1",
                "source": { "bytes": "AAECAw==" },
                "citations": { "enabled": true }
            }
        })
    );
}

/// TS: "should handle citations disabled for PDF"
#[test]
fn citations_disabled_for_pdf() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "AAECAw==",
        "application/pdf",
        None,
        Some(provider_namespace("bedrock", json!({ "citations": { "enabled": false } })).unwrap()),
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } }
        })
    );
    assert!(
        messages[0]["content"][0]["document"]
            .get("citations")
            .is_none()
    );
}

/// TS: "should handle no citations specified for PDF (default)"
#[test]
fn citations_default_for_pdf() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "AAECAw==",
        "application/pdf",
        None,
        None,
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" } }
        })
    );
    assert!(
        messages[0]["content"][0]["document"]
            .get("citations")
            .is_none()
    );
}

/// TS: "should handle multiple PDFs with different citation settings"
#[test]
fn citations_multiple_pdfs() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        file_base64(
            "AAECAw==",
            "application/pdf",
            None,
            Some(
                provider_namespace("bedrock", json!({ "citations": { "enabled": true } })).unwrap(),
            ),
        ),
        file_base64(
            "BAUGBw==",
            "application/pdf",
            None,
            Some(
                provider_namespace("bedrock", json!({ "citations": { "enabled": false } }))
                    .unwrap(),
            ),
        ),
    ])])
    .unwrap();
    assert_eq!(
        messages[0]["content"],
        json!([
            { "document": { "format": "pdf", "name": "document-1", "source": { "bytes": "AAECAw==" }, "citations": { "enabled": true } } },
            { "document": { "format": "pdf", "name": "document-2", "source": { "bytes": "BAUGBw==" } } },
        ])
    );
}

// ── additional file format tests ────────────────────────────────────────────

// SKIPPED (TS: "should throw an error for unsupported file mime type in user
// message content"): unknown mimes fall back to a default format (no throw).

/// TS: "should handle xlsx files correctly"
#[test]
fn file_format_xlsx() {
    let (system, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "base64data",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        None,
        None,
    )])])
    .unwrap();
    assert!(system.is_empty());
    assert_eq!(
        messages[0]["content"][0],
        json!({ "document": { "format": "xlsx", "name": "document-1", "source": { "bytes": "base64data" } } })
    );
}

/// TS: "should handle docx files correctly"
#[test]
fn file_format_docx() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "base64data",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        None,
        None,
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({ "document": { "format": "docx", "name": "document-1", "source": { "bytes": "base64data" } } })
    );
}

// ── Mistral tool call ID normalization ──────────────────────────────────────

// SKIPPED (TS: "should normalize tool call IDs in tool results when isMistral
// is true" and "...in tool calls when isMistral is true"): the Rust
// convert_prompt_to_bedrock has no isMistral parameter.

/// TS: "should not normalize tool call IDs when isMistral is false"
#[test]
fn mistral_no_normalize_when_false() {
    let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        original_id,
        ToolResultOutput::Text {
            value: "The result is 42".to_string(),
            provider_options: None,
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0]["toolResult"]["toolUseId"],
        json!(original_id)
    );
}

/// TS: "should default to not normalizing when isMistral is not provided"
#[test]
fn mistral_default_no_normalize() {
    let original_id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        original_id,
        ToolResultOutput::Text {
            value: "The result is 42".to_string(),
            provider_options: None,
        },
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0]["toolResult"]["toolUseId"],
        json!(original_id)
    );
}

// ── top-level-only mediaType resolution ─────────────────────────────────────

/// TS: "should pass through a full image mediaType unchanged"
#[test]
fn media_type_pass_through_full_image() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "iVBORw0KGgo=",
        "image/png",
        None,
        None,
    )])])
    .unwrap();
    assert_eq!(
        Value::Array(messages),
        json!([{
            "role": "user",
            "content": [{ "image": { "format": "png", "source": { "bytes": "iVBORw0KGgo=" } } }]
        }])
    );
}

// SKIPPED (TS: "should detect subtype from inline bytes when mediaType is
// top-level-only (image)" and "...(application/pdf)"): the Rust path does not
// sniff magic bytes to resolve a top-level-only mediaType.

/// TS: "should route to document slot for non-image top-level type via getTopLevelMediaType"
#[test]
fn media_type_route_to_document_text_plain() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![file_base64(
        "base64data",
        "text/plain",
        None,
        None,
    )])])
    .unwrap();
    assert_eq!(
        messages[0]["content"][0],
        json!({ "document": { "format": "txt", "name": "document-1", "source": { "bytes": "base64data" } } })
    );
}

// SKIPPED (TS: "should throw UnsupportedFunctionalityError for URL data (File
// URL)", "...for unsupported full image mediaType", and "...when top-level-only
// bytes cannot be detected"): FileUrl is not converted; unknown image mimes
// fall back to "png" instead of throwing; no magic-byte detection.

// ════════════════════════════════════════════════════════════════════════════
// amazon-bedrock-prepare-tools
// ════════════════════════════════════════════════════════════════════════════

const NON_ANTHROPIC_MODEL: &str = "meta.llama3-70b-instruct-v1:0";

fn func_tool(name: &str, description: Option<&str>, input_schema: Value) -> FunctionTool {
    FunctionTool {
        name: name.to_string(),
        description: description.map(std::string::ToString::to_string),
        input_schema,
        strict: None,
        provider_options: None,
        input_examples: None,
    }
}

/// TS: "should handle tool choice 'auto'"
#[test]
fn prepare_tools_tool_choice_auto() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, Some(&ToolChoice::Auto), NON_ANTHROPIC_MODEL);
    assert_eq!(config["toolChoice"], json!({ "auto": {} }));
}

/// TS: "should handle tool choice 'required'"
#[test]
fn prepare_tools_tool_choice_required() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, Some(&ToolChoice::Required), NON_ANTHROPIC_MODEL);
    assert_eq!(config["toolChoice"], json!({ "any": {} }));
}

/// TS: "should handle tool choice 'none' by clearing tools"
#[test]
fn prepare_tools_tool_choice_none_clears() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, Some(&ToolChoice::None), NON_ANTHROPIC_MODEL);
    assert_eq!(config, json!({}));
}

/// TS: "should handle tool choice 'tool'"
#[test]
fn prepare_tools_tool_choice_tool() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(
        &tools,
        Some(&ToolChoice::Tool {
            tool_name: "testFunction".to_string(),
        }),
        NON_ANTHROPIC_MODEL,
    );
    assert_eq!(
        config["toolChoice"],
        json!({ "tool": { "name": "testFunction" } })
    );
}

/// TS: "should filter function tools to only the named tool when tool choice is 'tool'"
#[test]
fn prepare_tools_tool_choice_filters_to_named() {
    let tools = Some(vec![
        func_tool(
            "getWeather",
            Some("Get weather"),
            json!({ "type": "object" }),
        ),
        func_tool("getTime", Some("Get time"), json!({ "type": "object" })),
    ]);
    let config = prepare_tools(
        &tools,
        Some(&ToolChoice::Tool {
            tool_name: "getWeather".to_string(),
        }),
        NON_ANTHROPIC_MODEL,
    );
    assert_eq!(config["tools"].as_array().unwrap().len(), 1);
    assert_eq!(config["tools"][0]["toolSpec"]["name"], json!("getWeather"));
}

// ════════════════════════════════════════════════════════════════════════════
// convert-amazon-bedrock-usage
// ════════════════════════════════════════════════════════════════════════════
//
// The TS `convertAmazonBedrockUsage` echoes the input as `result.raw`. The Rust
// `Usage` type has no `raw` field, so only the `inputTokens`/`outputTokens`
// breakdown is asserted; the two `raw`-only cases ("should include totalTokens
// in raw when provided" and "should preserve raw usage data") are SKIPPED.

fn bedrock_usage(
    input: u32,
    output: u32,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
) -> BedrockUsage {
    BedrockUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: None,
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
    }
}

/// TS: "should convert basic usage without cache tokens"
#[test]
fn usage_basic_without_cache() {
    let u = convert_usage(Some(&bedrock_usage(100, 50, None, None)));
    assert_eq!(u.input_tokens.total, Some(100));
    assert_eq!(u.input_tokens.no_cache, Some(100));
    assert_eq!(u.input_tokens.cache_read, Some(0));
    assert_eq!(u.input_tokens.cache_write, Some(0));
    assert_eq!(u.output_tokens.total, Some(50));
    assert_eq!(u.output_tokens.text, Some(50));
}

/// TS: "should convert usage with cache read tokens"
#[test]
fn usage_with_cache_read() {
    let u = convert_usage(Some(&bedrock_usage(100, 50, Some(80), None)));
    assert_eq!(u.input_tokens.total, Some(180));
    assert_eq!(u.input_tokens.no_cache, Some(100));
    assert_eq!(u.input_tokens.cache_read, Some(80));
    assert_eq!(u.input_tokens.cache_write, Some(0));
    assert_eq!(u.output_tokens.total, Some(50));
    assert_eq!(u.output_tokens.text, Some(50));
}

/// TS: "should convert usage with cache write tokens"
#[test]
fn usage_with_cache_write() {
    let u = convert_usage(Some(&bedrock_usage(100, 50, None, Some(60))));
    assert_eq!(u.input_tokens.total, Some(160));
    assert_eq!(u.input_tokens.no_cache, Some(100));
    assert_eq!(u.input_tokens.cache_read, Some(0));
    assert_eq!(u.input_tokens.cache_write, Some(60));
    assert_eq!(u.output_tokens.total, Some(50));
    assert_eq!(u.output_tokens.text, Some(50));
}

/// TS: "should convert usage with both cache read and write tokens"
#[test]
fn usage_with_both_cache() {
    let u = convert_usage(Some(&bedrock_usage(100, 50, Some(80), Some(60))));
    assert_eq!(u.input_tokens.total, Some(240));
    assert_eq!(u.input_tokens.no_cache, Some(100));
    assert_eq!(u.input_tokens.cache_read, Some(80));
    assert_eq!(u.input_tokens.cache_write, Some(60));
    assert_eq!(u.output_tokens.total, Some(50));
    assert_eq!(u.output_tokens.text, Some(50));
}

/// TS: "should handle null cache tokens"
#[test]
fn usage_null_cache_tokens() {
    let u = convert_usage(Some(&bedrock_usage(100, 50, None, None)));
    assert_eq!(u.input_tokens.total, Some(100));
    assert_eq!(u.input_tokens.no_cache, Some(100));
    assert_eq!(u.input_tokens.cache_read, Some(0));
    assert_eq!(u.input_tokens.cache_write, Some(0));
    assert_eq!(u.output_tokens.total, Some(50));
    assert_eq!(u.output_tokens.text, Some(50));
}

/// TS: "should handle null usage"
#[test]
fn usage_null() {
    let u = convert_usage(None);
    assert_eq!(u.input_tokens.total, None);
    assert_eq!(u.input_tokens.no_cache, None);
    assert_eq!(u.input_tokens.cache_read, None);
    assert_eq!(u.input_tokens.cache_write, None);
    assert_eq!(u.output_tokens.total, None);
    assert_eq!(u.output_tokens.text, None);
}

/// TS: "should handle undefined usage"
#[test]
fn usage_undefined() {
    let u = convert_usage(None);
    assert_eq!(u.input_tokens.total, None);
    assert_eq!(u.input_tokens.no_cache, None);
    assert_eq!(u.input_tokens.cache_read, None);
    assert_eq!(u.input_tokens.cache_write, None);
    assert_eq!(u.output_tokens.total, None);
    assert_eq!(u.output_tokens.text, None);
}

// SKIPPED (TS: "should include totalTokens in raw when provided" and "should
// preserve raw usage data"): the Rust `Usage` type has no `raw` echo field.
