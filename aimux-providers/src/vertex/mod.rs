//! Google Vertex AI provider.
//!
//! API keys select Express mode when the provider is created. Standard mode
//! uses project/location endpoints and application default credentials.

mod anthropic;
mod auth_certificate;
mod auth_external;
mod auth_google_auth_library;
mod embedding_model;
mod embedding_model_options;
mod gemini_transcription;
mod image_model;
mod maas;
mod partner_provider;
mod provider_base;
mod transcription_model;
mod video_model;
mod xai;

pub use anthropic::{
    VertexAnthropicModel, VertexAnthropicProvider, VertexAnthropicProviderSettings,
    create_google_vertex_anthropic, google_vertex_anthropic,
};
pub use auth_google_auth_library::{GoogleAuthOptions, GoogleAuthScopes};
pub use embedding_model::VertexEmbeddingModel;
pub use gemini_transcription::VertexGeminiTranscriptionModel;
pub use image_model::VertexImageModel;
pub use maas::{create_google_vertex_maas, google_vertex_maas};
pub use partner_provider::{
    VertexMaasProviderSettings, VertexPartnerProviderSettings, VertexXaiProviderSettings,
};
pub use provider_base::{
    VertexModel, VertexProvider, VertexProviderSettings, create_google_vertex, google_vertex,
};
pub use transcription_model::VertexTranscriptionModel;
pub use video_model::VertexVideoModel;
pub use xai::{VertexXaiProvider, create_google_vertex_xai, google_vertex_xai};

pub(crate) use provider_base::{API_KEY_ENV_VAR, Publisher, Resolver};

#[cfg(test)]
mod tests;
