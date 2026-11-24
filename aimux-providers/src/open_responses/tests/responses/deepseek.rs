//! DeepSeek Responses extension contracts (#525), using the real endpoint with
//! an injected transport. No network or API credential is needed.
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, Source};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{RawToolCall, ToolResult};
use aimux_core::types::ProviderMetadata;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::open_responses::{
    OpenResponsesModel, OpenResponsesProviderSettings, create_open_responses,
};
use crate::test_support::mock_fetch::{Canned, MockFetch};

fn model(endpoint: &str, id: &str, replies: Vec<Canned>) -> (OpenResponsesModel, Arc<MockFetch>) {
    let mock = MockFetch::new(replies);
    let mut settings = OpenResponsesProviderSettings::new("deepseek", endpoint);
    settings.fetch = Some(mock.transport());
    (create_open_responses(settings).unwrap().responses(id), mock)
}

fn options(search: Option<bool>) -> CallOptions {
    let mut options = CallOptions::new(vec![LanguageModelMessage::user_text("搜索火星")]);
    options.provider_options = search.map(|enabled| {
        serde_json::from_value(json!({
            "deepseek": {"deepseekWebSearch": enabled}
        }))
        .unwrap()
    });
    options
}

fn sse(events: &[Value]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>()
            .into_bytes(),
    }
}

fn response(output: Value) -> Value {
    json!({"id":"r1", "model":"deepseek-flash", "status":"completed", "output": output})
}

#[tokio::test]
async fn interleaved_search_citations_and_fresh_instance_replay() {
    let reasoning = json!({"id":"r", "type":"reasoning", "content":[{"type":"reasoning_text","text":"查找"}], "summary":[], "signature":"reason-sig", "kind":"opaque"});
    let search_a = json!({"id":"a", "type":"web_search_call", "status":"completed", "kind":"search", "signature":"sig-a", "action":{"type":"search","query":"火星","sources":[{"url":"https://example.com/a"}]}});
    let search_b = json!({"id":"b", "type":"web_search_call", "status":"failed", "kind":"page", "signature":"sig-b", "action":{"type":"open_page","url":"https://example.com/b"}, "page":{"title":"页面"}, "error":{"message":"unavailable"}});
    let citation = json!({"type":"url_citation", "url":"https://example.com/a", "title":"火星", "start_index":2, "end_index":4});
    let message = json!({"id":"m", "type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"前言", "annotations":[]}, {"type":"output_text", "text":"🌍看火星", "annotations":[citation.clone()]}]});
    let function = json!({"id":"f", "type":"function_call", "call_id":"local-call", "name":"weather", "arguments":"{}", "signature":"function-sig"});
    let ordered = json!([reasoning, search_a, message, search_b, function]);
    let events = vec![
        json!({"type":"response.output_item.added", "output_index":0, "item":reasoning}),
        json!({"type":"response.output_item.added", "output_index":1, "item":{"type":"web_search_call","id":"a"}}),
        json!({"type":"response.output_item.added", "output_index":3, "item":{"type":"web_search_call","id":"b"}}),
        // Deliberately complete the later search first.
        json!({"type":"response.output_item.done", "output_index":3, "item":search_b}),
        json!({"type":"response.output_item.done", "output_index":0, "item":reasoning}),
        json!({"type":"response.output_item.added", "output_index":2, "item":{"type":"message", "id":"m"}}),
        json!({"type":"response.output_text.delta", "item_id":"m", "output_index":2, "content_index":1, "delta":"🌍看火星"}),
        json!({"type":"response.output_text.annotation.added", "item_id":"m", "output_index":2, "content_index":1, "annotation_index":0, "annotation":citation}),
        json!({"type":"response.output_item.done", "output_index":1, "item":search_a}),
        json!({"type":"response.output_item.done", "output_index":2, "item":message}),
        json!({"type":"response.output_item.done", "output_index":4, "item":function}),
        json!({"type":"response.completed", "response":response(json!([]))}),
    ];
    let (first, transport) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![sse(&events)],
    );
    let parts = first
        .do_stream(&options(Some(true)))
        .await
        .unwrap()
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let sent = &transport.seen()[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, "https://api.deepseek.com/responses");
    assert_eq!(sent.json_body()["model"], "deepseek-flash");
    assert_eq!(
        sent.json_body()["tools"],
        json!([{"type":"deepseek_web_search"}])
    );
    let searches = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolCall(call) if call.tool_name == "deepseek_web_search" => Some(call),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(searches.len(), 2);
    assert!(
        searches
            .iter()
            .all(|call| call.provider_executed == Some(true))
    );
    assert_eq!(searches[0].tool_call_id, "b");
    assert_eq!(searches[1].tool_call_id, "a");
    let results = parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results[0].result, search_b);
    assert_eq!(results[0].is_error, Some(true));
    assert_eq!(results[1].result, search_a);
    let source = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Source(Source::Url {
                url,
                title,
                provider_metadata,
                ..
            }) => Some((url, title, provider_metadata)),
            _ => None,
        })
        .unwrap();
    assert_eq!(source.0, "https://example.com/a");
    assert_eq!(source.1.as_deref(), Some("火星"));
    let citation_meta = &source.2.as_ref().unwrap()["deepseek"];
    assert_eq!(citation_meta["itemId"], "m");
    assert_eq!(citation_meta["contentIndex"], 1);
    assert_eq!(citation_meta["startIndex"], 2);
    assert_eq!(citation_meta["endIndex"], 4);
    assert_eq!(citation_meta["offsetUnit"], "unicode-code-point");
    let metadata = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(metadata["deepseek"]["responseOutput"]["items"], ordered);
    // Persist via JSON and replay in a fresh provider, without enabling search.
    let saved = serde_json::to_string(&metadata).unwrap();
    let mut next = options(None);
    next.prompt.push(LanguageModelMessage::Assistant {
        content: vec![],
        provider_options: Some(serde_json::from_str(&saved).unwrap()),
    });
    next.prompt.push(serde_json::from_value(json!({"role":"tool", "content":[{"type":"tool-result","toolCallId":"local-call","toolName":"weather","output":{"type":"text","value":"sunny"}}]})).unwrap());
    next.prompt.push(LanguageModelMessage::user_text("继续"));
    let (second, transport) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![Canned::json(&response(json!([])))],
    );
    second.do_generate(&next).await.unwrap();
    let body = transport.seen()[0].json_body();
    assert!(body.get("tools").is_none());
    assert!(body.get("previous_response_id").is_none());
    let input = body["input"].as_array().unwrap();
    assert_eq!(&input[1..6], ordered.as_array().unwrap());
    assert_eq!(input[6]["type"], "function_call_output");
    assert_eq!(input[7]["content"][0]["text"], "继续");
}

