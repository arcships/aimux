//! Pure-function tests for the Amazon Bedrock provider's conversion layer.
//!
//! Translated from the TS test files:
//! - `convert-to-amazon-bedrock-chat-messages.test.ts` (64 cases)
//! - `amazon-bedrock-prepare-tools.test.ts` (23 cases incl. `it.each`)
//! - `convert-amazon-bedrock-usage.test.ts` (9 cases)
//!
//! Cases the Rust data model cannot express are skipped with an inline
//! comment. The main categories of skips:
//! - **Message-level `providerOptions`**: `LanguageModelPromptMessage` has no
//!   `provider_options` field, so message-level cache points (system / user /
//!   assistant message `bedrock.cachePoint`) cannot be expressed. Part-level
//!   cache points (on `ContentPart::Text` `provider_options`) ARE covered.
//! - **System-after-non-system throw / unsupported-mime throws**: the Rust
//!   `convert_prompt_to_bedrock` returns `(system, messages)` (no `Result`),
//!   so it cannot surface `UnsupportedFunctionalityError`.
//! - **S3 URLs / provider references**: `FileUrl` / `FileReference` are not
//!   converted by the Rust Bedrock path.
//! - **Top-level-only mediaType auto-detection from bytes**: the Rust path
//!   does not sniff magic bytes.
//! - **Mistral tool-call-id normalization (`isMistral`)**: the Rust
//!   `convert_prompt_to_bedrock` has no `isMistral` parameter. The
//!   non-Mistral (passthrough) cases ARE covered.
//! - **Redacted reasoning / foreign-provider reasoning**: `ContentPart::Reasoning`
//!   carries only `signature` (no `redactedData`, no provider distinction).
//! - **Provider-defined tools (web_search, anthropic provider tools)** and
//!   `additionalTools`/`betas`: the Rust `FunctionTool` has no `type`/`id`.
//! - **`raw` echo on `Usage`**: the Rust `Usage` type has no `raw` field.

use serde_json::{Value, json};

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::options::ToolChoice;
use aimux_core::tool::FunctionTool;

use aimux_providers::bedrock::convert::{
    BedrockUsage, convert_prompt_to_bedrock, convert_usage, prepare_tools, supports_strict_tools,
};

// ── prompt builders ─────────────────────────────────────────────────────────

fn msg(role: Role, content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content,
        provider_options: None,
    }
}

fn user(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::User, content)
}

fn assistant(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::Assistant, content)
}

fn system_msg(text: &str) -> LanguageModelPromptMessage {
    msg(Role::System, vec![ContentPart::text(text)])
}

fn tool_msg(content: Vec<ContentPart>) -> LanguageModelPromptMessage {
    msg(Role::Tool, content)
}

/// A `file` part whose inline data is an already-base64 string — mirrors the
/// TS `file` part with `data: { type: 'data', data: '<base64>' }` (the Rust
/// `ContentPart::FileBase64` holds the base64 string verbatim).
fn file_base64(
    data: &str,
    media_type: &str,
    filename: Option<&str>,
    provider_options: Option<Value>,
) -> ContentPart {
    ContentPart::FileBase64 {
        data: data.to_string(),
        media_type: media_type.to_string(),
        filename: filename.map(std::string::ToString::to_string),
        provider_options,
    }
}

fn text_with_cache(text: &str, cache_type: &str, ttl: Option<&str>) -> ContentPart {
    let mut cp = serde_json::Map::new();
    cp.insert("type".to_string(), json!(cache_type));
    if let Some(t) = ttl {
        cp.insert("ttl".to_string(), json!(t));
    }
    ContentPart::Text {
        text: text.to_string(),
        provider_options: Some(json!({ "bedrock": { "cachePoint": Value::Object(cp) } })),
    }
}

fn reasoning(text: &str, signature: Option<&str>) -> ContentPart {
    ContentPart::Reasoning {
        text: text.to_string(),
        signature: signature.map(std::string::ToString::to_string),
        provider_options: None,
    }
}

fn tool_call(id: &str, name: &str, input: Value) -> ContentPart {
    ContentPart::tool_call(id.to_string(), name.to_string(), input)
}

fn tool_result(id: &str, output: Value) -> ContentPart {
    ContentPart::tool_result(id.to_string(), output)
}

const ANTHROPIC_MODEL: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";
const NON_ANTHROPIC_MODEL: &str = "meta.llama3-70b-instruct-v1:0";

