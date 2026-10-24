//! Provider-defined tool args are checked as `@ai-sdk/anthropic` 4.0.71
//! `prepareTools` checks them with `validateTypes`
//! (`anthropic-prepare-tools.ts:197-225`, `:264-276`, `:303-408`, `:425-445`):
//! a mismatch fails the call before any request, and the tools upstream does
//! not validate pass their args through unchecked.

#[path = "common/mock_fetch.rs"]
mod mock_fetch;

use aimux_core::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::tool::{ProviderTool, Tool};
use aimux_providers::anthropic::{AnthropicProvider, AnthropicProviderSettings, create_anthropic};
use mock_fetch::{Canned, MockFetch};
use serde_json::{Value, json};

fn provider(fetch: &std::sync::Arc<MockFetch>) -> AnthropicProvider {
    create_anthropic(AnthropicProviderSettings {
        api_key: Some("test-api-key".into()),
        base_url: Some("https://test.invalid/v1".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

fn text_response() -> Canned {
    Canned::json(&json!({
        "id": "msg_1", "type": "message", "role": "assistant",
        "content": [{"type": "text", "text": "done"}],
        "model": "test-model", "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 4, "output_tokens": 1}
    }))
}

fn tool(id: &str, args: Value) -> Tool {
    Tool::Provider(ProviderTool {
        id: id.parse().unwrap(),
        name: "tool".to_string(),
        args: args.as_object().cloned().unwrap(),
        supports_deferred_results: None,
    })
}

/// Run one call with `tool`; the sent tools, or the error.
async fn call(tool: Tool) -> (Result<Value, AiMuxError>, usize) {
    let fetch = MockFetch::new(vec![text_response()]);
    let options = CallOptions {
        max_output_tokens: Some(100),
        tools: Some(vec![tool]),
        ..CallOptions::new(vec![LanguageModelMessage::user_text("Hi")])
    };
    let result = provider(&fetch)
        .messages("test-model")
        .do_generate(&options)
        .await
        .map(|_| fetch.seen()[0].json_body()["tools"].clone());
    (result, fetch.seen().len())
}

/// Each validated tool with args that match its schema, and args that don't.
fn cases() -> Vec<(&'static str, Value, Value)> {
    let web = json!({
        "maxUses": 2, "allowedDomains": ["a.example"], "blockedDomains": ["b.example"]
    });
    let with = |extra: Value| {
        let mut args = web.clone();
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap());
        args
    };
    let fetch = with(json!({ "citations": { "enabled": true }, "maxContentTokens": 500 }));
    let search = with(json!({
        "userLocation": { "type": "approximate", "city": "Berlin", "timezone": "Europe/Berlin" }
    }));
    vec![
        (
            "anthropic.web_fetch_20250910",
            fetch.clone(),
            json!({ "citations": {} }),
        ),
        (
            "anthropic.web_fetch_20260209",
            fetch.clone(),
            json!({ "maxUses": "2" }),
        ),
        (
            "anthropic.web_fetch_20260318",
            with(json!({ "useCache": false, "responseInclusion": "excluded" })),
            json!({ "responseInclusion": "partial" }),
        ),
        (
            "anthropic.web_search_20250305",
            search.clone(),
            json!({ "userLocation": { "type": "exact" } }),
        ),
        (
            "anthropic.web_search_20260209",
            search.clone(),
            json!({ "allowedDomains": "a.example" }),
        ),
        (
            "anthropic.web_search_20260318",
            with(json!({ "responseInclusion": "full" })),
            json!({ "blockedDomains": [1] }),
        ),
        (
            "anthropic.advisor_20260301",
            json!({
                "model": "claude-opus-4-8", "maxUses": 1, "maxTokens": 1024,
                "caching": { "type": "ephemeral", "ttl": "5m" }
            }),
            json!({ "model": "claude-opus-4-8", "maxTokens": 1023 }),
        ),
        (
            "anthropic.text_editor_20250728",
            json!({ "maxCharacters": 1000 }),
            json!({ "maxCharacters": null }),
        ),
        (
            "anthropic.computer_toolset_20260801",
            json!({ "configs": { "zoom": { "enabled": false }, "wait": { "deferLoading": true } } }),
            json!({ "configs": { "drag": { "enabled": true } } }),
        ),
    ]
}

#[tokio::test]
async fn args_that_match_the_schema_are_sent() {
    for (id, valid, _) in cases() {
        let (result, requests) = call(tool(id, valid)).await;
        assert!(result.is_ok(), "{id}: {result:?}");
        assert_eq!(requests, 1, "{id}");
    }
}

/// partial: `should reject advisor_20260301 maxTokens below 1024`; full port in src/anthropic/tests/prepare_tools.rs.
#[tokio::test]
async fn args_that_do_not_match_fail_the_call_before_any_request() {
    for (id, _, invalid) in cases() {
        let (result, requests) = call(tool(id, invalid)).await;
        match result {
            Err(AiMuxError::InvalidArgument { message, .. }) => assert_eq!(
                message,
                format!("Invalid configuration for provider-defined tool {id}")
            ),
            other => panic!("{id}: {other:?}"),
        }
        assert_eq!(requests, 0, "{id}");
    }
}

#[tokio::test]
async fn the_advisor_needs_a_model() {
    let (result, requests) = call(tool("anthropic.advisor_20260301", json!({}))).await;
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument { .. })),
        "{result:?}"
    );
    assert_eq!(requests, 0);
}

#[tokio::test]
async fn a_toolset_config_must_be_an_object() {
    let (result, _) = call(tool(
        "anthropic.computer_toolset_20260801",
        json!({ "configs": { "zoom": true } }),
    ))
    .await;
    assert!(
        matches!(result, Err(AiMuxError::InvalidArgument { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn tools_upstream_does_not_validate_pass_their_args_through() {
    let (result, _) = call(tool(
        "anthropic.computer_20250124",
        json!({ "displayWidthPx": "wide", "displayHeightPx": 768 }),
    ))
    .await;
    assert_eq!(
        result.unwrap(),
        json!([{
            "name": "computer", "type": "computer_20250124",
            "display_width_px": "wide", "display_height_px": 768
        }])
    );
}
