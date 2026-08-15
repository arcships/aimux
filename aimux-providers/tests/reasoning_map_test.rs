//! stage2-002（RFC-0017 阶段 2）：退役回归 + max_tokens_key 矩阵 + reasoning_effort 直传。
//!
//! 设计来源：[stage2-reasoning-map.md](../../docs/plan/analysis/stage2-reasoning-map.md) §4 I4-I5、§5。
//!
//! 覆盖：
//! - **I4**：退役后 DeepSeek 请求体不含 `thinking` 注入（除非用户 bodyOverrides 注入）
//! - **I5**：用户 `body_overrides: { thinking: { type: 'disabled' } }` → 请求体含之
//!   （阶段 1 能力回归）
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
use aimux_providers::openai::OpenAICompatProfile;
use aimux_providers::openai::convert::build_request_body_with_warnings;
use aimux_providers::provider_registry_entry;
use serde_json::json;

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
// I4: 退役后 DeepSeek 请求体不含 thinking（除非用户 bodyOverrides）
// ════════════════════════════════════════════════════════════════════════════

/// I4: `reasoning:'none'` 透传为 `reasoning_effort:"none"`，请求体**不含**
/// `thinking` 注入（退役语义——thinking 注入不再由内置特化产生）。
#[test]
fn i4_deepseek_retired_no_thinking_injection() {
    let result = build_request_body_with_warnings(
        "deepseek-reasoner",
        &opts_with_reasoning(ReasoningEffort::None),
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    )
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
    let result = build_request_body_with_warnings(
        "deepseek-reasoner",
        &CallOptions::new(user_prompt()),
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    )
    .unwrap();
    assert!(result.body.get("thinking").is_none());
    assert!(result.body.get("reasoning_effort").is_none());
}

// ════════════════════════════════════════════════════════════════════════════
// I5: 用户 body_overrides 注入 thinking → 请求体含之（阶段 1 能力回归）
// ════════════════════════════════════════════════════════════════════════════

/// I5: 用户 `body_overrides: { thinking: { type: 'disabled' } }` 原样进入请求体。
/// 关思考的语义完全由用户定义（退役后不再由 reasoning:'none' 自动注入）。
#[test]
fn i5_body_overrides_injects_thinking_disabled() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        body_overrides: Some(json!({ "thinking": { "type": "disabled" } })),
        ..CallOptions::default()
    };
    let result = build_request_body_with_warnings(
        "deepseek-reasoner",
        &opts,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    )
    .unwrap();
    assert_eq!(result.body["thinking"], json!({ "type": "disabled" }));
    assert_eq!(result.body["reasoning_effort"], json!("none"));
}

/// I5 补充: 用户注入 `thinking: { type: 'enabled' }` 同样原样进入请求体
/// （开思考回归——退役前由特化注入，现由用户定义）。
#[test]
fn i5_body_overrides_injects_thinking_enabled() {
    let opts = CallOptions {
        prompt: user_prompt(),
        body_overrides: Some(json!({ "thinking": { "type": "enabled" } })),
        ..CallOptions::default()
    };
    let result = build_request_body_with_warnings(
        "deepseek-reasoner",
        &opts,
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
    )
    .unwrap();
    assert_eq!(result.body["thinking"], json!({ "type": "enabled" }));
}

// ════════════════════════════════════════════════════════════════════════════
// max_tokens_key 矩阵：8 家接线 × 推理/非推理两分支
//
// profile 从注册表 `provider_registry_entry(name)` 实际构造取出——直接锁注册表接线
// （若注册表行被改回 full()，此处 profile.max_tokens_key 断言即失败，防死代码）。
// ════════════════════════════════════════════════════════════════════════════

