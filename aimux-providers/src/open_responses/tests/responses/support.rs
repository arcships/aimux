//! The harness of the older end-to-end ports (previously
//! `tests/open_responses_test.rs`): a wiremock server per test.

use crate::open_responses::{
    OpenResponsesModel, OpenResponsesProviderSettings, create_open_responses,
};
use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::stream_part::StreamPart;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

// ============================================================================
// Shared helpers
// ============================================================================

/// The TS `TEST_PROMPT`: a single user text message "Hello".
pub(super) fn wiremock_test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

/// `CallOptions` with only `prompt` set (everything else default/None).
pub(super) fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
    CallOptions::new(prompt)
}

/// Build Open Responses settings whose base URL points at the mock server.
pub(super) fn make_settings(server: &MockServer) -> OpenResponsesProviderSettings {
    OpenResponsesProviderSettings::new("lmstudio", server.uri())
}

/// Create a model pointing at the mock server.
pub(super) fn make_model(server: &MockServer, model_id: &str) -> OpenResponsesModel {
    let provider = create_open_responses(make_settings(server)).expect("valid settings");
    provider.responses(model_id)
}

/// Mount a JSON response on any path.
pub(super) async fn mock_json(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount an SSE stream response on any path.
pub(super) async fn mock_sse(server: &MockServer, body: String) {
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
pub(super) fn sse_event(json_str: &str) -> String {
    format!("data: {json_str}\n\n")
}

/// Concatenate SSE events and append the `[DONE]` sentinel.
pub(super) fn sse_body(events: &[&str]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(event);
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Collect every `StreamPart` from a `StreamResult` into a `Vec`.
pub(super) async fn collect_stream(result: aimux_core::result::StreamResult) -> Vec<StreamPart> {
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
pub(super) fn lmstudio_basic_json() -> Value {
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
pub(super) fn lmstudio_tool_call_json() -> Value {
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
pub(super) fn openai_pdf_json() -> Value {
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
