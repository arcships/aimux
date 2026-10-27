//! Response shape of the Bedrock Agent Runtime rerank endpoint (amazon-bedrock-reranking-api.ts).

use serde::Deserialize;

/// The response from the Bedrock `/rerank` endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct BedrockRerankingResponse {
    pub(super) results: Vec<BedrockRerankingResult>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub(super) struct BedrockRerankingResult {
    pub(super) index: u32,
    pub(super) relevanceScore: f64,
}
