//! OpenAI-compatible internal exports (`@ai-sdk/openai-compatible/internal`,
//! `src/internal/index.ts`), consumed by `preset` and `vertex` (`maas`, `xai`).

pub(crate) use super::chat::{ChatSettings, usage_from_raw};
pub(crate) use super::provider::{Assembly, BaseUrl, ResolveBaseUrl, ResolveFetchHeaders, build};
pub(crate) use super::request_rewrite::CompatibleBodyRewrite;
pub(crate) use super::{
    ConvertUsage, OpenAICompatibleChatModel, OpenAICompatibleProvider,
    OpenAICompatibleProviderSettings, TransformRequestBody,
};
