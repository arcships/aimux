//! Voyage AI provider.
//!
//! [`create_voyage`] takes [`VoyageProviderSettings`], validates the base URL,
//! fixes the provider name and returns a [`VoyageProvider`]. The API key is not
//! read there; it is loaded in the request headers of every call, from the
//! setting or from `VOYAGE_API_KEY`. [`voyage()`] is the default instance. The
//! AI SDK has no Voyage package; the factory follows the shape of its
//! embedding and reranking vendors.
//!
//! Implements the `EmbeddingModel` trait against the Voyage AI API
//! (`api.voyageai.com/v1/embeddings`).

mod embedding;
mod provider;
mod reranking;

pub use crate::shared::TransformRequestBody;
pub use embedding::VoyageEmbeddingModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{VoyageProvider, VoyageProviderSettings, create_voyage, voyage};
pub use reranking::VoyageRerankingModel;
