// Panic convert wrappers are #[deprecated]; these tests still use them.
#![allow(deprecated)]
//! Rust port of the remaining `convert-to-anthropic-prompt.test.ts` cases that
//! exercise features now supported by the Rust data model: mid-conversation
//! system messages, trailing-whitespace trimming, reasoning/thinking parts, URL
//! & base64 file parts, PDF beta, top-level media-type detection, and
//! provider-referenced files.
//!
//! Cases that remain out of scope (they need richer per-part provider options or
//! server-tool content parts not present in the Rust model) are documented in
//! the task summary: cache_control, citations, mid-conversation tool-changes,
//! server tools (web_search/web_fetch/code_execution/mcp/advisor), and
//! tool-result `content` outputs whose inner parts need conversion.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPromptMessage;
use aimux_core::message::Role;
use aimux_core::types::Warning;
use aimux_providers::anthropic::convert::convert_prompt_to_anthropic_full;
use serde_json::{Value, json};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn msg(role: Role, parts: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role,
        content: parts,
        provider_options: None,
    }
}

fn text_part(t: &str) -> ContentPart {
    ContentPart::text(t)
}

fn prompt(msgs: Vec<LanguageModelPromptMessage>) -> Vec<LanguageModelPromptMessage> {
    msgs
}

fn convert_full(
    msgs: Vec<LanguageModelPromptMessage>,
    send_reasoning: bool,
) -> aimux_providers::anthropic::convert::AnthropicPromptConversion {
    convert_prompt_to_anthropic_full(&msgs, send_reasoning)
}

fn assert_other_warning(warnings: &[Warning], message: &str) {
    let found = warnings.iter().any(|w| match w {
        Warning::Other { message: m } => m == message,
        _ => false,
    });
    assert!(
        found,
        "expected a Warning::Other with message {message:?}, got {warnings:?}"
    );
}

fn reasoning_part(text: &str, signature: Option<&str>) -> ContentPart {
    ContentPart::Reasoning {
        text: text.to_string(),
        signature: signature.map(std::string::ToString::to_string),
        provider_options: None,
    }
}

fn file_url(url: &str, media_type: &str) -> ContentPart {
    ContentPart::file_url(url.to_string(), media_type.to_string())
}

fn file_base64(data: &str, media_type: &str) -> ContentPart {
    ContentPart::file_base64(data.to_string(), media_type.to_string())
}

fn file_bytes(data: &[u8], media_type: &str) -> ContentPart {
    ContentPart::file(data.to_vec(), media_type.to_string())
}

fn file_bytes_named(data: &[u8], media_type: &str, filename: &str) -> ContentPart {
    ContentPart::File {
        data: data.to_vec(),
        media_type: media_type.to_string(),
        filename: Some(filename.to_string()),
        provider_options: None,
    }
}

fn file_ref(media_type: &str, reference: Value) -> ContentPart {
    ContentPart::file_reference(media_type.to_string(), reference)
}

// PNG magic bytes and their base64 encoding (matches the TS `pngBase64`).
const PNG_BYTES: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const PNG_BASE64: &str = "iVBORw0KGgo=";
// PDF magic bytes and their base64 encoding (matches the TS `pdfBase64`).
const PDF_BYTES: [u8; 5] = [0x25, 0x50, 0x44, 0x46, 0x2d];
const PDF_BASE64: &str = "JVBERi0=";

// ---------------------------------------------------------------------------
// mid-conversation system
// ---------------------------------------------------------------------------

#[test]
fn should_emit_a_mid_conversation_system_message_inline_and_add_the_beta() {
    let p = prompt(vec![
        msg(Role::System, vec![text_part("initial")]),
        msg(Role::User, vec![text_part("hi")]),
        msg(Role::Assistant, vec![text_part("hello")]),
        msg(Role::System, vec![text_part("switch tone")]),
        msg(Role::User, vec![text_part("go")]),
    ]);
    let result = convert_full(p, true);
    assert_eq!(
        result.system,
        Some(vec![json!({ "type": "text", "text": "initial" })])
    );
    assert!(
        result.messages.iter().any(|m| m
            == &json!({
                "role": "system",
                "content": [{ "type": "text", "text": "switch tone" }]
            })),
        "expected an inline system message, got {:?}",
        result.messages
    );
    assert!(result.betas.contains("mid-conversation-system-2026-04-07"));
}

// ---------------------------------------------------------------------------
// trailing whitespace
// ---------------------------------------------------------------------------

