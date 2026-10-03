//! Tests for provider-level `body_overrides` (RFC-0017) and `max_retries`
//! (per-call retry override).
//!
//! `body_overrides` is a JSON object configured on the provider and deep-merged
//! into the built request body right before sending (`apply_body_overrides`).
//! `null` values delete keys. There is no per-call override: `CallOptions` no
//! longer carries `body_overrides`.
//!
//! stage2-001 (RFC-0017 phase 2) additions: `max_tokens_key` branch and direct
//! `reasoning` → `reasoning_effort` passthrough. The old "reasoning no-mapping"
//! warning block was removed (F6): under v3 direct-passthrough semantics it is
//! unreachable dead code (is_custom_reasoning ⇒ resolved effort is always Some),
//! and the passthrough tests below now guard against re-adding it.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::types::{ReasoningEffort, Warning};
use aimux_providers::body_merge::apply_body_overrides;
use aimux_providers::deepseek::deepseek;
use aimux_providers::groq::groq;
use aimux_providers::openai::convert::build_request_body;
use aimux_providers::openai_compatible::{
    OpenAICompatibleProviderSettings, create_openai_compatible,
};
use aimux_providers::presets;
use serde_json::{Value, json};

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

/// The request body the OpenAI chat converter builds, with the provider-level
/// overrides applied the way `execute_generate` / `execute_stream` do.
fn body_with_overrides(options: &CallOptions, stream: bool, overrides: Value) -> Value {
    let mut body = build_request_body("gpt-4o", options, stream).unwrap();
    apply_body_overrides(&mut body, Some(&overrides));
    body
}

// ── body_overrides: inject ───────────────────────────────────────────────────

/// A top-level key in body_overrides is injected into the request body.
#[test]
fn body_overrides_injects_new_field() {
    let opts = CallOptions::new(user_prompt());
    let body = body_with_overrides(&opts, false, json!({ "enable_thinking": false }));
    assert_eq!(body["enable_thinking"], json!(false));
    assert_eq!(body["model"], json!("gpt-4o"));
}

/// body_overrides can inject nested objects.
#[test]
fn body_overrides_injects_nested_object() {
    let opts = CallOptions::new(user_prompt());
    let body = body_with_overrides(&opts, false, json!({ "thinking": { "type": "disabled" } }));
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
}

// ── body_overrides: override ─────────────────────────────────────────────────

/// body_overrides overwrites an existing field (e.g. temperature).
#[test]
fn body_overrides_overwrites_existing_field() {
    let mut opts = CallOptions::new(user_prompt());
    opts.temperature = Some(0.9); // set by standard option
    let body = body_with_overrides(&opts, false, json!({ "temperature": 0.1 }));
    // body_overrides wins over standard option
    assert_eq!(body["temperature"], json!(0.1));
}

/// body_overrides injects a field into the request body (RFC-0017).
/// stage2-001（RFC-0017 阶段 2）后内置 vendor override 已退役——此前 DeepSeek
/// 会从 `reasoning:none` 注入 `thinking:{type:"disabled"}`;现在 thinking 注入
/// 完全由 body_overrides 定义（此处直接注入 enabled）。
#[test]
fn body_overrides_overwrites_vendor_override_field() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(aimux_core::types::ReasoningEffort::None),
        ..CallOptions::default()
    };
    // DeepSeek 的 thinking 注入由 body_overrides 定义,不是内置特化。
    let mut body = deepseek()
        .chat("deepseek-v4-flash")
        .request_body(&opts, false)
        .unwrap()
        .body;
    apply_body_overrides(
        &mut body,
        Some(&json!({ "thinking": { "type": "enabled" } })),
    );
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
}

// ── body_overrides: deep merge ───────────────────────────────────────────────

/// Nested objects are merged recursively, not replaced wholesale.
#[test]
fn body_overrides_deep_merges_nested_objects() {
    // The standard body has stream_options: { include_usage: true } for streams.
    // body_overrides adds another key to stream_options without clobbering
    // include_usage.
    let opts = CallOptions::new(user_prompt());
    let body = body_with_overrides(
        &opts,
        true,
        json!({ "stream_options": { "include_usage": false, "extra": 1 } }),
    );
    assert_eq!(body["stream_options"]["include_usage"], json!(false));
    assert_eq!(body["stream_options"]["extra"], json!(1));
}

