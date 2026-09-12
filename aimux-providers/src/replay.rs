//! RFC-0023 请求回放(P4,层 2):按 `ProviderRecord` 自动重建 provider。
//!
//! 拆层(评审 R4,2026-08-06):自动构造需要 `aimux-providers` 的构造能力,
//! 放这里避免 core→providers→core 循环依赖。core 侧的 `replay_with_model`
//! 是 provider 无关的输入重建 + 重发;本模块负责"按录制配置重建 model"。
//!
//! **能力边界(定稿 R8)**:MVP 仅覆盖 **OpenAI 兼容族**——`LanguageModel::provider()`
//! 返回协议实现名,所有 OpenAI 兼容注册表 provider 都报 `"openai"`,所以按
//! `provider.provider == "openai"` 判定 + 录制的 `base_url` 区分具体厂商。
//! 其他协议(anthropic/google/azure/bedrock/...)明确 `Unsupported`:
//! 调用方需传 model 实例给 [`replay_with_model`](aimux_core::replay::replay_with_model)。
//!
//! api_key 来源(`ProviderRecord.api_key_source`):
//! - `explicit` → 必须传 `api_key`(录制不存明文,隐私强制)。
//! - `env:VAR` → 读该环境变量。
//! - `none`(本地模型)→ `"not-needed"` 占位。
//! - `unknown`(provider 尚未覆盖 `config_snapshot` 时的默认)→ 用传入
//!   `api_key`,未传则报错并提示。

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_core::recording::ProviderRecord;

use crate::openai::{OpenAICompatProfile, OpenAIConfig, OpenAIProvider};
use crate::provider::ProviderOptions;

/// 按 `ProviderRecord` 重建 provider(OpenAI 兼容族,MVP)。
///
/// - `provider.provider != "openai"` → `Unsupported`(R8:不猜测原生协议)。
/// - `base_url` 用录制值(OpenAI 兼容族区分厂商的关键);`provider_options`
///   中的 org/project/headers/body_overrides 一并恢复;`profile` 从录制恢复。
/// - 注意:录制的 headers 已脱敏,真实头无法从此恢复——需要时调用方重建后
///   再通过 `ProviderOptions.headers` 补。
///
/// # Errors
///
/// Returns `Unsupported` for non-OpenAI-compatible recordings,
/// `InvalidArgument` for a missing `base_url`, and key-resolution errors.
pub fn rebuild_provider(
    p: &ProviderRecord,
    api_key: Option<&str>,
) -> Result<Box<dyn LanguageModel>, AiMuxError> {
    if !is_openai_compatible_provider(&p.provider) {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "mock replay: rebuild_provider covers only the OpenAI-compatible family \
             (recorded provider '{}'); pass a model instance to replay_with_model",
            p.provider
        )));
    }
    if p.model_id.is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "mock replay: provider record has empty model_id".into(),
        ));
    }

    let key = resolve_api_key(p, api_key)?;
    // Preserve the recorded provider name (registry entry / thin-wrapper name)
    // so the rebuilt model keeps its real identity (RFC-0023 C2). Direct OpenAI
    // recordings carry `"openai"` and round-trip unchanged.
    let mut config = OpenAIConfig::new(key)
        .with_provider(&p.provider)
        .with_profile(profile_from_value(p.profile.as_ref()));

    if let Some(base_url) = &p.base_url {
        config = config.with_base_url(base_url);
    }

    if let Some(opts) = &p.provider_options {
        // provider_options 形状未知时静默降级为默认配置(不因多余/未知字段失败)。
        if let Ok(opts) = serde_json::from_value::<ProviderOptions>(opts.clone()) {
            if let Some(url) = opts.base_url {
                config = config.with_base_url(url);
            }
            if let Some(headers) = opts.headers {
                config = config.with_headers(headers);
            }
            if let Some(org) = opts.organization {
                config = config.with_org_id(org);
            }
            if let Some(project) = opts.project {
                config = config.with_project(project);
            }
            if let Some(max_retries) = opts.max_retries {
                config.retry_config.max_retries = max_retries;
            }
            if let Some(overrides) = opts.body_overrides {
                config = config.with_body_overrides(overrides);
            }
        }
    }

    OpenAIProvider::new(config).language_model(&p.model_id)
}