#[tokio::test]
async fn search_off_retains_ordinary_request_shape() {
    let (model, transport) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![
            Canned::json(&response(json!([]))),
            Canned::json(&response(json!([]))),
        ],
    );
    model.do_generate(&options(None)).await.unwrap();
    model.do_generate(&options(Some(false))).await.unwrap();
    let requests = transport.seen();
    assert_eq!(requests[0].json_body(), requests[1].json_body());
    assert!(requests[0].json_body().get("tools").is_none());
}

#[tokio::test]
async fn rejects_legacy_search_and_incompatible_routes_before_fetch() {
    for (endpoint, id) in [
        ("https://relay.example/v1", "deepseek-flash"),
        ("https://api.deepseek.com", "other-model"),
    ] {
        let (model, transport) = model(endpoint, id, vec![]);
        assert!(model.do_generate(&options(Some(true))).await.is_err());
        assert!(transport.seen().is_empty());
    }
    let (model, transport) = model("https://api.deepseek.com", "deepseek-flash", vec![]);
    let mut call = options(Some(true));
    call.tools = Some(
        serde_json::from_value(
            json!([{"type":"provider", "id":"openai.web_search", "name":"web_search", "args":{}}]),
        )
        .unwrap(),
    );
    assert!(model.do_generate(&call).await.is_err());
    assert!(transport.seen().is_empty());
}

#[tokio::test]
async fn nonstream_search_is_provider_executed_and_preserves_replay() {
    let item = json!({"type":"web_search_call","id":"s","status":"completed","signature":"keep", "action":{"sources":[{"url":"https://example.com"}]}});
    let (model, _) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![Canned::json(&response(json!([item])))],
    );
    let result = model.do_generate(&options(Some(true))).await.unwrap();
    assert!(
        matches!(&result.content[0], GenerateContent::ToolCall(call) if call.provider_executed == Some(true))
    );
    assert!(
        matches!(&result.content[1], GenerateContent::ToolResult(result) if result.result == item)
    );
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["responseOutput"]["items"],
        json!([item])
    );
}