// ── body_overrides: null = delete ────────────────────────────────────────────

/// A `null` value in body_overrides deletes the corresponding key from the
/// request body.
#[test]
fn body_overrides_null_deletes_key() {
    let mut opts = CallOptions::new(user_prompt());
    opts.temperature = Some(0.5);
    let body = body_with_overrides(
        &opts,
        true,
        json!({ "stream_options": null, "temperature": null }),
    );
    assert!(
        body.get("stream_options").is_none(),
        "stream_options should be deleted by null"
    );
    assert!(
        body.get("temperature").is_none(),
        "temperature should be deleted by null"
    );
}

/// null in a nested object deletes the nested key.
#[test]
fn body_overrides_null_deletes_nested_key() {
    let opts = CallOptions::new(user_prompt());
    let body = body_with_overrides(
        &opts,
        true,
        json!({ "stream_options": { "include_usage": null } }),
    );
    // stream_options still exists but include_usage is gone
    assert!(body.get("stream_options").is_some());
    assert!(body["stream_options"].get("include_usage").is_none());
}

// ── body_overrides: no overrides = unchanged ─────────────────────────────────

/// With no overrides, the request body is identical to the standard build.
#[test]
fn no_body_overrides_leaves_body_unchanged() {
    let opts = CallOptions::new(user_prompt());
    let standard = build_request_body("gpt-4o", &opts, false).unwrap();
    let mut applied = standard.clone();
    apply_body_overrides(&mut applied, None);
    assert_eq!(applied, standard);
    assert!(applied.get("enable_thinking").is_none());
}

// ── body_overrides: provider-level, end to end ───────────────────────────────

/// The compat package expresses a provider-level body rewrite as a
/// `transform_request_body` closure; `CallOptions` has no override of its own.
#[tokio::test]
async fn provider_body_overrides_reach_the_request() {
    use aimux_core::language_model::LanguageModel;
    use aimux_provider_utils::Resolvable;
    use aimux_providers::body_merge::deep_merge_json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "hi" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let overrides = json!({ "enable_thinking": false, "temperature": null });
    let model = create_openai_compatible(OpenAICompatibleProviderSettings {
        name: "acme".to_string(),
        base_url: server.uri(),
        api_key: Some(Resolvable::Value("test-key".to_string())),
        transform_request_body: Some(std::sync::Arc::new(move |mut body: Value| {
            deep_merge_json(&mut body, &overrides);
            body
        })),
        ..Default::default()
    })
    .unwrap()
    .chat("gpt-4o");
    let mut options = CallOptions::new(user_prompt());
    options.temperature = Some(0.7);
    model.do_generate(&options).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["enable_thinking"], json!(false));
    assert!(
        sent.get("temperature").is_none(),
        "null deletes the standard field: {sent}"
    );
}

/// The native package expresses the same provider-level need as a
/// `transform_request_body` closure: called once per request, after the body
/// is serialized, for `do_generate` and `do_stream` alike.
#[tokio::test]
async fn native_transform_request_body_reaches_the_request() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aimux_core::language_model::LanguageModel;
    use aimux_provider_utils::Resolvable;
    use aimux_providers::body_merge::deep_merge_json;
    use aimux_providers::openai::{OpenAIProviderSettings, create_openai};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1711115037,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "hi" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri()),
        transform_request_body: Some(Arc::new(move |mut body: Value| {
            counter.fetch_add(1, Ordering::SeqCst);
            deep_merge_json(
                &mut body,
                &json!({ "enable_thinking": false, "temperature": null }),
            );
            body
        })),
        ..Default::default()
    })
    .unwrap();
    let model = provider.chat("gpt-4o");
    let mut options = CallOptions::new(user_prompt());
    options.temperature = Some(0.7);
    model.do_generate(&options).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "once per request");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["enable_thinking"], json!(false));
    assert!(sent.get("temperature").is_none(), "{sent}");
}

// ── max_retries ──────────────────────────────────────────────────────────────

