//! Anthropic partner models on Vertex AI (`src/anthropic/`).

mod anthropic_provider;

pub use anthropic_provider::{
    VertexAnthropicModel, VertexAnthropicProvider, VertexAnthropicProviderSettings,
    create_google_vertex_anthropic, google_vertex_anthropic,
};