// ════════════════════════════════════════════════════════════════════════════
// convert-to-amazon-bedrock-chat-messages
// ════════════════════════════════════════════════════════════════════════════

// ── system messages ─────────────────────────────────────────────────────────

/// TS: "should combine multiple leading system messages into a single system message"
#[test]
fn system_combine_multiple_leading() {
    let (system, _) = convert_prompt_to_bedrock(&vec![system_msg("Hello"), system_msg("World")]);
    assert_eq!(
        Value::Array(system),
        json!([{ "text": "Hello" }, { "text": "World" }])
    );
}

// SKIPPED (TS: "should throw an error if a system message is provided after a
// non-system message"): convert_prompt_to_bedrock returns (system, messages)
// with no Result, so it cannot surface UnsupportedFunctionalityError.

// SKIPPED (TS: system message cache point, 5m, 1h — 3 cases): message-level
// providerOptions are not modelled on LanguageModelPromptMessage.

/// TS: "should extract the system message"
#[test]
fn system_extract_single() {
    let (system, _) = convert_prompt_to_bedrock(&vec![system_msg("Hello")]);
    assert_eq!(Value::Array(system), json!([{ "text": "Hello" }]));
}

// ── user messages ───────────────────────────────────────────────────────────

/// TS: "should convert messages with image parts"
#[test]
fn user_convert_image_parts() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        ContentPart::text("Hello"),
        file_base64("AAECAw==", "image/png", None, None),
    ])]);
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
        ContentPart::text("Hello"),
        file_base64("AAECAw==", "application/pdf", None, None),
    ])]);
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
        ContentPart::text("Hello"),
        file_base64(
            "AAECAw==",
            "application/pdf",
            Some("custom-filename.pdf"),
            None,
        ),
    ])]);
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
    )])]);
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
        assistant(vec![ContentPart::text("OK")]),
        user(vec![file_base64("AAECAw==", "application/pdf", None, None)]),
    ]);
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

// SKIPPED (TS: user message cache point, 5m, 1h — 3 cases): message-level
// providerOptions are not modelled on LanguageModelPromptMessage.

// SKIPPED (TS: "should throw for file parts with provider references"):
// FileReference is not converted (no Result to surface the throw).

/// TS: "should add cache point to user content part when specified"
#[test]
fn user_content_part_cache_point() {
    let (system, messages) = convert_prompt_to_bedrock(&vec![user(vec![
        ContentPart::text("Hello"),
        text_with_cache("cached", "default", Some("5m")),
        ContentPart::text("World"),
    ])]);
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
        user(vec![ContentPart::text("user content")]),
        assistant(vec![ContentPart::text("assistant content  ")]),
    ]);
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
        user(vec![ContentPart::text("user content")]),
        assistant(vec![
            ContentPart::text("assistant "),
            ContentPart::text("content  "),
        ]),
    ]);
    assert_eq!(
        messages[1]["content"],
        json!([{ "text": "assistant " }, { "text": "content" }])
    );
}

/// TS: "should keep trailing whitespace from assistant message when there is a further user message"
#[test]
fn assistant_keep_trailing_whitespace_with_further_user() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("user content")]),
        assistant(vec![ContentPart::text("assistant content  ")]),
        user(vec![ContentPart::text("user content 2")]),
    ]);
    assert_eq!(
        messages[1]["content"],
        json!([{ "text": "assistant content  " }])
    );
}

/// TS: "should combine multiple sequential assistant messages into a single message"
#[test]
fn assistant_combine_sequential() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("Hi!")]),
        assistant(vec![ContentPart::text("Hello")]),
        assistant(vec![ContentPart::text("World")]),
        assistant(vec![ContentPart::text("!")]),
    ]);
    assert_eq!(
        Value::Array(messages),
        json!([
            { "role": "user", "content": [{ "text": "Hi!" }] },
            { "role": "assistant", "content": [{ "text": "Hello" }, { "text": "World" }, { "text": "!" }] },
        ])
    );
}

// SKIPPED (TS: assistant message cache point, 5m, 1h — 3 cases): message-level
// providerOptions are not modelled on LanguageModelPromptMessage.

/// TS: "should add cache point to assistant content part when specified"
#[test]
fn assistant_content_part_cache_point() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![assistant(vec![
        ContentPart::text("Hello"),
        text_with_cache("cached", "default", Some("1h")),
        ContentPart::text("World"),
    ])]);
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
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my step-by-step reasoning process",
            Some("test-signature"),
        )]),
    ]);
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

