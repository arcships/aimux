//! `embedding/`: the OpenAI embedding model.

pub(super) mod embedding_api;
pub(super) mod embedding_model;
pub(super) mod embedding_model_options;

pub use embedding_model::OpenAIEmbeddingModel;
