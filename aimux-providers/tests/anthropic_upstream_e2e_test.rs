//! Small factory-level subset of the ported Anthropic upstream cases.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::GenerateContent;
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::FinishReasonUnified;
use aimux_provider_utils::Resolvable;
use aimux_providers::anthropic::{AnthropicProvider, AnthropicProviderSettings, create_anthropic};
use futures::StreamExt;
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some(Resolvable::Value("test-api-key".into())),
        base_url: Some("https://test.invalid/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn options() -> CallOptions {
    CallOptions {
        max_output_tokens: Some(4096),
        ..CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
    }
}

fn body(content: Value, stop_reason: &str, usage: Value) -> Value {
    json!({
        "id": "msg_017TfcQ4AgGxKyBduUpqYPZn", "type": "message",
        "role": "assistant", "content": content, "model": "test-model",
        "stop_reason": stop_reason, "stop_sequence": null, "usage": usage
    })
}

fn request() -> Value {
    json!({
        "model": "test-model", "max_tokens": 4096,
        "messages": [{"role": "user", "content": [{"type": "text", "text": "Hello"}]}]
    })
}

/// TS: "should use native structured output without a JSON tool fallback" (anthropic/src/anthropic-language-model.test.ts)
#[tokio::test]
async fn generate_native_structured_output() {
    let text = "{\"name\":\"Classic Lasagna\"}";
    let response = body(
        json!([{"type": "text", "text": text}]),
        "end_turn",
        json!({"input_tokens": 371, "output_tokens": 629}),
    );
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"], "additionalProperties": false});
    let mut options = options();
    options.max_output_tokens = Some(100);
    options.response_format = Some(ResponseFormat::Json {
        schema: Some(schema.clone()),
        name: None,
        description: None,
    });
    // Select native output explicitly so the neutral model ID needs no family detection.
    options.provider_options = Some(provider_namespace(
        "anthropic",
        json!({"structuredOutputMode": "outputFormat"}),
    ));
    let result = provider(&fetch)
        .messages("test-model")
        .do_generate(&options)
        .await
        .unwrap();
    let seen = fetch.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "https://test.invalid/v1/messages");
    assert_eq!(seen[0].headers["x-api-key"], "test-api-key");
    let mut expected = request();
    expected["max_tokens"] = json!(100);
    expected["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
    assert_eq!(seen[0].json_body(), expected);
    assert_eq!(
        result.content,
        vec![GenerateContent::Text {
            text: text.into(),
            provider_metadata: None
        }]
    );
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(result.usage.input_tokens.total, Some(371));
    assert_eq!(result.usage.input_tokens.no_cache, Some(371));
    assert_eq!(result.usage.input_tokens.cache_read, Some(0));
    assert_eq!(result.usage.input_tokens.cache_write, Some(0));
    assert_eq!(result.usage.output_tokens.total, Some(629));
    assert_eq!(result.request.unwrap().body, Some(expected));
    let returned = result.response.unwrap();
    assert_eq!(returned.model_id.as_deref(), Some("test-model"));
    assert_eq!(returned.id.as_deref(), Some("msg_017TfcQ4AgGxKyBduUpqYPZn"));
    assert_eq!(returned.body, Some(response));
}

