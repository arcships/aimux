//! Small Responses API subset from the ported xAI upstream cases.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::GenerateContent;
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{ProviderTool, Tool};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::Resolvable;
use aimux_providers::xai::{XAIProviderSettings, XaiResponsesModel, create_xai};
use futures::TryStreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn model(fetch: &std::sync::Arc<MockFetch>) -> XaiResponsesModel {
    create_xai(XAIProviderSettings {
        base_url: Some("https://example.test/v1/".into()),
        api_key: Some(Resolvable::Value("test-key".into())),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
    .responses("grok-4-fast-non-reasoning")
}

fn options() -> CallOptions {
    CallOptions {
        prompt: vec![LanguageModelMessage::user_text("hello")],
        ..Default::default()
    }
}

fn assert_request(fetch: &MockFetch, stream: bool, tools: bool) {
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "https://example.test/v1/responses");
    assert_eq!(request.headers["authorization"], "Bearer test-key");
    let mut expected = json!({
        "model": "grok-4-fast-non-reasoning",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": "hello"}]}]
    });
    if stream {
        expected["stream"] = json!(true);
    }
    if tools {
        expected["tools"] = json!([{"type": "web_search"}]);
        expected["tool_choice"] = json!("auto");
    }
    assert_eq!(request.json_body(), expected);
}

fn sse(events: &[Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: (events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            + "data: [DONE]\n\n")
            .into_bytes(),
    }
}

/// TS: "should extract usage correctly" (xai/src/responses/xai-responses-language-model.test.ts)
#[tokio::test]
async fn generate_converts_usage() {
    // The port used an unrelated search fixture for this case; restore the TS response.
    let usage = json!({
        "input_tokens": 345, "output_tokens": 538, "total_tokens": 883,
        "output_tokens_details": {"reasoning_tokens": 123}
    });
    let body = json!({
        "id": "resp_123", "object": "response", "status": "completed",
        "model": "grok-4-fast-non-reasoning", "output": [], "usage": usage
    });
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
    let result = model(&fetch).do_generate(&options()).await.unwrap();
    assert_request(&fetch, false, false);
    assert!(result.content.is_empty());
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("completed"));
    assert_eq!(
        serde_json::to_value(result.usage).unwrap(),
        json!({
            "input_tokens": {"total": 345, "no_cache": 345, "cache_read": 0},
            "output_tokens": {"total": 538, "text": 415, "reasoning": 123},
            "raw": usage
        })
    );
    assert!(result.provider_metadata.is_none());
    assert_eq!(result.response.unwrap().id.as_deref(), Some("resp_123"));
}

/// TS: "should expose cost_in_usd_ticks in finish providerMetadata" (xai/src/responses/xai-responses-language-model.test.ts)
#[tokio::test]
async fn stream_reports_cost_and_text() {
    let events = vec![
        json!({"type": "response.created", "response": {"id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": []}}),
        json!({"type": "response.output_text.delta", "output_index": 0, "content_index": 0, "delta": "Hello"}),
        json!({"type": "response.completed", "response": {"id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "status": "completed", "output": [{"type": "message", "id": "msg_001", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": "Hello"}]}], "usage": {"input_tokens": 10, "output_tokens": 5, "cost_in_usd_ticks": 113500}}}),
    ];
    let fetch = MockFetch::new(vec![sse(&events)]);
    let parts: Vec<_> = model(&fetch)
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .try_collect()
        .await
        .unwrap();
    assert_request(&fetch, true, false);
    let text: String = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello");
    let finishes: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata,
            } => Some((finish_reason, usage, provider_metadata)),
            _ => None,
        })
        .collect();
    assert_eq!(finishes.len(), 1);
    let (reason, usage, metadata) = finishes[0];
    assert_eq!(reason.unified, FinishReasonUnified::Stop);
    assert_eq!(reason.raw.as_deref(), Some("completed"));
    assert_eq!(usage.input_tokens.total, Some(10));
    assert_eq!(usage.output_tokens.total, Some(5));
    assert_eq!(
        metadata.as_ref().unwrap(),
        &provider_namespace("xai", json!({"costInUsdTicks": 113500}))
    );
    assert!(
        !parts
            .iter()
            .any(|part| matches!(part, StreamPart::Error { .. }))
    );
}

