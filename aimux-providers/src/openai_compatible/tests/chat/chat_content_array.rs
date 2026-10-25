use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;

use crate::openai_compatible::{OpenAICompatibleProviderSettings, create_openai_compatible};
use crate::test_support::mock_fetch::{Canned, MockFetch};

fn sse(chunks: &[String]) -> Canned {
    Canned {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body: chunks.concat().into_bytes(),
    }
}

fn data(value: &Value) -> String {
    format!("data: {value}\n\n")
}

fn prompt() -> CallOptions {
    serde_json::from_value(json!({
        "prompt": [
            {
                "role": "user",
                "content": [{ "type": "text", "text": "What is 17 * 23?" }],
            },
        ],
    }))
    .expect("call options")
}

/// `model.doStream({ prompt })` against the mock transport; every event in its
/// JSON form, a stream error as an `error` event.
async fn do_stream(chunks: &[String]) -> Vec<Value> {
    let mock = MockFetch::new(vec![sse(chunks)]);
    let model = create_openai_compatible(OpenAICompatibleProviderSettings {
        base_url: "https://my.api.com/v1".to_string(),
        name: "test-provider".to_string(),
        fetch: Some(mock.transport()),
        ..Default::default()
    })
    .expect("provider")
    .chat("test-model");
    let result = model.do_stream(&prompt()).await.expect("stream");
    let events = result
        .stream
        .map(|part| match part {
            Ok(part) => serde_json::to_value(part).expect("part serializes"),
            Err(error) => json!({ "type": "error", "error": error.to_string() }),
        })
        .collect::<Vec<_>>()
        .await;
    let seen = mock.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://my.api.com/v1/chat/completions");
    events
}

fn of_type(events: &[Value], keep: impl Fn(&str) -> bool) -> Vec<Value> {
    events
        .iter()
        .filter(|event| event["type"].as_str().is_some_and(&keep))
        .cloned()
        .collect()
}

/// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-content-array.test.ts › should stream text and thinking content parts
#[tokio::test]
async fn should_stream_text_and_thinking_content_parts() {
    let events = do_stream(&[
        data(&json!({
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": [
                            {
                                "type": "thinking",
                                "thinking": [{ "type": "text", "text": "Let me think." }],
                            },
                            { "type": "text", "text": "The answer is 391." },
                        ],
                    },
                    "finish_reason": null,
                },
            ],
        })),
        data(&json!({
            "choices": [
                {
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                },
            ],
        })),
        "data: [DONE]\n\n".to_string(),
    ])
    .await;

    assert_eq!(of_type(&events, |t| t == "error"), Vec::<Value>::new());
    assert_eq!(
        of_type(&events, |t| t.ends_with("delta")),
        vec![
            json!({
                "type": "reasoning-delta",
                "id": "reasoning-0",
                "delta": "Let me think.",
            }),
            json!({
                "type": "text-delta",
                "id": "txt-0",
                "delta": "The answer is 391.",
            }),
        ]
    );
}

/// upstream: @ai-sdk/openai-compatible src/chat/openai-compatible-chat-content-array.test.ts › should ignore unknown streamed content parts
#[tokio::test]
async fn should_ignore_unknown_streamed_content_parts() {
    let events = do_stream(&[
        data(&json!({
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": [
                            {
                                "type": "future-part",
                                "text": { "nested": true },
                                "thinking": { "nested": true },
                            },
                            { "type": "text", "text": "The answer is 391." },
                        ],
                    },
                    "finish_reason": null,
                },
            ],
        })),
        data(&json!({
            "choices": [
                {
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                },
            ],
        })),
        "data: [DONE]\n\n".to_string(),
    ])
    .await;

    assert_eq!(of_type(&events, |t| t == "error"), Vec::<Value>::new());
    assert_eq!(
        of_type(&events, |t| t == "text-delta"),
        vec![json!({
            "type": "text-delta",
            "id": "txt-0",
            "delta": "The answer is 391.",
        })]
    );
}
