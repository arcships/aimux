//! The `LanguageModel` trait — the provider-facing interface.
//!
//! Every LLM provider implements this trait. Users never call `do_generate` /
//! `do_stream` directly; they call `generate_text` / `stream_text` instead.

use async_trait::async_trait;

use crate::error::AiMuxError;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};

/// The unified language model trait (provider-facing).
///
/// Aligned with Vercel AI SDK `LanguageModelV4`.
///
/// # Implementation notes
///
/// - `do_generate` / `do_stream` are **not** user-facing API. Users call
///   `generate_text` / `stream_text` (free functions in [`crate::generate`]).
/// - Providers must emit `StreamStart` as the first stream part, and `Finish`
///   as the last (before any `Error`).
/// - Unsupported `CallOptions` fields must produce a `Warning::Unsupported`
///   rather than being silently dropped.
#[async_trait]
pub trait LanguageModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Model identifier, e.g. `"gpt-4o"`.
    fn model_id(&self) -> &str;

    /// Provider/model retry settings used when the caller does not override
    /// the retry count.
    fn retry_config(&self) -> crate::retry::RetryConfig {
        crate::retry::RetryConfig::default()
    }

    /// Generate a complete (non-streaming) response.
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError>;

    /// Generate a streaming response.
    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError>;

    /// RFC-0023: 配置侧快照供录制。默认返回最小信息(provider/model_id);
    /// 各 provider 覆盖以提供 base_url/profile/api_key 来源等完整配置。
    /// 透明 decorator(TraceLayer 等)必须覆盖并转发 inner 快照。
    fn config_snapshot(&self) -> crate::recording::ProviderRecord {
        crate::recording::ProviderRecord::minimal(self.provider(), self.model_id())
    }
}
