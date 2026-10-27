//! The shared helpers of the older ports (previously `tests/bedrock_upstream_e2e_test.rs`),
//! used by their parts across `src/bedrock/tests/`.

use crate::bedrock::event_stream_decoder::encode_message;
use crate::bedrock::{AmazonBedrockProviderSettings, create_amazon_bedrock};
use crate::test_support::mock_fetch::{Canned, MockFetch};
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::options::CallOptions;
use aimux_provider_utils::Resolvable;
use serde_json::{Value, json};

pub(super) fn provider(fetch: &std::sync::Arc<MockFetch>) -> crate::bedrock::AmazonBedrockProvider {
    create_amazon_bedrock(AmazonBedrockProviderSettings {
        api_key: Some(Resolvable::Value("test-token".into())),
        region: Some("us-west-2".into()),
        base_url: Some("https://bedrock.test".into()),
        fetch: Some(fetch.transport()),
        ..Default::default()
    })
    .unwrap()
}

pub(super) fn options() -> CallOptions {
    CallOptions::new(vec![LanguageModelMessage::user_text("Hello")])
}

pub(super) fn chat_body() -> Value {
    json!({"messages":[{"role":"user","content":[{"text":"Hello"}]}],
        "additionalModelResponseFieldPaths":["/delta/stop_sequence"]})
}

pub(super) fn stream_response(events: Value) -> Canned {
    let mut body = Vec::new();
    for event in events.as_array().unwrap() {
        let (kind, payload) = event.as_object().unwrap().iter().next().unwrap();
        body.extend(encode_message("event", kind, &payload.to_string()));
    }
    Canned {
        status: 200,
        headers: vec![(
            "content-type".into(),
            "application/vnd.amazon.eventstream".into(),
        )],
        body,
    }
}
