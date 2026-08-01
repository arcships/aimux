//! Tests for RFC-0017 phase 1: `body_overrides` (JSON deep-merge) and
//! `max_retries` (per-call retry override).
//!
//! `body_overrides` is a per-call JSON object deep-merged into the
//! provider-built request body after built-in vendor overrides. It lets users
//! inject/override arbitrary request fields (e.g. `enable_thinking`,
//! `thinking_budget`) without closure bridging — critical for aimux's
//! multi-language C ABI architecture. `null` values delete keys.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_providers::openai::convert::build_request_body;
use serde_json::{Value, json};

fn user_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}

fn opts_with_overrides(prompt: LanguageModelPrompt, body_overrides: Value) -> CallOptions {
    CallOptions {
        prompt,
        body_overrides: Some(body_overrides),
        ..CallOptions::default()
    }
}

// ── body_overrides: inject ───────────────────────────────────────────────────

/// A top-level key in body_overrides is injected into the request body.
#[test]
fn body_overrides_injects_new_field() {
    let opts = opts_with_overrides(user_prompt(), json!({ "enable_thinking": false }));
    let body = build_request_body("gpt-4o", &opts, false);
    assert_eq!(body["enable_thinking"], json!(false));
    assert_eq!(body["model"], json!("gpt-4o"));
}

/// body_overrides can inject nested objects.
#[test]
fn body_overrides_injects_nested_object() {
    let opts = opts_with_overrides(user_prompt(), json!({ "thinking": { "type": "disabled" } }));
    let body = build_request_body("gpt-4o", &opts, false);
    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
}

// ── body_overrides: override ─────────────────────────────────────────────────

/// body_overrides overwrites an existing field (e.g. temperature).
#[test]
fn body_overrides_overwrites_existing_field() {
    let mut opts = opts_with_overrides(user_prompt(), json!({ "temperature": 0.1 }));
    opts.temperature = Some(0.9); // set by standard option
    let body = build_request_body("gpt-4o", &opts, false);
    // body_overrides wins over standard option
    assert_eq!(body["temperature"], json!(0.1));
}

/// body_overrides overwrites a field set by a built-in vendor override
/// (DeepSeek thinking). This proves the merge runs after vendor overrides.
#[test]
fn body_overrides_overwrites_vendor_override_field() {
    let opts = CallOptions {
        prompt: user_prompt(),
        reasoning: Some(aimux_core::types::ReasoningEffort::None),
        body_overrides: Some(json!({ "thinking": { "type": "enabled" } })),
        ..CallOptions::default()
    };
    // DeepSeek profile: reasoning:none → thinking:{type:"disabled"}
    // body_overrides should override to enabled.
    let body = aimux_providers::openai::convert::build_request_body_with_warnings(
        "deepseek-v4-flash",
        &opts,
        false,
        "deepseek",
        &aimux_providers::openai::OpenAICompatProfile::deepseek(),
    )
    .body;
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
}

// ── body_overrides: deep merge ───────────────────────────────────────────────

/// Nested objects are merged recursively, not replaced wholesale.
#[test]
fn body_overrides_deep_merges_nested_objects() {
    // The standard body has stream_options: { include_usage: true } for streams.
    // body_overrides adds another key to stream_options without clobbering
    // include_usage.
    let opts = opts_with_overrides(
        user_prompt(),
        json!({ "stream_options": { "include_usage": false } }),
    );
    let body = build_request_body("gpt-4o", &opts, true);
    assert_eq!(body["stream_options"]["include_usage"], json!(false));
}

// ── body_overrides: null = delete ────────────────────────────────────────────

/// A `null` value in body_overrides deletes the corresponding key from the
/// request body.
#[test]
fn body_overrides_null_deletes_key() {
    let opts = opts_with_overrides(
        user_prompt(),
        json!({ "stream_options": null, "temperature": null }),
    );
    let body = build_request_body("gpt-4o", &opts, true);
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
    let opts = opts_with_overrides(
        user_prompt(),
        json!({ "stream_options": { "include_usage": null } }),
    );
    let body = build_request_body("gpt-4o", &opts, true);
    // stream_options still exists but include_usage is gone
    assert!(body.get("stream_options").is_some());
    assert!(body["stream_options"].get("include_usage").is_none());
}

// ── body_overrides: no overrides = unchanged ─────────────────────────────────

/// When body_overrides is None, the request body is identical to the standard
/// build (no merge happens).
#[test]
fn no_body_overrides_leaves_body_unchanged() {
    let opts = CallOptions::new(user_prompt());
    let body = build_request_body("gpt-4o", &opts, false);
    assert_eq!(body["model"], json!("gpt-4o"));
    assert!(body.get("enable_thinking").is_none());
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
