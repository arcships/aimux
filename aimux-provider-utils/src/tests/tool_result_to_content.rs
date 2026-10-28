//! Tests of `tool_result_to_content`. Upstream has no shared function or test
//! file for it: each chat converter repeats the same `switch`, so these cover
//! the cases that `switch` handles (`convert-to-openai-chat-messages.ts`).

use aimux_core::language_model_message::{FilePart, TextPart, ToolResultContent, ToolResultOutput};
use aimux_core::shared::{FileBytes, FileData};
use serde_json::{Value, json};

use crate::{tool_result_content_value, tool_result_to_content};

fn output(value: Value) -> ToolResultOutput {
    serde_json::from_value(value).expect("valid tool result output")
}

// Not an upstream case: `text` and `error-text` send the value as is.
#[test]
fn sends_text_outputs_as_is() {
    assert_eq!(
        tool_result_to_content(&output(json!({ "type": "text", "value": "sunny" }))),
        json!("sunny")
    );
    assert_eq!(
        tool_result_to_content(&output(json!({ "type": "error-text", "value": "failed" }))),
        json!("failed")
    );
}

// Not an upstream case: `execution-denied` sends the reason or a fixed text.
#[test]
fn sends_the_denial_reason_or_the_default_text() {
    assert_eq!(
        tool_result_to_content(&output(
            json!({ "type": "execution-denied", "reason": "not allowed" })
        )),
        json!("not allowed")
    );
    assert_eq!(
        tool_result_to_content(&output(json!({ "type": "execution-denied" }))),
        json!("Tool call execution denied.")
    );
}

// Not an upstream case: `json` and `error-json` send `JSON.stringify(value)`.
#[test]
fn stringifies_json_outputs() {
    assert_eq!(
        tool_result_to_content(&output(
            json!({ "type": "json", "value": { "temperature": 21 } })
        )),
        json!(r#"{"temperature":21}"#)
    );
    assert_eq!(
        tool_result_to_content(&output(
            json!({ "type": "error-json", "value": { "error": "timeout" } })
        )),
        json!(r#"{"error":"timeout"}"#)
    );
}

// Not an upstream case: `content` sends `JSON.stringify(value)` of the parts,
// with camelCase keys and a `Uint8Array` as an object keyed by byte index.
#[test]
fn stringifies_content_parts_as_upstream_serializes_them() {
    let content = vec![
        ToolResultContent::Text(TextPart {
            text: "see image".into(),
            provider_options: None,
        }),
        ToolResultContent::File(FilePart {
            data: FileData::Data {
                data: FileBytes::Binary(vec![1, 2]),
            },
            media_type: "image/png".into(),
            filename: Some("a.png".into()),
            provider_options: None,
        }),
        ToolResultContent::File(FilePart {
            data: FileData::Url {
                url: "https://example.com/b.png".into(),
                original_url: Some("https://example.com/original.png".into()),
            },
            media_type: "image/png".into(),
            filename: None,
            provider_options: None,
        }),
    ];

    let expected = json!([
        { "type": "text", "text": "see image" },
        {
            "type": "file",
            "data": { "type": "data", "data": { "0": 1, "1": 2 } },
            "mediaType": "image/png",
            "filename": "a.png"
        },
        {
            "type": "file",
            "data": {
                "type": "url",
                "url": "https://example.com/b.png",
                "originalUrl": "https://example.com/original.png"
            },
            "mediaType": "image/png"
        }
    ]);
    assert_eq!(tool_result_content_value(&content), expected);
    assert_eq!(
        tool_result_to_content(&ToolResultOutput::Content { value: content }),
        json!(expected.to_string())
    );
}