#[tokio::test]
async fn replay_is_validated_when_search_is_absent() {
    for replay in [
        json!({"modelId":"wrong","endpoint":"https://api.deepseek.com/responses","items":[]}),
        json!({"modelId":"deepseek-flash","endpoint":"https://api.deepseek.com/responses","items":[{"type":"message","role":"system","content":[]}]}),
        json!({"modelId":"deepseek-flash","endpoint":"https://api.deepseek.com/responses","items":{}}),
    ] {
        let (model, transport) = model("https://api.deepseek.com", "deepseek-flash", vec![]);
        let mut call = options(None);
        call.prompt.push(LanguageModelMessage::Assistant {
            content: vec![],
            provider_options: Some(
                serde_json::from_value(json!({"deepseek":{"responseOutput":replay}})).unwrap(),
            ),
        });
        assert!(model.do_generate(&call).await.is_err());
        assert!(transport.seen().is_empty());
    }
}

async fn stream_parts(model: &OpenResponsesModel, options: &CallOptions) -> Vec<StreamPart> {
    model
        .do_stream(options)
        .await
        .unwrap()
        .stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn finish_metadata(parts: &[StreamPart]) -> ProviderMetadata {
    parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                provider_metadata, ..
            } => provider_metadata.clone(),
            _ => None,
        })
        .unwrap()
}

fn tool_calls(parts: &[StreamPart]) -> Vec<&RawToolCall> {
    parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolCall(call) if call.tool_name == "deepseek_web_search" => Some(call),
            _ => None,
        })
        .collect()
}

fn tool_results(parts: &[StreamPart]) -> Vec<&ToolResult> {
    parts
        .iter()
        .filter_map(|part| match part {
            StreamPart::ToolResult(result) if result.tool_name == "deepseek_web_search" => {
                Some(result)
            }
            _ => None,
        })
        .collect()
}

// A search item that only appears in the terminal `response.output` must still
// reach the caller, and its recovered id must reach the replay record.
#[tokio::test]
async fn terminal_output_publishes_search_items_without_done_events() {
    let message = json!({"id":"m", "type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"答", "annotations":[]}]});
    let reported = json!({"type":"web_search_call", "status":"completed", "action":{"type":"search", "query":"火星", "sources":[{"url":"https://example.com/a"}]}});
    let events = vec![
        json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"web_search_call", "id":"late"}}),
        json!({"type":"response.output_item.done", "output_index":1, "item":message}),
        json!({"type":"response.completed", "response":response(json!([reported, message]))}),
    ];
    let (model, _) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![sse(&events)],
    );
    let parts = stream_parts(&model, &options(Some(true))).await;
    let calls = tool_calls(&parts);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_call_id, "late");
    let results = tool_results(&parts);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tool_call_id, "late");
    let mut persisted = reported.clone();
    persisted["id"] = json!("late");
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["responseOutput"]["items"],
        json!([persisted, message])
    );
}

// A completed item is normally reported twice: once by the per-item done
// event and again inside the terminal `response.output`. Both paths feed one
// collector, so an `output_index` publishes one call and one result no matter
// which path reports it first.
#[tokio::test]
async fn item_reported_by_done_and_terminal_output_is_published_once() {
    let search = json!({"id":"s", "type":"web_search_call", "status":"completed", "action":{"type":"search", "query":"火星", "sources":[{"url":"https://example.com/a"}]}});
    let message = json!({"id":"m", "type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"答", "annotations":[]}]});
    let terminal = response(json!([search, message]));
    let added = vec![
        json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"web_search_call", "id":"s"}}),
        json!({"type":"response.output_item.added", "output_index":1, "item":{"type":"message", "id":"m"}}),
    ];
    let done = vec![
        json!({"type":"response.output_item.done", "output_index":1, "item":message}),
        json!({"type":"response.output_item.done", "output_index":0, "item":search}),
    ];
    let completed = json!({"type":"response.completed", "response":terminal});
    let mut done_first = added.clone();
    done_first.extend(done.iter().cloned());
    done_first.push(completed.clone());
    let mut terminal_first = added;
    terminal_first.push(completed);
    terminal_first.extend(done);
    for events in [done_first, terminal_first] {
        let (model, _) = model(
            "https://api.deepseek.com",
            "deepseek-flash",
            vec![sse(&events)],
        );
        let parts = stream_parts(&model, &options(Some(true))).await;
        let calls = tool_calls(&parts);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id, "s");
        let results = tool_results(&parts);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_call_id, "s");
        assert_eq!(
            finish_metadata(&parts)["deepseek"]["responseOutput"]["items"],
            json!([search, message])
        );
    }
}

