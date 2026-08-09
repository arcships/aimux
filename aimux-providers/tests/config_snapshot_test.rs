//! RFC-0023 `config_snapshot` / `api_key_source` coverage (findings C2/C3/C6).
//!
//! These are pure construction-time checks (no HTTP): build a config the way
//! real callers do (`from_env` / `new` / registry `provider()`), then assert
//! the recorded `ProviderRecord` carries the right provider identity and
//! api-key *source* (never the plaintext key).

use aimux_core::language_model::LanguageModel;

use aimux_providers::provider;
use aimux_providers::{
    AnthropicConfig, AnthropicProvider, AzureConfig, AzureModel, CohereConfig, CohereProvider,
    CodexConfig, CodexProvider, GoogleConfig, GoogleProvider, MistralConfig, MistralProvider,
};

// ── C2: OpenAI-compatible chat path surfaces the registry provider name ─────

/// A registry-built deepseek model records `provider == "deepseek"` (not the
/// old hardcoded `"openai"`). The Chat path must use `config.provider`.
#[test]
fn registry_chat_snapshot_records_real_provider() {
    let model = provider("deepseek", Some("sk-test".into()), "deepseek-chat", None)
        .expect("deepseek constructs via registry");
    let snap = model.config_snapshot();
    assert_eq!(snap.provider, "deepseek", "snapshot must carry the registry name");
    assert_eq!(snap.model_id, "deepseek-chat");
    assert_eq!(model.provider(), "deepseek");
    // Registry `provider(name, None, ..)` reads the env var → env source.
    assert!(
        snap.api_key_source == "explicit" || snap.api_key_source.starts_with("env:"),
        "api_key_source should be explicit (key given) or env:, got {}",
        snap.api_key_source
    );
}

/// Explicit key → `explicit`; no plaintext leaks into the snapshot.
#[test]
fn registry_chat_explicit_key_source() {
    let model = provider("groq", Some("sk-test".into()), "llama-3.3-70b", None).unwrap();
    let snap = model.config_snapshot();
    assert_eq!(snap.api_key_source, "explicit");
    let json = serde_json::to_string(&snap).unwrap();
    assert!(!json.contains("sk-test"), "plaintext key leaked: {json}");
}

// ── C3: native providers record env vs explicit api_key_source ──────────────

/// Google `from_env` → `env:GOOGLE_GENERATIVE_AI_API_KEY`; `new` → `explicit`.
#[test]
fn google_api_key_source_env_vs_explicit() {
    let explicit = GoogleConfig::new("g-explicit");
    let snap = GoogleProvider::new(explicit).model("gemini-2.0-flash").config_snapshot();
    assert_eq!(snap.provider, "google.generative-ai");
    assert_eq!(snap.api_key_source, "explicit");

    // from_env requires the env var; set it so construction succeeds.
    unsafe { std::env::set_var("GOOGLE_GENERATIVE_AI_API_KEY", "g-env") };
    let env_cfg = GoogleConfig::from_env().expect("env var set");
    unsafe { std::env::remove_var("GOOGLE_GENERATIVE_AI_API_KEY") };
    let snap = GoogleProvider::new(env_cfg).model("gemini-2.0-flash").config_snapshot();
    assert_eq!(
        snap.api_key_source, "env:GOOGLE_GENERATIVE_AI_API_KEY",
        "from_env must mark the env source"
    );
    let json = serde_json::to_string(&snap).unwrap();
    assert!(!json.contains("g-env"), "plaintext key leaked: {json}");
}

