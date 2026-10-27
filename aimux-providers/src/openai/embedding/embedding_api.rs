//! Response shape of the OpenAI embeddings endpoint (openai-embedding-api.ts).

#[derive(serde::Deserialize)]
pub(crate) struct OpenAIEmbeddingResponse {
    pub(crate) data: Vec<OpenAIEmbeddingData>,
    pub(crate) usage: Option<OpenAIEmbeddingUsage>,
}

#[derive(serde::Deserialize)]
pub(crate) struct OpenAIEmbeddingData {
    pub(crate) embedding: Vec<f32>,
}

#[derive(serde::Deserialize)]
pub(crate) struct OpenAIEmbeddingUsage {
    pub(crate) prompt_tokens: f64,
}
