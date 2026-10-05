//! stage2-002（RFC-0017 阶段 2）：provider 级 transform + max_tokens_key 矩阵。
//!
//! 设计来源：[stage2-reasoning-map.md](../../docs/plan/analysis/stage2-reasoning-map.md) §4 I4-I5、§5。
//!
//! 覆盖：
//! - **I5**：provider 级 `transform_request_body` 写入 `thinking` → 发出的请求体含之
//!   （声明式 `body_overrides` 与调用级覆盖均已删除）
//! - **max_tokens_key 矩阵**：8 家接线（stepfun/siliconflow/sarvam/reka_ai/publicai/
//!   perplexity → `"max_tokens"`；groq/heroku → `"max_completion_tokens"`）×
//!   推理/非推理两分支。profile 取自注册表 `preset::lookup(name)`
//!   （锁注册表接线，防死代码）。
//!
//! DeepSeek 的 reasoning 映射（thinking / reasoning_effort）对齐 `@ai-sdk/deepseek`，其用例见
//! `deepseek_chat_test.rs` 的 `top-level reasoning`。

use aimux_core::language_model_message::{LanguageModelMessage, LanguageModelPrompt};
use aimux_core::options::CallOptions;
use aimux_core::types::ReasoningEffort;
use aimux_providers::PresetSettings;
use aimux_providers::deepseek::{DeepSeekProviderSettings, create_deepseek};
use aimux_providers::openai_compatible::OpenAICompatibleChatModel;
use serde_json::json;

/// The chat model of a registry preset, built the way the registry builds it.
fn preset_chat(name: &str, model_id: &str) -> OpenAICompatibleChatModel {
    aimux_providers::preset::create(name, PresetSettings::default())
        .unwrap()
        .chat(model_id)
}

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelMessage::user_text("Hello")]
}

fn opts_with_max_tokens(n: u32) -> CallOptions {
    CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(n),
        ..CallOptions::default()
    }
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

// ════════════════════════════════════════════════════════════════════════════
// max_tokens_key 矩阵：8 家接线 × 推理/非推理两分支
//
// profile 从注册表 `preset::lookup(name)` 实际构造取出——直接锁注册表接线
// （若注册表行被改回 full()，此处 profile.max_tokens_key 断言即失败，防死代码）。
// ════════════════════════════════════════════════════════════════════════════

/// 7 家接线清单：(provider 名, 期望 max_tokens_key)。
fn wired_vendors() -> Vec<(&'static str, &'static str)> {
    vec![
        ("stepfun", "max_tokens"),
        ("siliconflow", "max_tokens"),
        ("sarvam", "max_tokens"),
        ("reka_ai", "max_tokens"),
        ("publicai", "max_tokens"),
        ("perplexity", "max_tokens"),
        ("heroku", "max_completion_tokens"),
    ]
}

/// 断言单个厂商在指定模型（推理/非推理）下请求体 key 名正确、另一 key 缺席。
fn assert_vendor_key(provider: &str, expected_key: &str, model_id: &str, branch: &str) {
    let result = preset_chat(provider, model_id)
        .request_body(&opts_with_max_tokens(100), false)
        .unwrap();
    assert_eq!(
        result.body[expected_key],
        json!(100),
        "[{provider}] {branch} 分支: 请求体应含 {expected_key}"
    );
    let other_key = if expected_key == "max_tokens" {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    assert!(
        result.body.get(other_key).is_none(),
        "[{}] {} 分支: 不应含 {}（只认 {}）: {:?}",
        provider,
        branch,
        other_key,
        expected_key,
        result.body
    );
}

/// 接线本身：注册表行的 max_tokens_key 与清单一致（防注册表行被改回）。
#[test]
fn max_tokens_key_wiring_registry() {
    for (provider, expected) in wired_vendors() {
        let descriptor = aimux_providers::preset::lookup(provider)
            .unwrap()
            .descriptor;
        assert_eq!(
            descriptor.max_tokens_key,
            Some(expected),
            "[{provider}] 注册表接线错误"
        );
    }
}

/// 矩阵 × 推理模型名（兼容包不按模型名推断，按厂商 key 发）。
#[test]
fn max_tokens_key_matrix_reasoning_branch() {
    for (provider, expected) in wired_vendors() {
        assert_vendor_key(provider, expected, "o3-mini", "推理");
    }
}

/// 矩阵 × 非推理模型名。
#[test]
fn max_tokens_key_matrix_non_reasoning_branch() {
    for (provider, expected) in wired_vendors() {
        assert_vendor_key(provider, expected, "gpt-4o", "非推理");
    }
}

/// 未接线厂商（None）发 `max_tokens`（AI SDK 兼容基线），与模型名无关
/// （回归护栏——兼容包不做 OpenAI 原生的 mct 推断）。
#[test]
fn max_tokens_key_none_sends_max_tokens() {
    for model_id in ["o3-mini", "gpt-4o"] {
        let result = preset_chat("abacus", model_id)
            .request_body(&opts_with_max_tokens(100), false)
            .unwrap();
        assert_eq!(result.body["max_tokens"], json!(100));
        assert!(result.body.get("max_completion_tokens").is_none());
    }
}