#[test]
fn should_remove_trailing_whitespace_from_last_assistant_message_when_no_further_user_message() {
    let p = prompt(vec![
        msg(Role::User, vec![text_part("user content")]),
        msg(Role::Assistant, vec![text_part("assistant content  ")]),
    ]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "user content" }] }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "assistant content" }] }),
        ]
    );
}

#[test]
fn should_remove_trailing_whitespace_from_last_assistant_message_with_multi_part_content() {
    let p = prompt(vec![
        msg(Role::User, vec![text_part("user content")]),
        msg(
            Role::Assistant,
            vec![text_part("assistant "), text_part("content  ")],
        ),
    ]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "user content" }] }),
            json!({
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "assistant " },
                    { "type": "text", "text": "content" },
                ]
            }),
        ]
    );
}

#[test]
fn should_keep_trailing_whitespace_from_assistant_message_when_there_is_a_further_user_message() {
    let p = prompt(vec![
        msg(Role::User, vec![text_part("user content")]),
        msg(Role::Assistant, vec![text_part("assistant content  ")]),
        msg(Role::User, vec![text_part("user content 2")]),
    ]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "user content" }] }),
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "assistant content  " }] }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "user content 2" }] }),
        ]
    );
}

// ---------------------------------------------------------------------------
// reasoning / thinking
// ---------------------------------------------------------------------------

#[test]
fn should_convert_reasoning_parts_with_signature_into_thinking_parts_when_send_reasoning_is_true() {
    let p = prompt(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part(
                "I need to count the number of \"r\"s in the word \"strawberry\".",
                Some("test-signature"),
            ),
            text_part("The word \"strawberry\" has 2 \"r\"s."),
        ],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "assistant",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "I need to count the number of \"r\"s in the word \"strawberry\".",
                    "signature": "test-signature",
                },
                { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
            ]
        })]
    );
    assert!(result.warnings.is_empty());
}

#[test]
fn should_ignore_reasoning_parts_without_signature_when_send_reasoning_is_true() {
    let p = prompt(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part(
                "I need to count the number of \"r\"s in the word \"strawberry\".",
                None,
            ),
            text_part("The word \"strawberry\" has 2 \"r\"s."),
        ],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
            ]
        })]
    );
    assert_eq!(result.warnings.len(), 1);
    assert_other_warning(&result.warnings, "unsupported reasoning metadata");
}

#[test]
fn should_omit_reasoning_parts_with_signature_when_send_reasoning_is_false() {
    let p = prompt(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part(
                "I need to count the number of \"r\"s in the word \"strawberry\".",
                Some("test-signature"),
            ),
            text_part("The word \"strawberry\" has 2 \"r\"s."),
        ],
    )]);
    let result = convert_full(p, false);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
            ]
        })]
    );
    assert_eq!(result.warnings.len(), 1);
    assert_other_warning(
        &result.warnings,
        "sending reasoning content is disabled for this model",
    );
}

#[test]
fn should_omit_reasoning_parts_without_signature_when_send_reasoning_is_false() {
    let p = prompt(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part(
                "I need to count the number of \"r\"s in the word \"strawberry\".",
                None,
            ),
            text_part("The word \"strawberry\" has 2 \"r\"s."),
        ],
    )]);
    let result = convert_full(p, false);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "The word \"strawberry\" has 2 \"r\"s." },
            ]
        })]
    );
    assert_eq!(result.warnings.len(), 1);
    assert_other_warning(
        &result.warnings,
        "sending reasoning content is disabled for this model",
    );
}

// ---------------------------------------------------------------------------
// Extended-thinking input echo (build_request_body_with_warnings wiring)
// ---------------------------------------------------------------------------

#[test]
fn thinking_enabled_echoes_reasoning_parts_as_thinking_blocks() {
    use aimux_core::options::CallOptions;
    use aimux_providers::anthropic::convert::build_request_body_with_warnings;

    let mut options = CallOptions::new(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part("Let me weigh the options.", Some("sig-echo-1")),
            text_part("Here is the answer."),
        ],
    )]);
    options.provider_options = Some(
        [(
            "anthropic".to_string(),
            json!({ "thinking": { "type": "enabled", "budgetTokens": 1024 } }),
        )]
        .into_iter()
        .collect(),
    );
    let result =
        build_request_body_with_warnings("claude-sonnet-4-20250514", &options, false).unwrap();
    let blocks = result.body["messages"][0]["content"].as_array().unwrap();
    let thinking = blocks
        .iter()
        .find(|b| b["type"] == "thinking")
        .expect("reasoning part must be echoed as a thinking block");
    assert_eq!(thinking["signature"], "sig-echo-1");
    assert_eq!(thinking["thinking"], "Let me weigh the options.");
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Other { message } if message.contains("disabled"))),
        "no 'disabled' warning when thinking is enabled: {:?}",
        result.warnings
    );
}

