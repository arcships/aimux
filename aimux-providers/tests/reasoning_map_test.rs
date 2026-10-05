use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::types::ReasoningEffort;
use aimux_providers::deepseek::{DeepSeekProviderSettings, create_deepseek};
use serde_json::json;

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

// ════════════════════════════════════════════════════════════════════════════
// I5: provider 级 transform_request_body 注入 thinking → 发出的请求体含之
// ════════════════════════════════════════════════════════════════════════════

/// Send one non-streaming DeepSeek chat call whose provider rewrites the body
/// with `thinking`, and return the JSON that reached the wire.
async fn deepseek_wire_body(thinking: serde_json::Value, opts: &CallOptions) -> serde_json::Value {
    use aimux_core::language_model::LanguageModel;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c1", "object": "chat.completion", "created": 1, "model": "deepseek-reasoner",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"},
                         "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })))
        .mount(&server)
        .await;
    let model = create_deepseek(DeepSeekProviderSettings {
        base_url: Some(server.uri()),
        api_key: Some(aimux_provider_utils::Resolvable::Value("k".into())),
        transform_request_body: Some(std::sync::Arc::new(move |mut body| {
            body["thinking"] = thinking.clone();
            body
        })),
        ..Default::default()
    })
    .unwrap()
    .chat("deepseek-reasoner");
    model.do_generate(opts).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests[0].body).unwrap()
}

/// I5: a provider-level transform that writes `thinking: { type: 'disabled' }`
/// reaches the wire as is.
#[tokio::test]
async fn i5_transform_injects_thinking_disabled() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        ..CallOptions::default()
    };
    let body = deepseek_wire_body(json!({ "type": "disabled" }), &opts).await;
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    // upstream: `reasoning: 'none'` sends `thinking: disabled` and no effort.
    assert!(body.get("reasoning_effort").is_none());
}

/// I5 补充: `thinking: { type: 'enabled' }` 同样原样进入请求体。
#[tokio::test]
async fn i5_transform_injects_thinking_enabled() {
    let opts = CallOptions {
        prompt: user_prompt(),
        ..CallOptions::default()
    };
    let body = deepseek_wire_body(json!({ "type": "enabled" }), &opts).await;
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
}