/// TS: "should forward web_search_call action and sources to tool-result" (xai/src/responses/xai-responses-language-model.test.ts)
#[tokio::test]
async fn tool_results_preserve_actions_and_sources() {
    let body = json!({
        "id": "resp_123",
        "object": "response",
        "status": "completed",
        "model": "grok-4-fast-non-reasoning",
        "output": [
            {"type": "web_search_call", "id": "ws_action_1", "name": "web_search", "call_id": "", "status": "completed", "action": {"type": "search", "query": "latest AI news", "sources": [{"type": "url", "url": "https://example.com/a"}, {"type": "url", "url": "https://example.com/b"}]}},
            {"type": "web_search_call", "id": "ws_action_2", "name": "web_search", "call_id": "", "status": "completed", "action": {"type": "open_page", "url": "https://example.com/a"}},
            {"type": "web_search_call", "id": "ws_action_3", "name": "web_search", "call_id": "", "status": "completed", "action": {"type": "find_in_page", "url": "https://example.com/a", "pattern": "climate"}},
            {"type": "web_search_call", "id": "ws_action_4", "name": "web_search", "call_id": "", "status": "completed", "action": {"type": "search", "query": null, "sources": null}},
            {"type": "web_search_call", "id": "ws_action_5", "name": "web_search", "call_id": "", "status": "completed", "action": {"type": "open_page", "url": null}},
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5},
    });
    let fetch = MockFetch::new(vec![Canned::json(&body)]);
    let mut options = options();
    options.tools = Some(vec![Tool::Provider(ProviderTool {
        id: "xai.web_search".into(),
        name: "web_search".into(),
        args: json!({}),
    })]);
    let result = model(&fetch).do_generate(&options).await.unwrap();
    assert_request(&fetch, false, true);
    let expected = vec![
        json!({"action": {"type": "search", "query": "latest AI news"}, "sources": [{"type": "url", "url": "https://example.com/a"}, {"type": "url", "url": "https://example.com/b"}]}),
        json!({"action": {"type": "openPage", "url": "https://example.com/a"}}),
        json!({"action": {"type": "findInPage", "url": "https://example.com/a", "pattern": "climate"}}),
        json!({"action": {"type": "search"}}),
        json!({"action": {"type": "openPage", "url": null}}),
    ];
    assert_eq!(result.content.len(), expected.len() * 2);
    for (index, (pair, expected_result)) in result
        .content
        .as_chunks::<2>()
        .0
        .iter()
        .zip(expected)
        .enumerate()
    {
        let id = format!("ws_action_{}", index + 1);
        let GenerateContent::ToolCall(call) = &pair[0] else {
            panic!("missing tool call")
        };
        assert_eq!(call.tool_call_id, id);
        assert_eq!(call.tool_name, "web_search");
        assert_eq!(call.input, "");
        assert_eq!(call.provider_executed, Some(true));
        let GenerateContent::ToolResult(output) = &pair[1] else {
            panic!("missing tool result")
        };
        assert_eq!(output.tool_call_id, id);
        assert_eq!(output.tool_name, "web_search");
        assert_eq!(output.result, expected_result);
        assert_eq!(output.is_error, None);
    }
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
}

/// TS: "should set finish reason to error" (xai/src/responses/xai-responses-language-model.test.ts)
#[tokio::test]
async fn failed_response_emits_error_and_finish() {
    let events = vec![
        json!({"type": "response.created", "response": {"id": "resp_123", "object": "response", "model": "grok-4-fast-non-reasoning", "output": []}}),
        json!({"type": "response.failed", "response": {"error": {"code": "server_error", "message": "Internal server error"}, "usage": {"input_tokens": 50, "output_tokens": 0}}}),
    ];
    let fetch = MockFetch::new(vec![sse(&events)]);
    let parts: Vec<_> = model(&fetch)
        .do_stream(&options())
        .await
        .unwrap()
        .stream
        .try_collect()
        .await
        .unwrap();
    assert_request(&fetch, true, false);
    let errors: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::Error { error } => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1);
    let AiMuxError::ApiCall(error) = errors[0] else {
        panic!("expected API error")
    };
    assert_eq!(error.message, "Internal server error");
    assert_eq!(error.provider_code.as_deref(), Some("server_error"));
    assert_eq!(error.status_code, Some(500));
    assert!(error.is_retryable);
    assert_eq!(error.data.as_ref(), Some(&events[1]));
    let finishes: Vec<_> = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::Finish {
                finish_reason,
                usage,
                ..
            } => Some((finish_reason, usage)),
            _ => None,
        })
        .collect();
    assert_eq!(finishes.len(), 1);
    let (reason, usage) = finishes[0];
    assert_eq!(reason.unified, FinishReasonUnified::Error);
    assert_eq!(reason.raw.as_deref(), Some("error"));
    assert_eq!(usage.input_tokens.total, Some(50));
    assert_eq!(usage.output_tokens.total, Some(0));
}