/// Whether a recorded provider name belongs to the OpenAI-compatible family
/// (rebuildable via `OpenAIProvider`). After RFC-0023 C2, OpenAI-compatible
/// models surface their *real* provider name (registry entry or thin-wrapper
/// name) instead of a hardcoded `"openai"`, so the replay guard must recognise
/// all of them. Native protocols (anthropic / google / azure / bedrock / …)
/// are rejected — their snapshots are not OpenAI-shaped.
///
/// Covers:
/// - `"openai"` (direct `OpenAIProvider`),
/// - any name in `provider_registry.json` (cloud OpenAI-compatible providers),
/// - the fixed set of local/server thin wrappers that wrap `OpenAIProvider`.
fn is_openai_compatible_provider(name: &str) -> bool {
    if name == "openai" {
        return true;
    }
    // Registry-backed cloud OpenAI-compatible providers (deepseek, groq, …).
    if crate::provider::provider_registry_entry(name).is_some() {
        return true;
    }
    // Externally-registered providers via the RFC-0020 overlay.
    if crate::provider::is_external_provider(name) {
        return true;
    }
    // Local / self-hosted inference servers that wrap `OpenAIProvider`.
    // Keep in sync with the thin-wrapper modules (each sets `with_provider`).
    const THIN_WRAPPERS: &[&str] = &[
        "ollama",
        "lmstudio",
        "vllm",
        "llamafile",
        "mistralrs",
        "llamacpp",
        "localai",
        "local",
        "mlx",
        "omlx",
        "onnx",
        "oobabooba",
        "openvino",
        "sglang",
        "xinference",
        "cybertron",
        "docker_model_runner",
        "gaudi",
        "jlama",
        "litellm_proxy",
        "openrouter",
    ];
    THIN_WRAPPERS.contains(&name)
}

/// 从录制 JSON 恢复 `OpenAICompatProfile`;缺失/未知字段回退默认(`full()`)。
/// `&'static str` 字段从 String leak(bounded:每录制文件每条最多一次,同
/// registry `profile_from_registry` 模式)。
fn profile_from_value(v: Option<&serde_json::Value>) -> OpenAICompatProfile {
    let Some(v) = v else {
        return OpenAICompatProfile::full();
    };
    let s = |k: &str| {
        v[k].as_str().map(|s| {
            let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
            leaked
        })
    };
    OpenAICompatProfile {
        supports_top_k: v["supports_top_k"].as_bool().unwrap_or(true),
        supports_tools: v["supports_tools"].as_bool().unwrap_or(true),
        supports_response_format: v["supports_response_format"].as_bool().unwrap_or(true),
        stream_usage_key: s("stream_usage_key"),
        max_tokens_key: s("max_tokens_key"),
    }
}

