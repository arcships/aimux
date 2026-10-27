//! `reranking/cohere-reranking-api.ts`: the `/rerank` response schema.

use serde::Deserialize;

/// The response from the Cohere `/rerank` endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct CohereRerankingResponse {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) results: Vec<CohereRerankingResult>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CohereRerankingResult {
    pub(super) index: u32,
    pub(super) relevance_score: f64,
}