/// max_retries is stored in CallOptions and defaults to None.
#[test]
fn max_retries_defaults_to_none() {
    let opts = CallOptions::new(user_prompt());
    assert!(opts.max_retries.is_none());
}

/// max_retries can be set (e.g. Some(0) to disable retries).
#[test]
fn max_retries_can_be_set() {
    let opts = CallOptions {
        prompt: user_prompt(),
        max_retries: Some(0),
        ..CallOptions::default()
    };
    assert_eq!(opts.max_retries, Some(0));
}

// ── max_tokens_key (stage2-001, RFC-0017 phase 2 §2.3) ──────────────────────

/// 注册表里 `max_tokens_key: "max_tokens"` 的厂商(stepfun)只发 `max_tokens`,
/// 即使模型名像推理模型(兼容包不按模型名推断)。
#[test]
fn max_tokens_key_max_tokens_reasoning_model() {
    let opts = CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(100),
        ..CallOptions::default()
    };
    let body = presets::stepfun()
        .chat("o4-mini")
        .request_body(&opts, false)
        .unwrap()
        .body;
    assert_eq!(body["max_tokens"], json!(100));
    assert!(
        body.get("max_completion_tokens").is_none(),
        "只认 max_tokens 的厂商不应收到 max_completion_tokens"
    );
}

/// `max_tokens_key: "max_completion_tokens"`(heroku、groq:max_tokens 弃用)→
/// 非推理模型也发 mct。
#[test]
fn max_tokens_key_max_completion_tokens_non_reasoning() {
    let opts = CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(100),
        ..CallOptions::default()
    };
    for chat in [
        presets::heroku().chat("gpt-4o"),
        groq().chat("llama-3.3-70b-versatile"),
    ] {
        let body = chat.request_body(&opts, false).unwrap().body;
        assert_eq!(body["max_completion_tokens"], json!(100));
        assert!(
            body.get("max_tokens").is_none(),
            "max_tokens_key=mct 时不应再发 max_tokens"
        );
    }
}

/// 没有 `max_tokens_key` 的兼容厂商发 `max_tokens`(AI SDK 基线)。
#[test]
fn compat_without_max_tokens_key_sends_max_tokens() {
    let opts = CallOptions {
        prompt: user_prompt(),
        max_output_tokens: Some(100),
        ..CallOptions::default()
    };
    let body = presets::abacus()
        .chat("o4-mini")
        .request_body(&opts, false)
        .unwrap()
        .body;
    assert_eq!(body["max_tokens"], json!(100));
    assert!(body.get("max_completion_tokens").is_none());
}

// ── reasoning 直传: 无 warning（F6: 旧"无映射提示"warning 块已删除）────────────
//
// v3 直传语义下 `reasoning` 一律映射为 `reasoning_effort`,warning 块不可达已被
// 删除。以下两个用例断言"无 reasoning warning",作为未来误加 warning 的回归护栏。

/// 直传路径: groq 不再特化归一化(`none` 原样透传 'none')→ 已发 effort,不 warning。
#[test]
fn groq_none_passthrough_no_warning() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        ..CallOptions::default()
    };
    let result = groq()
        .chat("llama-3.3-70b-versatile")
        .request_body(&opts, false)
        .unwrap();
    assert_eq!(result.body["reasoning_effort"], json!("none"));
    let reasoning_warning = result
        .warnings
        .iter()
        .find(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"));
    assert!(
        reasoning_warning.is_none(),
        "已发 effort 时不应 warning(防误报): {:?}",
        result.warnings
    );
}

/// 已发 effort 路径（'none' 透传,OpenAI 有效）→ 不 warning（防误报）。
#[test]
fn no_warning_when_reasoning_translated() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        ..CallOptions::default()
    };
    let result = presets::abacus()
        .chat("deepseek-reasoner")
        .request_body(&opts, false)
        .unwrap();
    assert_eq!(result.body["reasoning_effort"], json!("none"));
    let reasoning_warning = result
        .warnings
        .iter()
        .find(|w| matches!(w, Warning::Compatibility { feature, .. } if feature == "reasoning"));
    assert!(
        reasoning_warning.is_none(),
        "已发 effort 时不应 warning(防误报): {:?}",
        result.warnings
    );
}
