//! The wiremock harness of the older Hugging Face Responses ports: every test
//! builds the model through `create_huggingface(..).responses(model)` against a
//! loopback `MockServer`.

pub(super) use aimux_core::tool::ToolResult;
pub(super) use aimux_core::tool::{FunctionTool, RawToolCall};

pub(super) use futures::StreamExt;
pub(super) use serde_json::{Value, json};
pub(super) use wiremock::matchers::{method, path};
pub(super) use wiremock::{Mock, MockServer, ResponseTemplate};

pub(super) use aimux_core::error::AiMuxError;
pub(super) use aimux_core::generate::{GenerateTextOptions, generate_text};
pub(super) use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelPrompt, ReasoningPart, TextPart, ToolCallPart, UserPart,
};
pub(super) use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, Tool};
pub(super) use aimux_core::result::{GenerateContent, ReasoningOutput, Source};
pub(super) use aimux_core::shared::{FileBytes, FileData};
pub(super) use aimux_core::stream_part::StreamPart;
pub(super) use aimux_core::types::{FinishReasonUnified, ResponseMetadata};

pub(super) use crate::huggingface::responses::convert_to_huggingface_responses_messages;
pub(super) use crate::{HuggingFaceProvider, HuggingFaceProviderSettings, create_huggingface};

// ─────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

// `test_prompt` (the TS `TEST_PROMPT`) is the one in responses_language_model.rs.

/// `CallOptions` with only `prompt` set (everything else default/None).
pub(super) fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build a Hugging Face provider whose base URL points at the mock server.
pub(super) fn make_provider(server: &MockServer) -> HuggingFaceProvider {
    create_huggingface(HuggingFaceProviderSettings {
        api_key: Some("APIKEY".to_string().into()),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .expect("valid settings")
}

/// Mount a JSON response on `/responses`.
pub(super) async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on `/responses`.
pub(super) async fn mock_sse(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

/// Build a single SSE `data: <json>\n\n` event string.
pub(super) fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events (no `[DONE]` sentinel — HF Responses SSE has none).
pub(super) fn sse_body(events: &[String]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
pub(super) async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
    let mut parts = Vec::new();
    let mut stream = result.stream;
    while let Some(part) = stream.next().await {
        match part {
            Ok(p) => parts.push(p),
            Err(e) => panic!("stream error: {e:?}"),
        }
    }
    parts
}

/// A minimal completed response body with a single message output.
pub(super) fn basic_response_body() -> Value {
    json!({
        "id": "resp_67c97c0203188190a025beb4a75242bc",
        "model": "deepseek-ai/DeepSeek-V3-0324",
        "object": "response",
        "created_at": 1741257730,
        "status": "completed",
        "error": null,
        "instructions": null,
        "max_output_tokens": null,
        "metadata": null,
        "tool_choice": "auto",
        "tools": [],
        "temperature": 1.0,
        "top_p": 1.0,
        "incomplete_details": null,
        "usage": {
            "input_tokens": 12,
            "output_tokens": 25,
            "total_tokens": 37
        },
        "output": [
            {
                "id": "msg_67c97c02656c81908e080dfdf4a03cd1",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Hello! How can I help you today?"
                    }
                ]
            }
        ],
        "output_text": "Hello! How can I help you today?"
    })
}

/// Helper: mount a stub response so do_generate succeeds and we can inspect
/// the request body.
pub(super) async fn stub_empty_response(server: &MockServer) {
    mock_json(
        server,
        json!({
            "id": "resp_test",
            "model": "moonshotai/Kimi-K2-Instruct",
            "object": "response",
            "created_at": 1741257730,
            "status": "completed",
            "error": null,
            "instructions": null,
            "max_output_tokens": null,
            "metadata": null,
            "tool_choice": "auto",
            "tools": [],
            "temperature": 1.0,
            "top_p": 1.0,
            "incomplete_details": null,
            "usage": null,
            "output": [],
            "output_text": "Test response"
        }),
    )
    .await;
}

pub(super) const PNG_BASE64: &str = "iVBORw0KGgo=";
