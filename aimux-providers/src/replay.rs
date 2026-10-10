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

/// Rebuild a decision model using the caller's registered provider configuration.
/// # Errors
/// Rejects missing identities, missing providers and a different model provider.
pub fn rebuild_decision_provider(
    p: &ProviderRecord,
    registry: &ProviderRegistry,
) -> Result<Box<dyn aimux_core::DecisionModel>, AiMuxError> {
    if p.provider_id.is_empty() || p.model_id.is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "decision replay: empty provider/model identity".into(),
        ));
    }
    let model = registry
        .provider(&p.provider_id)?
        .decision_model(&p.model_id)?;
    if model.provider() != p.provider {
        return Err(AiMuxError::InvalidArgument(
            "decision replay: registered model provider differs from recording".into(),
        ));
    }
    Ok(model)
}

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

    #[test]
    fn empty_model_id_errors() {
        let registry =
            aimux_core::create_provider_registry(crate::default_providers(), Default::default());
        let err = rebuild_provider(&ProviderRecord::new("groq", "groq.chat", ""), &registry)
            .err()
            .unwrap();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "{err}");
    }
}
