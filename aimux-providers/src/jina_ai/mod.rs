//! Jina AI provider — rerank modality only.
//!
//! Implements the `RerankingModel` trait against the Jina AI rerank API
//! (`POST https://api.jina.ai/v1/rerank`).
//!
//! Jina AI is a modality-specific provider: it exposes a native rerank
//! protocol (query + documents → relevance scores) and does not support
//! language models.
//!
//! [`create_jina_ai`] takes [`JinaAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`JinaAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `JINA_AI_API_KEY`.
//! [`jina_ai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod reranking_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{JinaAiProvider, JinaAiProviderSettings, create_jina_ai, jina_ai};
pub use reranking_model::JinaAiRerankingModel;
