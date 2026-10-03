//! Provider-level request-body rewrites (`transform_request_body`) and the
//! call-level `max_retries` default.
//!
//! A provider that needs to change what the converter produced passes a
//! `transform_request_body` closure in its settings. The closure runs once per
//! request on the finished JSON body (so it sees `stream_options`, the vendor
//! `max_tokens` key, and so on) and its result is what goes on the wire. There
//! is no declarative override any more, at provider or call level.
//!
//! stage2-001 (RFC-0017 phase 2) additions: `max_tokens_key` branch and direct
//! `reasoning` → `reasoning_effort` passthrough. The old "reasoning no-mapping"
//! warning block was removed (F6): under v3 direct-passthrough semantics it is
//! unreachable dead code (is_custom_reasoning ⇒ resolved effort is always Some),
//! and the passthrough tests below guard against re-adding it.

use std::sync::{Arc, Mutex};

use aimux_core::content::ContentPart;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_core::types::{ReasoningEffort, Warning};
use aimux_provider_utils::Resolvable;
use aimux_providers::deepseek::deepseek;
use aimux_providers::groq::groq;
use aimux_providers::openai::{OpenAIProviderSettings, create_openai};
use aimux_providers::openai_compatible::{
    OpenAICompatibleProviderSettings, create_openai_compatible,
};
use aimux_providers::presets;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type Transform = Arc<dyn Fn(Value) -> Value + Send + Sync>;

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn completion_response() -> Value {
    json!({
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
    })
}

async fn mock_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion_response()))
        .mount(&server)
        .await;
    server
}

/// The one JSON body the mock server received.
async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "exactly one request");
    serde_json::from_slice(&requests[0].body).unwrap()
}

/// Which package a transform is installed on.
#[derive(Clone, Copy, Debug)]
enum Package {
    Compat,
    Native,
}

/// Send one request through `package` with `transform` installed and return
/// the body that reached the wire. `stream` only decides which entry point
/// builds the body; the mock answers with a plain completion either way.
async fn wire_body(
    package: Package,
    transform: Transform,
    options: &CallOptions,
    stream: bool,
) -> Value {
    let server = mock_server().await;
    let model: Arc<dyn LanguageModel> = match package {
        Package::Compat => Arc::new(
            create_openai_compatible(OpenAICompatibleProviderSettings {
                name: "acme".to_string(),
                base_url: server.uri(),
                api_key: Some(Resolvable::Value("test-key".to_string())),
                transform_request_body: Some(transform),
                ..Default::default()
            })
            .unwrap()
            .chat("gpt-4o"),
        ),
        Package::Native => Arc::new(
            create_openai(OpenAIProviderSettings {
                api_key: Some(Resolvable::Value("test-key".to_string())),
                base_url: Some(server.uri()),
                transform_request_body: Some(transform),
                ..Default::default()
            })
            .unwrap()
            .chat("gpt-4o"),
        ),
    };
    if stream {
        // The body is what is under test; the plain-JSON answer is not a valid
        // event stream and that is fine.
        let _ = model.do_stream(options).await;
    } else {
        model.do_generate(options).await.unwrap();
    }
    sent_body(&server).await
}

const BOTH: [Package; 2] = [Package::Compat, Package::Native];

// ── inject / overwrite / delete ──────────────────────────────────────────────

/// A transform can add a top-level key and a nested object.
#[tokio::test]
async fn transform_injects_fields() {
    for package in BOTH {
        let body = wire_body(
            package,
            Arc::new(|mut body| {
                body["enable_thinking"] = json!(false);
                body["thinking"] = json!({ "type": "disabled" });
                body
            }),
            &CallOptions::new(user_prompt()),
            false,
        )
        .await;
        assert_eq!(body["enable_thinking"], json!(false), "{package:?}");
        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert_eq!(body["model"], json!("gpt-4o"));
    }
}

/// A transform overwrites a field the converter set from a standard option.
#[tokio::test]
async fn transform_overwrites_converter_fields() {
    for package in BOTH {
        let mut opts = CallOptions::new(user_prompt());
        opts.temperature = Some(0.9);
        let body = wire_body(
            package,
            Arc::new(|mut body| {
                body["temperature"] = json!(0.1);
                body
            }),
            &opts,
            false,
        )
        .await;
        assert_eq!(body["temperature"], json!(0.1), "{package:?}");
    }
}

/// A transform removes keys, nested ones (inside a message) included.
#[tokio::test]
async fn transform_deletes_fields() {
    for package in BOTH {
        let mut opts = CallOptions::new(user_prompt());
        opts.temperature = Some(0.5);
        let body = wire_body(
            package,
            Arc::new(|mut body| {
                body.as_object_mut().unwrap().remove("temperature");
                body["messages"][0]
                    .as_object_mut()
                    .expect("a message object")
                    .remove("role");
                body
            }),
            &opts,
            false,
        )
        .await;
        assert!(body.get("temperature").is_none(), "{package:?}: {body}");
        assert!(body["messages"][0].get("role").is_none(), "{body}");
        assert!(body["messages"][0].get("content").is_some());
    }
}

/// The transform sees the finished body (here a streaming one, `stream: true`)
/// and its result is what is sent. It runs once per request.
#[tokio::test]
async fn transform_runs_once_on_the_finished_body() {
    for package in BOTH {
        let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
        let recorder = seen.clone();
        let body = wire_body(
            package,
            Arc::new(move |body| {
                recorder.lock().unwrap().push(body.clone());
                body
            }),
            &CallOptions::new(user_prompt()),
            true,
        )
        .await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{package:?}: once per request");
        assert_eq!(seen[0], body, "an identity transform changes nothing");
        assert_eq!(body["stream"], json!(true), "{package:?}");
    }
}

/// Without a transform the body is exactly what the converter built.
#[tokio::test]
async fn no_transform_leaves_the_body_unchanged() {
    let server = mock_server().await;
    let model = create_openai(OpenAIProviderSettings {
        api_key: Some(Resolvable::Value("test-key".to_string())),
        base_url: Some(server.uri()),
        ..Default::default()
    })
    .unwrap()
    .chat("gpt-4o");
    let opts = CallOptions::new(user_prompt());
    model.do_generate(&opts).await.unwrap();
    let sent = sent_body(&server).await;
    assert_eq!(sent["model"], json!("gpt-4o"));
    assert!(sent.get("enable_thinking").is_none());
}

/// DeepSeek's `thinking` switch is no longer a built-in specialisation; a
/// provider that wants it states it in a transform. The converter alone never
/// produces it.
#[test]
fn deepseek_converter_has_no_thinking_without_a_transform() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(ReasoningEffort::None),
        ..CallOptions::default()
    };
    let body = deepseek()
        .chat("deepseek-v4-flash")
        .request_body(&opts, false)
        .unwrap()
        .body;
    assert!(body.get("thinking").is_none());
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