/// 按 api_key_source 解析 key;explicit/unknown 缺 key 时报清晰错误。
fn resolve_api_key(p: &ProviderRecord, api_key: Option<&str>) -> Result<String, AiMuxError> {
    let source = p.api_key_source.trim();
    if let Some(var) = source.strip_prefix("env:") {
        return std::env::var(var).map_err(|_| {
            AiMuxError::InvalidArgument(format!(
                "mock replay: env var '{var}' not set (needed to rebuild provider '{}')",
                p.provider
            ))
        });
    }
    match source {
        // 录制不存明文(隐私强制),调用方必须补 key。
        "explicit" => api_key.map(str::to_string).ok_or_else(|| {
            AiMuxError::InvalidArgument(
                "mock replay: recording uses an explicit api key (not stored); \
                 pass api_key to rebuild_provider"
                    .to_string(),
            )
        }),
        // 本地模型无需 key。
        "none" => Ok("not-needed".to_string()),
        // "unknown"(默认快照)或空:用调用方给的 key,否则报错兜底。
        _ => api_key.map(str::to_string).ok_or_else(|| {
            AiMuxError::InvalidArgument(format!(
                "mock replay: no api key source for provider '{}' (api_key_source={:?}); \
                 pass api_key or provide a model instance to replay_with_model",
                p.provider, p.api_key_source
            ))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::recording::Recorder;

    fn openai_record(api_key_source: &str, model_id: &str) -> ProviderRecord {
        ProviderRecord {
            provider: "openai".into(),
            model_id: model_id.into(),
            base_url: Some("https://api.openai.com/v1".into()),
            api_key_source: api_key_source.into(),
            profile: None,
            provider_options: None,
        }
    }

    #[test]
    fn explicit_key_builds_model() {
        let p = openai_record("explicit", "gpt-4o");
        let model = rebuild_provider(&p, Some("sk-test")).unwrap();
        assert_eq!(model.provider(), "openai");
        assert_eq!(model.model_id(), "gpt-4o");
    }

    #[test]
    fn explicit_key_missing_errors() {
        let p = openai_record("explicit", "gpt-4o");
        let err = rebuild_provider(&p, None).err().unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
        assert!(err.to_string().contains("explicit"), "{err}");
    }

    #[test]
    fn env_key_reads_var() {
        let p = openai_record("env:AIMUX_REPLAY_TEST_KEY", "gpt-4o");
        unsafe { std::env::set_var("AIMUX_REPLAY_TEST_KEY", "sk-env") };
        let model = rebuild_provider(&p, None).unwrap();
        assert_eq!(model.model_id(), "gpt-4o");
        unsafe { std::env::remove_var("AIMUX_REPLAY_TEST_KEY") };
    }

    #[test]
    fn env_key_missing_errors() {
        unsafe { std::env::remove_var("AIMUX_REPLAY_TEST_MISSING") };
        let p = openai_record("env:AIMUX_REPLAY_TEST_MISSING", "gpt-4o");
        let err = rebuild_provider(&p, None).err().unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
        assert!(
            err.to_string().contains("AIMUX_REPLAY_TEST_MISSING"),
            "{err}"
        );
    }

    #[test]
    fn none_source_uses_placeholder() {
        let p = openai_record("none", "llama-3.3-70b");
        let model = rebuild_provider(&p, None).unwrap();
        assert_eq!(model.model_id(), "llama-3.3-70b");
    }

    #[test]
    fn unknown_source_falls_back_to_caller_key() {
        // provider 未覆盖 config_snapshot 时 api_key_source 为 "unknown"。
        let p = openai_record("unknown", "gpt-4o");
        let model = rebuild_provider(&p, Some("sk-caller")).unwrap();
        assert_eq!(model.model_id(), "gpt-4o");
    }

    #[test]
    fn unknown_source_without_key_errors() {
        let p = openai_record("unknown", "gpt-4o");
        let err = rebuild_provider(&p, None).err().unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn non_openai_protocol_is_unsupported() {
        let p = ProviderRecord {
            provider: "anthropic".into(),
            model_id: "claude-3-5-sonnet".into(),
            base_url: None,
            api_key_source: "env:ANTHROPIC_API_KEY".into(),
            profile: None,
            provider_options: None,
        };
        let err = rebuild_provider(&p, Some("sk")).err().unwrap();
        assert!(
            matches!(err, AiMuxError::UnsupportedFunctionality(_)),
            "{err}"
        );
        assert!(err.to_string().contains("replay_with_model"), "{err}");
    }

    #[test]
    fn external_overlay_provider_is_replayable() {
        // RFC-0020 overlay: a provider registered at runtime must be
        // recognized by the replay path (is_openai_compatible_provider),
        // otherwise rebuild_provider returns Unsupported and replay silently
        // fails for newly-registered external providers.
        let name = "test-replay-overlay";
        crate::provider::register_provider(crate::provider::ExternalProviderEntry {
            name: name.into(),
            display: None,
            base_url: "https://relay.test.example/v1".into(),
            env_var: None,
            api_key: Some("dummy".into()),
            protocol: "openai_compat".into(),
            profile: crate::provider::ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            max_retries: None,
            body_overrides: None,
            comment: None,
        })
        .unwrap();
        assert!(
            is_openai_compatible_provider(name),
            "external overlay provider must be replay-recognizable"
        );
        // rebuild_provider should succeed (not Unsupported) for the overlay name.
        let p = ProviderRecord {
            provider: name.into(),
            model_id: "m".into(),
            base_url: Some("https://relay.test.example/v1".into()),
            api_key_source: "explicit".into(),
            profile: None,
            provider_options: None,
        };
        let result = rebuild_provider(&p, Some("dummy"));
        assert!(
            result.is_ok(),
            "rebuild_provider should accept overlay name"
        );
        // Clean up the overlay so other tests are unaffected.
        crate::provider::clear_overlay(name);
    }

    #[test]
    fn empty_model_id_errors() {
        let p = openai_record("explicit", "");
        let err = rebuild_provider(&p, Some("sk")).err().unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn provider_options_restore_org_and_project() {
        let mut p = openai_record("explicit", "gpt-4o");
        p.provider_options = Some(serde_json::json!({
            "organization": "org-123",
            "project": "proj-456",
        }));
        let model = rebuild_provider(&p, Some("sk")).unwrap();
        assert_eq!(model.model_id(), "gpt-4o");
    }

    #[test]
    fn config_snapshot_roundtrips_through_rebuild() {
        // 构造带非默认 profile + base_url 的 OpenAI 兼容 model(groq 风格)。
        let config = crate::openai::OpenAIConfig::new("sk-test")
            .with_base_url("https://api.groq.com/openai/v1")
            .with_provider("groq")
            .with_profile(crate::openai::OpenAICompatProfile::groq());
        let model = crate::openai::OpenAIProvider::new(config).model("llama-3.3-70b");
        let snap = model.config_snapshot();
        assert_eq!(snap.provider, "groq"); // RFC-0023 C2: surfaces the registry name, not "openai"
        assert_eq!(
            snap.base_url.as_deref(),
            Some("https://api.groq.com/openai/v1")
        );
        assert_eq!(snap.api_key_source, "explicit");
        assert!(snap.profile.is_some(), "profile must be recorded");

        // 从 snapshot 重建,再 snapshot → 配置等价(round-trip)。
        let rebuilt = rebuild_provider(&snap, Some("sk-test")).unwrap();
        let snap2 = rebuilt.config_snapshot();
        assert_eq!(snap2.provider, snap.provider);
        assert_eq!(snap2.model_id, snap.model_id);
        assert_eq!(snap2.base_url, snap.base_url);
        assert_eq!(snap2.profile, snap.profile);
        assert_eq!(snap2.api_key_source, snap.api_key_source);
        assert_eq!(snap2.provider_options, snap.provider_options);
    }

    #[test]
    fn config_snapshot_marks_env_source() {
        let config = crate::openai::OpenAIConfig::from_env().unwrap_or_else(|_| {
            crate::openai::OpenAIConfig::new("sk").with_api_key_source(Some("env:OPENAI_API_KEY"))
        });
        let model = crate::openai::OpenAIProvider::new(config).model("gpt-4o");
        assert_eq!(model.config_snapshot().api_key_source, "env:OPENAI_API_KEY");
    }

    #[test]
    fn local_wrapper_marks_none_source() {
        // ollama 等本地 wrapper:占位 key → "none",回放重建无需 key。
        let config = crate::ollama::OllamaConfig::from_env().unwrap();
        let model = crate::ollama::OllamaProvider::new(config).model("llama3.2");
        let snap = model.config_snapshot();
        assert_eq!(snap.api_key_source, "none", "{snap:?}");
        // "none" 来源重建:不传 key 也应成功。
        let rebuilt = rebuild_provider(&snap, None).unwrap();
        assert_eq!(rebuilt.model_id(), "llama3.2");
    }

    #[test]
    fn recording_boundary_redacts_sensitive_headers() {
        // 脱敏验证(P5):config.headers 带 Authorization 明文,snapshot 内存中
        // 可含(录制边界为门控),但经 RingRecorder 录制导出后必须无明文。
        use std::collections::HashMap;
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer sk-secret".to_string());
        headers.insert("X-Custom".to_string(), "ok".to_string());
        let config = crate::openai::OpenAIConfig::new("sk-secret").with_headers(headers);
        let model = crate::openai::OpenAIProvider::new(config).model("gpt-4o");
        let snap = model.config_snapshot();
        assert!(snap.provider_options.is_some(), "headers must be recorded");

        let ring = aimux_core::recording::RingRecorder::with_capacity(8);
        let options = aimux_core::generate::GenerateTextOptions::default().into_call_options(vec![
            aimux_core::language_model_message::LanguageModelPromptMessage {
                role: aimux_core::message::Role::User,
                content: vec![aimux_core::content::ContentPart::Text {
                    text: "ping".into(),
                    provider_options: None,
                }],
                provider_options: None,
            },
        ]);
        ring.record_input("c1", &options, "openai", "gpt-4o");
        ring.record_provider("c1", &snap);
        ring.record_outcome(
            "c1",
            &aimux_core::recording::OutcomeRecord {
                status: aimux_core::recording::OutcomeStatus::Success,
                finish_reason: Some("stop".into()),
                error: None,
                error_value: None,
                usage: None,
            },
        );
        ring.record_transport_closed("c1");

        let mut buf = Vec::new();
        ring.export_jsonl(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains("sk-secret"), "plaintext key leaked: {text}");
        assert!(text.contains("[REDACTED]"), "must be redacted: {text}");
        assert!(
            text.contains("\"X-Custom\":\"ok\""),
            "non-sensitive kept: {text}"
        );
    }
}
