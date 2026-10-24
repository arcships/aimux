//! Amazon Bedrock reranking (the upstream `reranking/` directory).

mod reranking_api;
mod reranking_model;
mod reranking_model_options;

pub use reranking_model::BedrockRerankingModel;
