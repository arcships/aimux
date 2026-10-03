//! RFC-0023 请求回放(P4,层 2):按 `ProviderRecord` 重建 model。
//!
//! 拆层(评审 R4,2026-08-06):自动构造需要 `aimux-providers` 的构造能力,
//! 放这里避免 core→providers→core 循环依赖。core 侧的 `replay_with_model`
//! 是 provider 无关的输入重建 + 重发;本模块负责"按录制身份重建 model"。
//!
//! 录制只记身份(`provider_id` / `provider` / `model_id`),不记配置
//! (RFC-0036 §3.3)。重建只按 `provider_id` + `model_id` 走
//! [`provider`](crate::provider::provider):base URL 与 profile 来自 registry,
//! 凭证来自调用方显式传入的 key,否则读 registry 条目的环境变量。
//! 原生协议包(anthropic / google / bedrock …)接入 registry 之前,它们的
//! `provider_id` 返回 `NoSuchProvider`:调用方需传 model 实例给
//! [`replay_with_model`](aimux_core::replay::replay_with_model)。

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::recording::ProviderRecord;

/// 按 `ProviderRecord` 的 `provider_id` + `model_id` 重建 model。
///
/// `api_key` 为 `None` 时由 registry 条目的环境变量取 key。
///
/// # Errors
///
/// Returns `InvalidArgument` for an empty `provider_id` or `model_id`,
/// [`AiMuxError::NoSuchProvider`] when `provider_id` is not a registered
/// provider (including native-protocol providers, which have no registry
/// entry yet), and key-resolution errors.
pub fn rebuild_provider(
    p: &ProviderRecord,
    api_key: Option<&str>,
) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
    if p.provider_id.is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "mock replay: provider record has empty provider_id".into(),
        ));
    }
    if p.model_id.is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "mock replay: provider record has empty model_id".into(),
        ));
    }
    crate::provider::provider(
        &p.provider_id,
        api_key.map(str::to_string),
        &p.model_id,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::recording::Recorder;

    fn record(provider_id: &str, model_id: &str) -> ProviderRecord {
        ProviderRecord::new(provider_id, provider_id, model_id)
    }

    fn register_overlay(name: &str, env_var: Option<&str>) {
        crate::provider::register_provider(crate::provider::ExternalProviderEntry {
            name: name.into(),
            display: None,
            base_url: "https://relay.test.example/v1".into(),
            env_var: env_var.map(Into::into),
            api_key: None,
            protocol: "openai_compat".into(),
            profile: crate::provider::ProviderProfile::default(),
            headers: None,
            organization: None,
            project: None,
            comment: None,
        })
        .unwrap();
    }

    #[test]
    fn rebuilds_registry_provider_by_id_and_model() {
        let model = rebuild_provider(&record("groq", "llama-3.3-70b"), Some("sk-test")).unwrap();
        assert_eq!(model.provider(), "groq.chat");
        assert_eq!(model.model_id(), "llama-3.3-70b");
    }

    #[test]
    fn rebuild_ignores_recorded_provider_string() {
        // Only `provider_id` selects the provider; `provider` is informational.
        let p = ProviderRecord::new("deepseek", "something.else", "deepseek-chat");
        let model = rebuild_provider(&p, Some("sk-test")).unwrap();
        assert_eq!(model.provider(), "deepseek.chat");
        assert_eq!(model.model_id(), "deepseek-chat");
    }

    #[test]
    fn missing_env_key_names_the_variable() {
        let name = "test-replay-missing-env";
        register_overlay(name, Some("AIMUX_REPLAY_TEST_MISSING"));
        unsafe { std::env::remove_var("AIMUX_REPLAY_TEST_MISSING") };
        let err = rebuild_provider(&record(name, "m"), None).err().unwrap();
        crate::provider::clear_overlay(name);
        assert!(
            matches!(&err, AiMuxError::LoadApiKey { env_var, .. } if env_var == "AIMUX_REPLAY_TEST_MISSING"),
            "{err}"
        );
    }

    #[test]
    fn env_key_is_read_from_registry_variable() {
        let name = "test-replay-env";
        register_overlay(name, Some("AIMUX_REPLAY_TEST_KEY"));
        unsafe { std::env::set_var("AIMUX_REPLAY_TEST_KEY", "sk-env") };
        let result = rebuild_provider(&record(name, "m"), None);
        unsafe { std::env::remove_var("AIMUX_REPLAY_TEST_KEY") };
        crate::provider::clear_overlay(name);
        assert_eq!(result.unwrap().model_id(), "m");
    }

    #[test]
    fn native_protocol_provider_is_no_such_provider() {
        // Native packages have no registry entry until they get their own
        // factories; callers pass a model to `replay_with_model` instead.
        let err = rebuild_provider(&record("anthropic", "claude-3-5-sonnet"), Some("sk"))
            .err()
            .unwrap();
        assert!(
            matches!(&err, AiMuxError::NoSuchProvider { provider_id } if provider_id == "anthropic"),
            "{err}"
        );
    }

    #[test]
    fn external_overlay_provider_is_replayable() {
        // RFC-0020 overlay: a provider registered at runtime is found by id.
        let name = "test-replay-overlay";
        register_overlay(name, None);
        let result = rebuild_provider(&record(name, "m"), Some("dummy"));
        crate::provider::clear_overlay(name);
        let model = result.expect("rebuild_provider should accept an overlay provider id");
        assert_eq!(model.provider(), format!("{name}.chat"));
    }

    #[test]
    fn empty_model_id_errors() {
        let err = rebuild_provider(&record("groq", ""), Some("sk"))
            .err()
            .unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn empty_provider_id_errors() {
        let err = rebuild_provider(&record("", "m"), Some("sk"))
            .err()
            .unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn recorded_provider_identity_carries_no_configuration() {
        // The record that reaches disk is identity only: no base URL, headers
        // or key source, so nothing from the provider config can leak.
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
        ring.record_input("c1", &options, "openai.chat", "gpt-4o");
        ring.record_provider("c1", &ProviderRecord::from_model("openai.chat", "gpt-4o"));
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
        let line = String::from_utf8(buf).unwrap();
        let rec: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            rec["provider"],
            serde_json::json!({
                "provider_id": "openai",
                "provider": "openai.chat",
                "model_id": "gpt-4o",
            })
        );
    }
}
