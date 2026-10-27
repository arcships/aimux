//! Vertex AI xAI (Grok) models (`src/xai/`).

mod xai_provider;

pub use xai_provider::{VertexXaiProvider, create_google_vertex_xai, google_vertex_xai};