#[test]
fn thinking_disabled_omits_reasoning_parts() {
    use aimux_core::options::CallOptions;
    use aimux_providers::anthropic::convert::build_request_body_with_warnings;

    let mut options = CallOptions::new(vec![msg(
        Role::Assistant,
        vec![
            reasoning_part("Let me weigh the options.", Some("sig-echo-1")),
            text_part("Here is the answer."),
        ],
    )]);
    options.provider_options = Some(
        [(
            "anthropic".to_string(),
            json!({ "thinking": { "type": "disabled" } }),
        )]
        .into_iter()
        .collect(),
    );
    let result =
        build_request_body_with_warnings("claude-sonnet-4-20250514", &options, false).unwrap();
    let blocks = result.body["messages"][0]["content"].as_array().unwrap();
    assert!(
        !blocks.iter().any(|b| b["type"] == "thinking"),
        "thinking block must be omitted when thinking is disabled"
    );
}

// ---------------------------------------------------------------------------
// URL images
// ---------------------------------------------------------------------------

#[test]
fn should_add_image_parts_for_url_images() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_url("https://example.com/image.png", "image/*")],
    )]);
    let result = convert_full(p, true);
    assert!(result.system.is_none());
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [{
                "type": "image",
                "source": { "type": "url", "url": "https://example.com/image.png" },
            }],
        })]
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_treat_url_strings_in_image_file_data_as_urls_not_base64() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_url("https://example.com/image.png", "image/png")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [{
                "type": "image",
                "source": { "type": "url", "url": "https://example.com/image.png" },
            }],
        })]
    );
}

#[test]
fn passes_through_url_for_top_level_only_image() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_url("https://example.com/x.png", "image")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "image",
            "source": { "type": "url", "url": "https://example.com/x.png" },
        })
    );
}

// ---------------------------------------------------------------------------
// URL PDFs
// ---------------------------------------------------------------------------

#[test]
fn should_treat_url_strings_in_pdf_file_data_as_urls_not_base64() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_url(
            "https://example.com/document.pdf",
            "application/pdf",
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [{
                "type": "document",
                "source": { "type": "url", "url": "https://example.com/document.pdf" },
            }],
        })]
    );
    let mut expected_betas = BTreeSet::new();
    expected_betas.insert("pdfs-2024-09-25".to_string());
    assert_eq!(result.betas, expected_betas);
}

#[test]
fn should_add_pdf_file_parts_for_url_pdfs() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_url(
            "https://example.com/document.pdf",
            "application/pdf",
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "document",
            "source": { "type": "url", "url": "https://example.com/document.pdf" },
        })
    );
    assert!(result.betas.contains("pdfs-2024-09-25"));
}

// ---------------------------------------------------------------------------
// base64 PDFs
// ---------------------------------------------------------------------------

#[test]
fn should_add_pdf_file_parts_for_base64_pdfs() {
    // The TS test uses the opaque placeholder string 'base64PDFdata'; the Rust
    // `FileBase64` variant holds the base64 string verbatim (it is NOT
    // decoded/re-encoded), so the same placeholder round-trips exactly.
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64("base64PDFdata", "application/pdf")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [{
                "type": "document",
                "source": {
                    "type": "base64",
                    "media_type": "application/pdf",
                    "data": "base64PDFdata",
                },
            }],
        })]
    );
    assert!(result.betas.contains("pdfs-2024-09-25"));
}

// ---------------------------------------------------------------------------
// text/plain documents (filename -> title)
// ---------------------------------------------------------------------------

#[test]
fn should_add_text_file_parts_for_text_plain_documents() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_bytes_named(
            b"sample text content",
            "text/plain",
            "sample.txt",
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "document",
            "source": { "type": "text", "media_type": "text/plain", "data": "sample text content" },
            "title": "sample.txt",
        })
    );
    assert!(result.betas.is_empty());
}

