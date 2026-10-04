//! RFC-0023 请求回放(P4,层 2):按 `ProviderRecord` 重建 model。
//!
//! 拆层(评审 R4,2026-08-06):自动构造需要 `aimux-providers` 的构造能力,
//! 放这里避免 core→providers→core 循环依赖。core 侧的 `replay_with_model`
//! 是 provider 无关的输入重建 + 重发;本模块负责"按录制身份重建 model"。
//!
//! 录制只记身份(`provider_id` / `provider` / `model_id`),不记配置
//! (RFC-0036 §3.3)。重建按 `provider_id` 从调用方的 registry 取 provider,
//! 保留其配置,再按 `model_id` 取语言模型。重建出的 model 若与录制时的
//! `provider` 不是同一个
//! (例如录制用的是 `openai.responses`,默认语言模型是 `openai.chat`),拒绝
//! 重建:调用方需传 model 实例给
//! [`replay_with_model`](aimux_core::replay::replay_with_model)。

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider_registry::ProviderRegistry;
use aimux_core::recording::ProviderRecord;

/// 按 `ProviderRecord` 的 `provider_id` + `model_id` 重建 model。
///
/// 使用调用方 registry 中的 provider 配置。
///
/// # Errors
///
/// `InvalidArgument` for an empty `provider_id` or `model_id`, or when the
/// provider's default language model is not the one the recording was made
/// with; [`AiMuxError::NoSuchProvider`] when `provider_id` is not registered.
pub fn rebuild_provider(
    p: &ProviderRecord,
    registry: &ProviderRegistry,
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
    let model = registry
        .provider(&p.provider_id)?
        .language_model(&p.model_id)?;
    if model.provider() != p.provider {
        return Err(AiMuxError::InvalidArgument(format!(
            "mock replay: the recording was made with `{}` but provider '{}' rebuilds `{}`; \
             pass the model to replay_with_model",
            p.provider,
            p.provider_id,
            model.provider()
        )));
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::recording::Recorder;

    #[test]
    fn rebuilds_a_registered_alias_and_a_vendor_package_by_id() {
        let mut providers = crate::default_providers();
        let provider = providers.remove("abacus").unwrap();
        providers.insert("custom".into(), provider);
        let registry = aimux_core::create_provider_registry(providers, Default::default());
        let p = ProviderRecord::new("custom", "abacus.chat", "some-model");
        let model = rebuild_provider(&p, &registry).unwrap();
        assert_eq!(model.provider(), "abacus.chat");
        assert_eq!(model.model_id(), "some-model");

        let p = ProviderRecord::new("anthropic", "anthropic.messages", "some-model");
        let model = rebuild_provider(&p, &registry).unwrap();
        assert_eq!(model.provider(), "anthropic.messages");
    }

    #[test]
    fn rebuilds_responses_and_refuses_a_recording_made_with_another_method() {
        let registry =
            aimux_core::create_provider_registry(crate::default_providers(), Default::default());
        let p = ProviderRecord::new("openai", "openai.responses", "some-model");
        let model = rebuild_provider(&p, &registry).unwrap();
        assert_eq!(model.provider(), "openai.responses");
        assert_eq!(model.model_id(), "some-model");

        let p = ProviderRecord::new("openai", "openai.chat", "some-model");
        let err = rebuild_provider(&p, &registry).err().unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn an_unknown_provider_id_is_no_such_provider() {
        let registry =
            aimux_core::create_provider_registry(crate::default_providers(), Default::default());
        let p = ProviderRecord::new("nope", "nope.chat", "m");
        let err = rebuild_provider(&p, &registry).err().unwrap();
        assert!(matches!(err, AiMuxError::NoSuchProvider { .. }), "{err}");
    }

    #[test]
    fn empty_model_id_errors() {
        let registry =
            aimux_core::create_provider_registry(crate::default_providers(), Default::default());
        let err = rebuild_provider(&ProviderRecord::new("groq", "groq.chat", ""), &registry)
            .err()
            .unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }

    #[test]
    fn empty_provider_id_errors() {
        let registry =
            aimux_core::create_provider_registry(crate::default_providers(), Default::default());
        let err = rebuild_provider(&ProviderRecord::new("", "", "m"), &registry)
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
        ring.record_input("c1", &options, "openai.chat", "some-model");
        ring.record_provider(
            "c1",
            &ProviderRecord::from_model("openai.chat", "some-model"),
        );
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
                "model_id": "some-model",
            })
        );
    }
}
