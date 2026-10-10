//! The `LanguageModel` trait — the provider-facing interface.
//!
//! Every LLM provider implements this trait. Users never call `do_generate` /
//! `do_stream` directly; they call `generate_text` / `stream_text` instead.

use std::collections::HashMap;

use async_trait::async_trait;
use regex::Regex;

use crate::error::AiMuxError;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};

/// URL patterns a language model can fetch itself, keyed by media-type
/// pattern (`"image/*"`, `"application/pdf"`, …). Each value is a list of
/// regular expressions matched against the full URL.
///
/// The empty default means the model fetches nothing itself.
#[derive(Debug, Clone, Default)]
pub struct SupportedUrls(pub HashMap<String, Vec<Regex>>);

impl SupportedUrls {
    /// Whether no media type has a supported URL pattern.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.values().all(Vec::is_empty)
    }
}

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
    /// Provider name, e.g. `"openai"`.
    fn provider(&self) -> &str;

    /// Model identifier, e.g. `"gpt-4o"`.
    fn model_id(&self) -> &str;

    /// Generate a complete (non-streaming) response.
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError>;

    /// Generate a streaming response.
    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError>;

    /// URLs the model can fetch on its own, keyed by media-type pattern
    /// (e.g. `"image/*"`), AI SDK `supportedUrls`.
    ///
    /// Default: empty, meaning every file part is sent as inline data.
    /// Transparent decorators must forward the inner model's value. Core does
    /// not consult this yet.
    fn supported_urls(&self) -> SupportedUrls {
        SupportedUrls::default()
    }
}