/// Anthropic `from_env` → `env:ANTHROPIC_API_KEY`; `new` → `explicit`. Also
/// covers C6: the snapshot's `provider_options` is a ProviderOptions-shaped
/// object carrying `headers`/`api_version`/`max_retries`/`body_overrides`.
#[test]
fn anthropic_api_key_source_and_snapshot_shape() {
    let explicit = AnthropicConfig::new("a-explicit");
    let snap = AnthropicProvider::new(explicit)
        .model("claude-3-5-sonnet-20241022")
        .config_snapshot();
    assert_eq!(snap.provider, "anthropic.messages");
    assert_eq!(snap.api_key_source, "explicit");

    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "a-env") };
    let env_cfg = AnthropicConfig::from_env().expect("env var set");
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    let snap = AnthropicProvider::new(env_cfg)
        .model("claude-3-5-sonnet-20241022")
        .config_snapshot();
    assert_eq!(snap.api_key_source, "env:ANTHROPIC_API_KEY");

    // C6: provider_options shape (not a bare headers map).
    let opts = snap
        .provider_options
        .as_ref()
        .expect("anthropic snapshot must carry provider_options");
    assert!(opts.get("headers").is_some(), "missing headers: {opts}");
    assert_eq!(opts["api_version"], "2023-06-01");
    assert!(opts.get("max_retries").is_some(), "missing max_retries: {opts}");
    assert!(opts.get("body_overrides").is_some(), "missing body_overrides: {opts}");

    let json = serde_json::to_string(&snap).unwrap();
    assert!(!json.contains("a-env"), "plaintext key leaked: {json}");
}

/// Mistral `from_env` → `env:MISTRAL_API_KEY`.
#[test]
fn mistral_api_key_source_env() {
    unsafe { std::env::set_var("MISTRAL_API_KEY", "m-env") };
    let cfg = MistralConfig::from_env().expect("env var set");
    unsafe { std::env::remove_var("MISTRAL_API_KEY") };
    let snap = MistralProvider::new(cfg).model("mistral-small-latest").config_snapshot();
    assert_eq!(snap.provider, "mistral");
    assert_eq!(snap.api_key_source, "env:MISTRAL_API_KEY");
}

/// Cohere `from_env` → `env:COHERE_API_KEY`.
#[test]
fn cohere_api_key_source_env() {
    unsafe { std::env::set_var("COHERE_API_KEY", "c-env") };
    let cfg = CohereConfig::from_env().expect("env var set");
    unsafe { std::env::remove_var("COHERE_API_KEY") };
    let snap = CohereProvider::new(cfg).model("command-r-plus").config_snapshot();
    assert_eq!(snap.provider, "cohere");
    assert_eq!(snap.api_key_source, "env:COHERE_API_KEY");
}

/// Azure `from_env` → `env:AZURE_API_KEY`; explicit api-key → `explicit`.
#[test]
fn azure_api_key_source_env_vs_explicit() {
    let cfg = AzureConfig::new()
        .with_resource_name("my-resource")
        .with_api_key("az-explicit");
    let snap = AzureModel::new("gpt-4o".into(), cfg).config_snapshot();
    assert_eq!(snap.provider, "azure");
    assert_eq!(snap.api_key_source, "explicit");

    unsafe {
        std::env::set_var("AZURE_API_KEY", "az-env");
        std::env::set_var("AZURE_RESOURCE_NAME", "res");
    }
    let cfg = AzureConfig::from_env().expect("env var set");
    unsafe {
        std::env::remove_var("AZURE_API_KEY");
        std::env::remove_var("AZURE_RESOURCE_NAME");
    }
    let snap = AzureModel::new("gpt-4o".into(), cfg).config_snapshot();
    assert_eq!(snap.api_key_source, "env:AZURE_API_KEY");
    let json = serde_json::to_string(&snap).unwrap();
    assert!(!json.contains("az-env"), "plaintext key leaked: {json}");
}

/// Codex `from_env` → `env:CODEX_API_KEY`; `new` → `explicit`.
#[test]
fn codex_api_key_source_env_vs_explicit() {
    let snap = CodexProvider::new(CodexConfig::new("sk-test"))
        .model("gpt-5.2-codex")
        .config_snapshot();
    assert_eq!(snap.provider, "codex");
    assert_eq!(snap.api_key_source, "explicit");

    unsafe { std::env::set_var("CODEX_API_KEY", "cx-env") };
    let cfg = CodexConfig::from_env().expect("env var set");
    unsafe { std::env::remove_var("CODEX_API_KEY") };
    let snap = CodexProvider::new(cfg)
        .model("gpt-5.2-codex")
        .config_snapshot();
    assert_eq!(snap.api_key_source, "env:CODEX_API_KEY");
}