/// 8 家接线清单：(provider 名, 实际 profile, 期望 max_tokens_key)。
fn wired_vendors() -> Vec<(&'static str, OpenAICompatProfile, &'static str)> {
    vec![
        (
            "stepfun",
            provider_registry_entry("stepfun").unwrap(),
            "max_tokens",
        ),
        (
            "siliconflow",
            provider_registry_entry("siliconflow").unwrap(),
            "max_tokens",
        ),
        (
            "sarvam",
            provider_registry_entry("sarvam").unwrap(),
            "max_tokens",
        ),
        (
            "reka_ai",
            provider_registry_entry("reka_ai").unwrap(),
            "max_tokens",
        ),
        (
            "publicai",
            provider_registry_entry("publicai").unwrap(),
            "max_tokens",
        ),
        (
            "perplexity",
            provider_registry_entry("perplexity").unwrap(),
            "max_tokens",
        ),
        (
            "groq",
            provider_registry_entry("groq").unwrap(),
            "max_completion_tokens",
        ),
        (
            "heroku",
            provider_registry_entry("heroku").unwrap(),
            "max_completion_tokens",
        ),
    ]
}

/// 断言单个厂商在指定模型（推理/非推理）下请求体 key 名正确、另一 key 缺席。
fn assert_vendor_key(
    provider: &str,
    profile: &OpenAICompatProfile,
    expected_key: &str,
    model_id: &str,
    branch: &str,
) {
    let result = build_request_body_with_warnings(
        model_id,
        &opts_with_max_tokens(100),
        false,
        provider,
        profile,
    )
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

/// 接线本身：每个厂商 profile.max_tokens_key 与清单一致（防注册表行被改回）。
#[test]
fn max_tokens_key_wiring_registry() {
    for (provider, profile, expected) in wired_vendors() {
        assert_eq!(
            profile.max_tokens_key,
            Some(expected),
            "[{provider}] 注册表接线错误"
        );
    }
}

/// 矩阵 × 推理分支（o3-mini 是推理模型 → 若未接线会推断发 mct，接线后按厂商 key 发）。
#[test]
fn max_tokens_key_matrix_reasoning_branch() {
    for (provider, profile, expected) in wired_vendors() {
        assert_vendor_key(provider, &profile, expected, "o3-mini", "推理");
    }
}

/// 矩阵 × 非推理分支（gpt-4o 非推理 → 若未接线会推断发 max_tokens，接线后按厂商 key 发）。
#[test]
fn max_tokens_key_matrix_non_reasoning_branch() {
    for (provider, profile, expected) in wired_vendors() {
        assert_vendor_key(provider, &profile, expected, "gpt-4o", "非推理");
    }
}

/// 未接线厂商（None）保持现状推断：推理模型发 mct、非推理发 max_tokens
/// （回归护栏——full() 默认不受接线影响）。
#[test]
fn max_tokens_key_none_keeps_inference() {
    let profile = OpenAICompatProfile::full();
    assert_eq!(profile.max_tokens_key, None);

    let reasoning = build_request_body_with_warnings(
        "o3-mini",
        &opts_with_max_tokens(100),
        false,
        "openai",
        &profile,
    )
    .unwrap();
    assert_eq!(reasoning.body["max_completion_tokens"], json!(100));
    assert!(reasoning.body.get("max_tokens").is_none());

    let non_reasoning = build_request_body_with_warnings(
        "gpt-4o",
        &opts_with_max_tokens(100),
        false,
        "openai",
        &profile,
    )
    .unwrap();
    assert_eq!(non_reasoning.body["max_tokens"], json!(100));
    assert!(non_reasoning.body.get("max_completion_tokens").is_none());
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
        let result = build_request_body_with_warnings(
            "deepseek-reasoner",
            &opts_with_reasoning(effort),
            false,
            "deepseek",
            &OpenAICompatProfile::deepseek(),
        )
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
    let default_body = build_request_body_with_warnings(
        "deepseek-reasoner",
        &opts_with_reasoning(ReasoningEffort::ProviderDefault),
        false,
        "deepseek",
        &OpenAICompatProfile::deepseek(),
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
        let result = build_request_body_with_warnings(
            "deepseek-reasoner",
            &opts_with_reasoning(effort),
            false,
            "deepseek",
            &OpenAICompatProfile::deepseek(),
        )
        .unwrap();
        assert_eq!(
            result.body["reasoning_effort"],
            json!(expected),
            "reasoning 档位应原样透传（无归一化）"
        );
    }
}