// SKIPPED (TS: "should properly convert redacted-reasoning content type"):
// ContentPart::Reasoning has no redactedData field.

// SKIPPED (TS: "should omit assistant message reasoning parts signed by a
// foreign provider"): ContentPart::Reasoning.signature carries no provider
// distinction, so foreign-provider (anthropic) signatures cannot be detected.

/// TS: "should preserve assistant message reasoning parts with amazonBedrock providerOptions"
#[test]
fn assistant_preserve_amazon_bedrock_reasoning() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning(
                "Bedrock-signed reasoning round-tripped to Bedrock",
                Some("bedrock-signature"),
            ),
            ContentPart::text("final answer"),
        ]),
    ]);
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
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![reasoning(
            "This is my reasoning with trailing space    ",
            Some("test-signature"),
        )]),
    ]);
    assert_eq!(
        messages[1]["content"][0]["reasoningContent"]["reasoningText"]["text"],
        json!("This is my reasoning with trailing space    ")
    );
}

/// TS: "should omit reasoning content without signature"
#[test]
fn assistant_omit_reasoning_without_signature() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning("This is my reasoning with trailing space    ", None),
            ContentPart::text("final answer"),
        ]),
    ]);
    assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
}

/// TS: "should omit multiple reasoning parts without signatures"
#[test]
fn assistant_omit_multiple_reasoning_without_signatures() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            reasoning("First reasoning with trailing space    ", None),
            reasoning("Second reasoning with trailing space    ", None),
            ContentPart::text("final answer"),
        ]),
    ]);
    assert_eq!(messages[1]["content"], json!([{ "text": "final answer" }]));
}

/// TS: "should omit unsigned reasoning while preserving tool calls in multi-turn tool use"
#[test]
fn assistant_omit_unsigned_reasoning_preserving_tool_calls() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![
        user(vec![ContentPart::text("What is the weather?")]),
        assistant(vec![
            reasoning("I should call the weather tool.", None),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            json!({ "type": "text", "value": "Sunny, 72F" }),
        )]),
    ]);
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
        user(vec![ContentPart::text("What is the weather?")]),
        assistant(vec![
            reasoning("Let me check the weather API.\n", Some("sig-abc123")),
            tool_call("call-1", "getWeather", json!({ "city": "SF" })),
        ]),
        tool_msg(vec![tool_result(
            "call-1",
            json!({ "type": "text", "value": "Sunny, 72F" }),
        )]),
        assistant(vec![
            reasoning("The weather is sunny and warm.\n", Some("sig-def456")),
            ContentPart::text("It is sunny and 72F in SF."),
        ]),
    ]);
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
        user(vec![ContentPart::text("Explain your reasoning")]),
        assistant(vec![
            ContentPart::text("My answer is 42."),
            reasoning(
                "I calculated this by analyzing the meaning of life",
                Some("reasoning-process"),
            ),
        ]),
    ]);
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
        user(vec![ContentPart::text("Hello")]),
        assistant(vec![
            ContentPart::text("\n\n"),
            tool_call("call-123", "test", json!({})),
            ContentPart::text("  "),
            ContentPart::text("actual content"),
        ]),
    ]);
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
    )])]);
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
    ])]);
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
        user(vec![ContentPart::text("Hello")]),
        assistant(vec![
            reasoning("thinking...", Some("sig-1")),
            ContentPart::text(""),
            reasoning("more thinking...", Some("sig-2")),
            ContentPart::text("response text"),
            tool_call("call-123", "test", json!({})),
        ]),
    ]);
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
        json!({ "type": "content", "value": [{ "type": "text", "text": "The result is 42" }] }),
    )])]);
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
        json!({ "type": "content", "value": [{
            "type": "file",
            "data": { "type": "data", "data": "base64data" },
            "mediaType": "image/jpeg"
        }] }),
    )])]);
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
        json!({ "type": "content", "value": [{
            "type": "file",
            "data": { "type": "data", "data": "base64data" },
            "mediaType": "application/pdf",
            "filename": "tool-result.pdf"
        }] }),
    )])]);
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

// SKIPPED (TS: "should throw error for unsupported image format in tool result
// content" and "should throw error for unsupported mime type in tool result
// file content"): convert_prompt_to_bedrock returns no Result; unknown mimes
// fall back to a default format instead of throwing.

