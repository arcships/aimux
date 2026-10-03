//! stage2-002（RFC-0017 阶段 2）：退役回归 + max_tokens_key 矩阵 + reasoning_effort 直传。
//!
//! 设计来源：[stage2-reasoning-map.md](../../docs/plan/analysis/stage2-reasoning-map.md) §4 I4-I5、§5。
//!
//! 覆盖：
//! - **I4**：退役后 DeepSeek 请求体不含 `thinking` 注入（除非 provider 级
//!   `transform_request_body` 注入）
//! - **I5**：provider 级 `transform_request_body` 写入 `thinking` → 发出的请求体含之
//!   （声明式 `body_overrides` 与调用级覆盖均已删除）
//! - **max_tokens_key 矩阵**：8 家接线（stepfun/siliconflow/sarvam/reka_ai/publicai/
//!   perplexity → `"max_tokens"`；groq/heroku → `"max_completion_tokens"`）×
//!   推理/非推理两分支。profile 取自注册表 `provider_registry_entry(name)`
//!   （锁注册表接线，防死代码）。
//! - **无 warning 断言**：直传语义下 reasoning 不再产生"未翻译"warning（防未来误加）。
//! - **reasoning_effort 直传**：7 档无归一化（none/minimal/low/medium/high/xhigh 原样
//!   透传；provider-default 不发字段）。

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::types::{ReasoningEffort, Warning};
use aimux_providers::deepseek::{DeepSeekProviderSettings, create_deepseek, deepseek};
use aimux_providers::openai_compatible::OpenAICompatibleChatModel;
use aimux_providers::{PresetFamily, PresetSettings, provider_registry_entry};
use serde_json::json;

/// The chat model of a registry preset, built the way the registry builds it.
fn preset_chat(name: &str, model_id: &str) -> OpenAICompatibleChatModel {
    let entry = aimux_providers::presets::lookup(name).unwrap();
    (entry.create)(PresetSettings::default())
        .unwrap()
        .chat(model_id)
}

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn opts_with_max_tokens(n: u32) -> CallOptions {
    CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(n),
        ..CallOptions::default()
    }
}

fn opts_with_reasoning(r: ReasoningEffort) -> CallOptions {
    CallOptions {
        prompt: user_prompt(),
        reasoning: Some(r),
        ..CallOptions::default()
    }
}