#[test]
fn should_map_inline_text_file_parts_to_inline_text_document_source() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_bytes_named(
            b"inline text content",
            "text/plain",
            "inline.txt",
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "document",
            "source": { "type": "text", "media_type": "text/plain", "data": "inline text content" },
            "title": "inline.txt",
        })
    );
}

#[test]
fn still_routes_text_plain_inline_text_through_document_source() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_bytes(b"hello", "text/plain")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "document",
            "source": { "type": "text", "media_type": "text/plain", "data": "hello" },
        })
    );
}

// ---------------------------------------------------------------------------
// unsupported file types
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "media type: video/mp4")]
fn should_throw_error_for_unsupported_file_types() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64("base64data", "video/mp4")],
    )]);
    let _ = convert_full(p, true);
}

// ---------------------------------------------------------------------------
// top-level-only media type resolution (byte sniffing)
// ---------------------------------------------------------------------------

#[test]
fn detects_image_subtype_from_inline_bytes_for_top_level_image() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64(PNG_BASE64, "image")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": PNG_BASE64 },
        })
    );
}

#[test]
fn normalizes_image_star_via_detection() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64(PNG_BASE64, "image/*")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0]["source"]["media_type"],
        json!("image/png")
    );
}

#[test]
fn detects_pdf_subtype_from_inline_bytes_for_top_level_application() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64(PDF_BASE64, "application")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0]["source"]["media_type"],
        json!("application/pdf")
    );
    assert!(result.betas.contains("pdfs-2024-09-25"));
}

#[test]
fn preserves_full_image_png_pass_through() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_base64(PNG_BASE64, "image/png")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": PNG_BASE64 },
        })
    );
}

#[test]
fn detects_image_subtype_from_inline_bytes_for_top_level_image_via_file_bytes() {
    // Same as above but through the raw-bytes `File` variant, which re-encodes
    // the bytes to base64 (rather than passing a string through verbatim).
    let p = prompt(vec![msg(Role::User, vec![file_bytes(&PNG_BYTES, "image")])]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": PNG_BASE64 },
        })
    );
}

#[test]
fn detects_pdf_subtype_from_inline_bytes_via_file_bytes() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_bytes(&PDF_BYTES, "application")],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0]["source"]["media_type"],
        json!("application/pdf")
    );
    assert!(result.betas.contains("pdfs-2024-09-25"));
}

// ---------------------------------------------------------------------------
// provider-referenced files
// ---------------------------------------------------------------------------

#[test]
fn should_convert_messages_with_image_file_parts_using_provider_reference() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_ref(
            "image/png",
            json!({ "anthropic": "file-img-12345" }),
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [{
                "type": "image",
                "source": { "type": "file", "file_id": "file-img-12345" },
            }],
        })]
    );
    assert!(result.betas.contains("files-api-2025-04-14"));
}

#[test]
fn should_convert_messages_with_pdf_file_parts_using_provider_reference() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_ref(
            "application/pdf",
            json!({ "anthropic": "file-pdf-12345" }),
        )],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages[0]["content"][0],
        json!({
            "type": "document",
            "source": { "type": "file", "file_id": "file-pdf-12345" },
        })
    );
    assert!(result.betas.contains("files-api-2025-04-14"));
}

#[test]
fn should_convert_provider_referenced_file_parts_to_container_uploads_when_requested() {
    let part = ContentPart::FileReference {
        media_type: "text/csv".to_string(),
        reference: json!({ "anthropic": "file-csv-12345" }),
        filename: None,
        provider_options: Some(json!({ "anthropic": { "containerUpload": true } })),
    };
    let p = prompt(vec![msg(
        Role::User,
        vec![text_part("Analyze this data."), part],
    )]);
    let result = convert_full(p, true);
    assert_eq!(
        result.messages,
        vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "Analyze this data." },
                { "type": "container_upload", "file_id": "file-csv-12345" },
            ],
        })]
    );
    assert!(result.betas.contains("files-api-2025-04-14"));
}

#[test]
#[should_panic(expected = "No provider reference found for provider 'anthropic'")]
fn should_throw_when_provider_reference_does_not_contain_anthropic_key() {
    let p = prompt(vec![msg(
        Role::User,
        vec![file_ref("application/pdf", json!({ "openai": "file-xyz" }))],
    )]);
    let _ = convert_full(p, true);
}

// Keep the `Value` import used even if some tests only use `json!`.
#[allow(dead_code)]
fn _ensure_value_import_used(_v: Value) {}