// `"id": null` is missing identity too: the `added` id must be used both on
// the call and in the stored item, instead of the synthetic fallback id.
#[tokio::test]
async fn null_item_id_is_recovered_into_the_replay_record() {
    let reported = json!({"id":null, "type":"web_search_call", "status":"completed", "action":{"type":"search", "query":"q"}});
    let events = vec![
        json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"web_search_call", "id":"real"}}),
        json!({"type":"response.output_item.done", "output_index":0, "item":reported}),
        json!({"type":"response.completed", "response":response(json!([]))}),
    ];
    let (model, _) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![sse(&events)],
    );
    let parts = stream_parts(&model, &options(Some(true))).await;
    assert_eq!(tool_calls(&parts)[0].tool_call_id, "real");
    assert_eq!(tool_results(&parts)[0].tool_call_id, "real");
    let mut persisted = reported.clone();
    persisted["id"] = json!("real");
    assert_eq!(
        finish_metadata(&parts)["deepseek"]["responseOutput"]["items"],
        json!([persisted])
    );
}

#[tokio::test]
async fn non_boolean_option_is_rejected_before_fetch() {
    let (model, transport) = model("https://api.deepseek.com", "deepseek-flash", vec![]);
    let mut call = options(None);
    call.provider_options =
        Some(serde_json::from_value(json!({"deepseek":{"deepseekWebSearch":"yes"}})).unwrap());
    let error = model.do_generate(&call).await.unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, .. }
            if message == "deepseekWebSearch must be a boolean"),
        "unexpected error: {error:?}"
    );
    assert!(transport.seen().is_empty());
}

// The replay record is attached for the official route even without the search
// opt-in, because a later turn still replays signed reasoning output; other
// routes keep their previous `providerMetadata` of `null`.
#[tokio::test]
async fn replay_record_is_attached_without_the_search_opt_in() {
    let items = json!([{"id":"r", "type":"reasoning", "content":[{"type":"reasoning_text", "text":"想"}], "summary":[], "signature":"sig", "kind":"opaque"}]);
    let (official, _) = model(
        "https://api.deepseek.com",
        "deepseek-flash",
        vec![Canned::json(&response(items.clone()))],
    );
    let result = official.do_generate(&options(None)).await.unwrap();
    assert_eq!(
        result.provider_metadata.unwrap()["deepseek"]["responseOutput"]["items"],
        items
    );
    let (relay, _) = model(
        "https://relay.example/v1",
        "deepseek-flash",
        vec![Canned::json(&response(items))],
    );
    assert!(
        relay
            .do_generate(&options(None))
            .await
            .unwrap()
            .provider_metadata
            .is_none()
    );
}

// The stream `Finish` carries the same replay record, including when the
// search opt-in is absent or `false`; other endpoints keep it `null`.
#[tokio::test]
async fn stream_finish_keeps_the_replay_record_without_the_search_opt_in() {
    let reasoning = json!({"id":"r", "type":"reasoning", "content":[{"type":"reasoning_text", "text":"想"}], "summary":[], "signature":"sig", "kind":"opaque"});
    let events = vec![
        json!({"type":"response.output_item.added", "output_index":0, "item":reasoning}),
        json!({"type":"response.output_item.done", "output_index":0, "item":reasoning}),
        json!({"type":"response.completed", "response":response(json!([reasoning]))}),
    ];
    let expected = json!({
        "modelId": "deepseek-flash",
        "endpoint": "https://api.deepseek.com/responses",
        "items": [reasoning],
    });
    for search in [None, Some(false)] {
        let (official, transport) = model(
            "https://api.deepseek.com",
            "deepseek-flash",
            vec![sse(&events)],
        );
        let parts = stream_parts(&official, &options(search)).await;
        assert!(transport.seen()[0].json_body().get("tools").is_none());
        assert_eq!(
            finish_metadata(&parts)["deepseek"]["responseOutput"],
            expected
        );
    }
    let (relay, _) = model(
        "https://relay.example/v1",
        "deepseek-flash",
        vec![sse(&events)],
    );
    let parts = stream_parts(&relay, &options(None)).await;
    let finish = parts
        .iter()
        .find_map(|part| match part {
            StreamPart::Finish {
                provider_metadata, ..
            } => Some(provider_metadata.clone()),
            _ => None,
        })
        .unwrap();
    assert!(finish.is_none(), "unexpected metadata: {finish:?}");
}