/// TS: "should fallback to stringified result when content is undefined" (json output)
#[test]
fn tool_result_json_output() {
    let (_, messages) = convert_prompt_to_bedrock(&vec![tool_msg(vec![tool_result(
        "call-123",
        json!({ "type": "json", "value": { "value": 42 } }),
    )])]);
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
        Some(json!({ "bedrock": { "citations": { "enabled": true } } })),
    )])]);
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
        Some(json!({ "bedrock": { "citations": { "enabled": false } } })),
    )])]);
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
    )])]);
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
            Some(json!({ "bedrock": { "citations": { "enabled": true } } })),
        ),
        file_base64(
            "BAUGBw==",
            "application/pdf",
            None,
            Some(json!({ "bedrock": { "citations": { "enabled": false } } })),
        ),
    ])]);
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
    )])]);
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
    )])]);
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
        json!({ "type": "text", "value": "The result is 42" }),
    )])]);
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
        json!({ "type": "text", "value": "The result is 42" }),
    )])]);
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
    )])]);
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
    )])]);
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

fn func_tool_strict(
    name: &str,
    description: Option<&str>,
    input_schema: Value,
    strict: Option<bool>,
) -> FunctionTool {
    FunctionTool {
        name: name.to_string(),
        description: description.map(std::string::ToString::to_string),
        input_schema,
        strict,
        provider_options: None,
        input_examples: None,
    }
}