fn has_reasoning_warning(warnings: &[Warning]) -> bool {
    warnings
        .iter()
        .any(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"))
}

// ════════════════════════════════════════════════════════════════════════════
// I4: 退役后 DeepSeek 请求体不含 thinking（除非 provider 级 transform_request_body）
// ════════════════════════════════════════════════════════════════════════════

/// I4: `reasoning:'none'` 透传为 `reasoning_effort:"none"`，请求体**不含**
/// `thinking` 注入（退役语义——thinking 注入不再由内置特化产生）。
#[test]
fn i4_deepseek_retired_no_thinking_injection() {
    let result = deepseek()
        .chat("deepseek-reasoner")
        .request_body(&opts_with_reasoning(ReasoningEffort::None), false)
        .unwrap();
    assert_eq!(result.body["reasoning_effort"], json!("none"));
    assert!(
        result.body.get("thinking").is_none(),
        "I4: 退役后 DeepSeek 请求体不应含 thinking 注入: {:?}",
        result.body
    );
}

/// I4 补充: 完全不设 reasoning 时同样不含 thinking（依赖 API 默认）。
#[test]
fn i4_deepseek_no_thinking_when_reasoning_unset() {
    let result = deepseek()
        .chat("deepseek-reasoner")
        .request_body(&CallOptions::new(user_prompt()), false)
        .unwrap();
    assert!(result.body.get("thinking").is_none());
    assert!(result.body.get("reasoning_effort").is_none());
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
/// reaches the wire as is. What "thinking off" means is the user's to define
/// (since retirement `reasoning: 'none'` no longer injects it).
#[tokio::test]
async fn i5_transform_injects_thinking_disabled() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        ..CallOptions::default()
    };
    let body = deepseek_wire_body(json!({ "type": "disabled" }), &opts).await;
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    assert_eq!(body["reasoning_effort"], json!("none"));
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
// profile 从注册表 `provider_registry_entry(name)` 实际构造取出——直接锁注册表接线
// （若注册表行被改回 full()，此处 profile.max_tokens_key 断言即失败，防死代码）。
// ════════════════════════════════════════════════════════════════════════════

/// 8 家接线清单：(provider 名, 期望 max_tokens_key)。
fn wired_vendors() -> Vec<(&'static str, &'static str)> {
    vec![
        ("stepfun", "max_tokens"),
        ("siliconflow", "max_tokens"),
        ("sarvam", "max_tokens"),
        ("reka_ai", "max_tokens"),
        ("publicai", "max_tokens"),
        ("perplexity", "max_tokens"),
        ("groq", "max_completion_tokens"),
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

/// 接线本身：注册表行的 max_tokens_key 与清单一致（防注册表行被改回）。groq 行指向
/// groq 包（family），它的 `max_completion_tokens` 由包内方言定义，由下面的矩阵锁住。
#[test]
fn max_tokens_key_wiring_registry() {
    for (provider, expected) in wired_vendors() {
        let descriptor = provider_registry_entry(provider).unwrap();
        if descriptor.family == PresetFamily::Groq {
            assert_eq!(
                descriptor.max_tokens_key, None,
                "[{provider}] 由 groq 包定义"
            );
        } else {
            assert_eq!(
                descriptor.max_tokens_key,
                Some(expected),
                "[{provider}] 注册表接线错误"
            );
        }
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

// ════════════════════════════════════════════════════════════════════════════
// 无 warning 断言（直传语义下无"未翻译"warning——防未来误加）
// ════════════════════════════════════════════════════════════════════════════

/// 直传语义下 `reasoning` 各档一律映射为 `reasoning_effort`，不应产生
/// "reasoning 未翻译/无映射"兼容性 warning（stage2 已删除该死代码块）。
#[test]
fn no_reasoning_warning_on_direct_passthrough() {
    for effort in [
        ReasoningEffort::None,
        ReasoningEffort::Minimal,
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::Xhigh,
    ] {
        let result = deepseek()
            .chat("deepseek-reasoner")
            .request_body(&opts_with_reasoning(effort), false)
            .unwrap();
        assert!(
            !has_reasoning_warning(&result.warnings),
            "effort={} 直传不应产生 reasoning warning: {:?}",
            effort,
            result.warnings
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// reasoning_effort 直传：7 档无归一化
// ════════════════════════════════════════════════════════════════════════════

/// 7 档（provider-default 不发字段；其余 6 档原样透传，含 none/minimal/xhigh，
/// 无 xhigh→max / minimal→low 归一化）。
#[test]
fn reasoning_effort_passthrough_all_seven_levels() {
    // provider-default: 不发 reasoning_effort（非自定义）。
    let default_body = deepseek()
        .chat("deepseek-reasoner")
        .request_body(
            &opts_with_reasoning(ReasoningEffort::ProviderDefault),
            false,
        )
        .unwrap();
    assert!(
        default_body.body.get("reasoning_effort").is_none(),
        "provider-default 不应发 reasoning_effort"
    );

    for (effort, expected) in [
        (ReasoningEffort::None, "none"),
        (ReasoningEffort::Minimal, "minimal"),
        (ReasoningEffort::Low, "low"),
        (ReasoningEffort::Medium, "medium"),
        (ReasoningEffort::High, "high"),
        (ReasoningEffort::Xhigh, "xhigh"),
    ] {
        let result = deepseek()
            .chat("deepseek-reasoner")
            .request_body(&opts_with_reasoning(effort), false)
            .unwrap();
        assert_eq!(
            result.body["reasoning_effort"],
            json!(expected),
            "reasoning 档位应原样透传（无归一化）"
        );
    }
}
