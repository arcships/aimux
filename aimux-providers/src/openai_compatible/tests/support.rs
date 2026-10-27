//! The harness of the older end-to-end ports (previously
//! `tests/openai_compatible_upstream_e2e_test.rs`), shared by their parts in
//! `chat/`, `completion/`, `embedding/` and `image/`.

use std::sync::Arc;

use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use serde_json::{Value, json};

use crate::openai_compatible::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};
use crate::test_support::mock_fetch::{Canned, MockFetch};

pub(super) const BASE_URL: &str = "https://my.api.com/v1";

pub(super) fn provider(fetch: &Arc<MockFetch>) -> OpenAICompatibleProvider {
    create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "test-provider".into(),
        base_url: BASE_URL.into(),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

pub(super) fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

pub(super) fn chat_response(message: Value, usage: Value) -> Value {
    json!({"id":"chatcmpl-test", "created":1711115037, "model":"test-model",
        "choices":[{"index":0,"message":message,"finish_reason":"stop"}],
        "usage":usage})
}

pub(super) fn stream_response(values: &[Value]) -> Canned {
    let mut body = values
        .iter()
        .map(|value| format!("data: {value}\n\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

pub(super) fn assert_request(fetch: &MockFetch, path: &str, body: Value) {
    let requests = fetch.seen();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, format!("{BASE_URL}{path}"));
    assert_eq!(requests[0].json_body(), body);
}

pub(super) fn stream_request() -> Value {
    json!({"model":"test-model","messages":[{"role":"user","content":"Hello"}],"stream":true})
}