/// TS: "should return empty toolConfig when tools are undefined"
#[test]
fn prepare_tools_empty_when_undefined() {
    let config = prepare_tools(&None, &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(config, json!({}));
}

/// TS: "should return empty toolConfig when tools are empty"
#[test]
fn prepare_tools_empty_when_empty() {
    let config = prepare_tools(&Some(vec![]), &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(config, json!({}));
}

/// TS: "should correctly prepare function tools"
#[test]
fn prepare_tools_function_tools() {
    let tools = Some(vec![func_tool(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(
        config["tools"],
        json!([{
            "toolSpec": {
                "name": "testFunction",
                "description": "A test function",
                "inputSchema": { "json": { "type": "object", "properties": {} } }
            }
        }])
    );
}

/// TS: "should exclude description when it is empty string"
#[test]
fn prepare_tools_exclude_description_empty() {
    let tools = Some(vec![func_tool(
        "testFunction",
        Some(""),
        json!({ "type": "object", "properties": {} }),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    let spec = &config["tools"][0]["toolSpec"];
    assert!(spec.get("description").is_none());
}

/// TS: "should exclude description when it is whitespace-only"
#[test]
fn prepare_tools_exclude_description_whitespace() {
    let tools = Some(vec![func_tool(
        "testFunction",
        Some("   "),
        json!({ "type": "object", "properties": {} }),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    let spec = &config["tools"][0]["toolSpec"];
    assert!(spec.get("description").is_none());
}

/// TS: "should include description when it has content"
#[test]
fn prepare_tools_include_description() {
    let tools = Some(vec![func_tool(
        "testFunction",
        Some("Valid description"),
        json!({ "type": "object", "properties": {} }),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(
        config["tools"][0]["toolSpec"]["description"],
        json!("Valid description")
    );
}

// SKIPPED (TS: "should warn for provider-defined tools on non-anthropic models",
// "should warn and filter out web_search_20250305 tool", "should return empty
// toolConfig when all tools are filtered out"): the Rust FunctionTool has no
// type/id, so provider-defined tools are not modelled.

/// TS: "should handle tool choice 'auto'"
#[test]
fn prepare_tools_tool_choice_auto() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, NON_ANTHROPIC_MODEL);
    assert_eq!(config["toolChoice"], json!({ "auto": {} }));
}

/// TS: "should handle tool choice 'required'"
#[test]
fn prepare_tools_tool_choice_required() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, &ToolChoice::Required, NON_ANTHROPIC_MODEL);
    assert_eq!(config["toolChoice"], json!({ "any": {} }));
}

/// TS: "should handle tool choice 'none' by clearing tools"
#[test]
fn prepare_tools_tool_choice_none_clears() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(&tools, &ToolChoice::None, NON_ANTHROPIC_MODEL);
    assert_eq!(config, json!({}));
}

/// TS: "should handle tool choice 'tool'"
#[test]
fn prepare_tools_tool_choice_tool() {
    let tools = Some(vec![func_tool("testFunction", Some("Test"), json!({}))]);
    let config = prepare_tools(
        &tools,
        &ToolChoice::Tool {
            tool_name: "testFunction".to_string(),
        },
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
        &ToolChoice::Tool {
            tool_name: "getWeather".to_string(),
        },
        NON_ANTHROPIC_MODEL,
    );
    assert_eq!(config["tools"].as_array().unwrap().len(), 1);
    assert_eq!(config["tools"][0]["toolSpec"]["name"], json!("getWeather"));
}

/// TS: "should pass through strict mode when strict is true"
#[test]
fn prepare_tools_strict_true() {
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(
        config["tools"],
        json!([{
            "toolSpec": {
                "name": "testFunction",
                "description": "A test function",
                "strict": true,
                "inputSchema": { "json": { "type": "object", "properties": {} } }
            }
        }])
    );
}

/// TS: "should pass through strict mode when strict is false"
#[test]
fn prepare_tools_strict_false() {
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(false),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    assert_eq!(
        config["tools"],
        json!([{
            "toolSpec": {
                "name": "testFunction",
                "description": "A test function",
                "strict": false,
                "inputSchema": { "json": { "type": "object", "properties": {} } }
            }
        }])
    );
}

/// TS: "should not include strict when strict is undefined"
#[test]
fn prepare_tools_strict_undefined() {
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        None,
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    let spec = &config["tools"][0]["toolSpec"];
    assert!(spec.get("strict").is_none());
}

/// TS: "should pass through strict mode for multiple tools with different strict settings"
#[test]
fn prepare_tools_strict_multiple() {
    let tools = Some(vec![
        func_tool_strict(
            "strictTool",
            Some("A strict tool"),
            json!({ "type": "object", "properties": {} }),
            Some(true),
        ),
        func_tool_strict(
            "nonStrictTool",
            Some("A non-strict tool"),
            json!({ "type": "object", "properties": {} }),
            Some(false),
        ),
        func_tool_strict(
            "defaultTool",
            Some("A tool without strict setting"),
            json!({ "type": "object", "properties": {} }),
            None,
        ),
    ]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, ANTHROPIC_MODEL);
    let tools_arr = config["tools"].as_array().unwrap();
    assert_eq!(tools_arr.len(), 3);
    assert_eq!(tools_arr[0]["toolSpec"]["strict"], json!(true));
    assert_eq!(tools_arr[1]["toolSpec"]["strict"], json!(false));
    assert!(tools_arr[2]["toolSpec"].get("strict").is_none());
}

/// TS: it.each "should omit strict for %s" (claude-opus-5, claude-sonnet-5,
/// claude-fable-5). These models reject newer schema fields, so `strict` is
/// omitted even when set.
#[test]
fn prepare_tools_omit_strict_for_opus_5() {
    assert!(!supports_strict_tools("us.anthropic.claude-opus-5"));
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, "us.anthropic.claude-opus-5");
    assert!(config["tools"][0]["toolSpec"].get("strict").is_none());
}

#[test]
fn prepare_tools_omit_strict_for_sonnet_5() {
    assert!(!supports_strict_tools("anthropic.claude-sonnet-5"));
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, "anthropic.claude-sonnet-5");
    assert!(config["tools"][0]["toolSpec"].get("strict").is_none());
}

#[test]
fn prepare_tools_omit_strict_for_fable_5() {
    assert!(!supports_strict_tools("eu.anthropic.claude-fable-5"));
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, "eu.anthropic.claude-fable-5");
    assert!(config["tools"][0]["toolSpec"].get("strict").is_none());
}

/// TS: "should omit strict for claude-opus-4-7"
#[test]
fn prepare_tools_omit_strict_for_opus_4_7() {
    assert!(!supports_strict_tools("us.anthropic.claude-opus-4-7"));
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, "us.anthropic.claude-opus-4-7");
    assert!(config["tools"][0]["toolSpec"].get("strict").is_none());
}

/// TS: "should omit strict for claude-opus-4-8"
#[test]
fn prepare_tools_omit_strict_for_opus_4_8() {
    assert!(!supports_strict_tools("anthropic.claude-opus-4-8"));
    let tools = Some(vec![func_tool_strict(
        "testFunction",
        Some("A test function"),
        json!({ "type": "object", "properties": {} }),
        Some(true),
    )]);
    let config = prepare_tools(&tools, &ToolChoice::Auto, "anthropic.claude-opus-4-8");
    assert!(config["tools"][0]["toolSpec"].get("strict").is_none());
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