/// TS: "should stream text deltas" (anthropic/src/anthropic-language-model.test.ts)
#[tokio::test]
async fn stream_text_deltas() {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_01KfpJoAEabmH2iHRRFjQMAG", "type": "message", "role": "assistant", "content": [], "model": "test-model", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 17, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello"}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": ", "}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "World!"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 227}}),
        json!({"type": "message_stop"}),
    ];
    let fetch = MockFetch::new(vec![Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }]);
    let result = provider(&fetch)
        .messages("test-model")
        .do_stream(&options())
        .await
        .unwrap();
    let mut expected = request();
    expected["stream"] = json!(true);
    assert_eq!(fetch.seen().len(), 1);
    assert_eq!(fetch.seen()[0].json_body(), expected);
    assert_eq!(result.request.unwrap().body, Some(expected));
    let parts = result
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(parts.len(), 8);
    assert!(matches!(&parts[0], StreamPart::StreamStart { warnings } if warnings.is_empty()));
    assert!(
        matches!(&parts[1], StreamPart::ResponseMetadata(metadata) if metadata.id.as_deref() == Some("msg_01KfpJoAEabmH2iHRRFjQMAG") && metadata.model_id.as_deref() == Some("test-model") && metadata.timestamp.is_none())
    );
    assert!(
        matches!(&parts[2], StreamPart::TextStart { id, provider_metadata } if id == "0" && provider_metadata.is_none())
    );
    for (part, expected_delta) in parts[3..6].iter().zip(["Hello", ", ", "World!"]) {
        assert!(
            matches!(part, StreamPart::TextDelta { id, delta, provider_metadata } if id == "0" && delta == expected_delta && provider_metadata.is_none())
        );
    }
    assert!(
        matches!(&parts[6], StreamPart::TextEnd { id, provider_metadata } if id == "0" && provider_metadata.is_none())
    );
    let StreamPart::Finish {
        finish_reason,
        usage,
        provider_metadata,
    } = &parts[7]
    else {
        panic!("expected finish");
    };
    assert_eq!(finish_reason.unified, FinishReasonUnified::Stop);
    assert_eq!(finish_reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(usage.input_tokens.total, Some(17));
    assert_eq!(usage.input_tokens.no_cache, Some(17));
    assert_eq!(usage.input_tokens.cache_read, Some(0));
    assert_eq!(usage.input_tokens.cache_write, Some(0));
    assert_eq!(usage.output_tokens.total, Some(227));
    assert_eq!(
        usage.raw,
        Some(json!({"input_tokens": 17, "output_tokens": 227}))
    );
    assert_eq!(provider_metadata.as_ref().unwrap()["anthropic"], serde_json::from_value::<serde_json::Map<String, Value>>(json!({"container": null, "contextManagement": null, "iterations": null, "stopSequence": null, "usage": {"input_tokens": 17, "output_tokens": 227}})).unwrap());
}

/// TS: "should extract tool calls" (anthropic/src/anthropic-language-model.test.ts)
#[tokio::test]
async fn generate_tool_call() {
    let response = body(
        json!([
            {"type": "text", "text": "Some text\n\n"},
            {"type": "tool_use", "id": "toolu_1", "name": "test-tool", "input": {"value": "example value"}}
        ]),
        "tool_use",
        json!({"input_tokens": 4, "output_tokens": 30}),
    );
    let fetch = MockFetch::new(vec![Canned::json(&response)]);
    let schema = json!({"type": "object", "properties": {"value": {"type": "string"}}, "required": ["value"], "additionalProperties": false, "$schema": "http://json-schema.org/draft-07/schema#"});
    let mut options = options();
    options.tools = Some(vec![Tool::Function(FunctionTool::new(
        "test-tool",
        schema.clone(),
    ))]);
    let result = provider(&fetch)
        .messages("test-model")
        .do_generate(&options)
        .await
        .unwrap();
    let mut expected = request();
    expected["tools"] = json!([{"name": "test-tool", "input_schema": schema}]);
    expected["tool_choice"] = json!({"type": "auto"});
    assert_eq!(fetch.seen().len(), 1);
    assert_eq!(fetch.seen()[0].json_body(), expected);
    assert_eq!(result.content.len(), 2);
    assert_eq!(
        result.content[0],
        GenerateContent::Text {
            text: "Some text\n\n".into(),
            provider_metadata: None
        }
    );
    let GenerateContent::ToolCall(call) = &result.content[1] else {
        panic!("expected tool call");
    };
    assert_eq!(call.tool_call_id, "toolu_1");
    assert_eq!(call.tool_name, "test-tool");
    assert_eq!(call.input, "{\"value\":\"example value\"}");
    assert_eq!(call.provider_metadata, None);
    assert_eq!(result.finish_reason.unified, FinishReasonUnified::ToolCalls);
    assert_eq!(result.finish_reason.raw.as_deref(), Some("tool_use"));
    assert_eq!(result.usage.input_tokens.total, Some(4));
    assert_eq!(result.usage.output_tokens.total, Some(30));
}

/// TS: "should parse overloaded error" (anthropic/src/anthropic-error.test.ts)
#[tokio::test]
async fn overloaded_error_response() {
    let mut canned = Canned::json(
        &json!({"type": "error", "error": {"details": null, "type": "overloaded_error", "message": "Overloaded"}}),
    );
    canned.status = 529;
    let fetch = MockFetch::new(vec![canned]);
    let error = provider(&fetch)
        .messages("test-model")
        .do_generate(&options())
        .await
        .unwrap_err();
    assert_eq!(fetch.seen().len(), 1);
    assert_eq!(fetch.seen()[0].json_body(), request());
    let AiMuxError::ApiCall(error) = error else {
        panic!("expected provider API error");
    };
    assert_eq!(error.message, "Overloaded");
    assert_eq!(error.provider_code.as_deref(), Some("overloaded_error"));
    assert_eq!(error.status_code, Some(529));
}
