//! Google Vertex AI provider.
//!
//! Implements the `LanguageModel` trait against the Vertex AI API
//! (`{location}-aiplatform.googleapis.com/v1beta1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent`).
//!
//! Vertex AI uses the same request/response format as the Google Gemini API,
//! so this provider reuses the shared [`google::convert`] message conversion
//! logic. The differences are:
//! - **Endpoint**: Vertex uses a project/location-scoped URL instead of the
//!   public Gemini API URL.
//! - **Authentication**: Vertex uses a Google Cloud OAuth2 Bearer token
//!   (obtained from a service account or `gcloud auth print-access-token`)
//!   instead of an API key. Express mode (API key) is also supported.
//!
//! Reference: https://cloud.google.com/vertex-ai/generative-ai/docs/multimodal/call-gemini

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::without_trailing_slash;
use reqwest::Client;

mod embedding;
pub mod image;
mod model;
mod transcription;
mod video;

pub use embedding::VertexEmbeddingModel;
pub use image::VertexImageModel;
pub use model::{VertexConfig, VertexModel};
pub use transcription::VertexTranscriptionModel;
pub use video::VertexVideoModel;

/// Authentication method for the Vertex AI provider.
#[derive(Debug, Clone)]
pub enum VertexAuth {
    /// Google Cloud OAuth2 Bearer token (standard Vertex AI auth).
    BearerToken(String),
    /// API key for Vertex AI Express mode.
    ApiKey(String),
}

/// Configuration for the Google Vertex AI provider.
#[derive(Debug, Clone)]
pub struct VertexProviderConfig {
    /// The base URL for Vertex AI API calls.
    pub base_url: String,
    /// Google Cloud project ID (used for Speech-to-Text transcription).
    pub project: Option<String>,
    /// Default location/region (used for Speech-to-Text transcription).
    pub location: Option<String>,
    /// Authentication method.
    pub auth: VertexAuth,
}

impl VertexProviderConfig {
    /// Create a config using a Google Cloud OAuth2 Bearer token.
    ///
    /// # Arguments
    /// - `access_token` - A Google Cloud access token (e.g. from
    ///   `gcloud auth print-access-token` or a service account JWT exchange).
    /// - `project` - The Google Cloud project ID.
    /// - `location` - The Vertex AI location (e.g. `"us-central1"`).
    pub fn new(
        access_token: impl Into<String>,
        project: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        let location = location.into();
        let project = project.into();
        let base_url = build_base_url(&project, &location, false);
        Self {
            base_url,
            project: Some(project),
            location: Some(location),
            auth: VertexAuth::BearerToken(access_token.into()),
        }
    }

    /// Create a config using an API key (Vertex AI Express mode).
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            base_url: "https://aiplatform.googleapis.com/v1/publishers/google".to_string(),
            project: None,
            location: None,
            auth: VertexAuth::ApiKey(api_key.into()),
        }
    }

    /// Override the base URL (useful for tests / proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from environment variables.
    ///
    /// Checks for `GOOGLE_VERTEX_API_KEY` (Express mode) first, then falls
    /// back to `GOOGLE_VERTEX_ACCESS_TOKEN` + `GOOGLE_VERTEX_PROJECT` +
    /// `GOOGLE_VERTEX_LOCATION`.
    pub fn from_env() -> Result<Self, AiMuxError> {
        // Express mode with API key.
        if let Ok(key) = std::env::var("GOOGLE_VERTEX_API_KEY")
            && !key.trim().is_empty()
        {
            return Ok(Self::with_api_key(key));
        }

        // Standard auth with access token.
        let access_token = std::env::var("GOOGLE_VERTEX_ACCESS_TOKEN").map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_ACCESS_TOKEN (or GOOGLE_VERTEX_API_KEY) environment variable is required for Vertex AI"
                    .to_string(),
            )
        })?;
        let project = std::env::var("GOOGLE_VERTEX_PROJECT").map_err(|_| {
            AiMuxError::InvalidArgument(
                "GOOGLE_VERTEX_PROJECT environment variable is required for Vertex AI".to_string(),
            )
        })?;
        let location =
            std::env::var("GOOGLE_VERTEX_LOCATION").unwrap_or_else(|_| "us-central1".to_string());

        Ok(Self::new(access_token, project, location))
    }
}

/// Build the Vertex AI base URL for a project/location.
///
/// - `global` → `https://aiplatform.googleapis.com/v1beta1/projects/{project}/locations/global/publishers/google`
/// - `eu`/`us` → `https://aiplatform.{region}.rep.googleapis.com/v1beta1/projects/{project}/locations/{region}/publishers/google`
/// - other → `https://{location}-aiplatform.googleapis.com/v1beta1/projects/{project}/locations/{location}/publishers/google`
fn build_base_url(project: &str, location: &str, _endpoint: bool) -> String {
    let host = match location {
        "global" => "aiplatform.googleapis.com".to_string(),
        "eu" | "us" => format!("aiplatform.{}.rep.googleapis.com", location),
        _ => format!("{}-aiplatform.googleapis.com", location),
    };
    format!(
        "https://{}/v1beta1/projects/{}/locations/{}/publishers/google",
        host, project, location
    )
}

/// Google Vertex AI provider — creates [`VertexModel`] instances.
pub struct VertexProvider {
    config: VertexProviderConfig,
    client: Client,
}

impl VertexProvider {
    pub fn new(config: VertexProviderConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    /// Create a model instance for the given Vertex AI model name
    /// (e.g. `"gemini-2.0-flash"`).
    ///
    /// Returns an error when Express Mode (API key auth) is used with a tuned
    /// model (`endpoints/…`), which the Vertex AI API does not support.
    pub fn model(&self, model_id: &str) -> Result<VertexModel, AiMuxError> {
        if model_id.starts_with("endpoints/") && matches!(self.config.auth, VertexAuth::ApiKey(_)) {
            return Err(AiMuxError::InvalidArgument(
                "Google Vertex tuned models do not support Express Mode API keys. Use standard Google Cloud credentials instead.".to_string(),
            ));
        }
        Ok(VertexModel::new(
            model_id.to_string(),
            VertexConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
            self.client.clone(),
        ))
    }

    /// Create an embedding model instance for the given Vertex AI model name
    /// (e.g. `"textembedding-gecko@001"`).
    pub fn embedding_model(&self, model_id: &str) -> VertexEmbeddingModel {
        VertexEmbeddingModel::new(
            model_id.to_string(),
            VertexConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
            self.client.clone(),
        )
    }

    /// Create an image generation model instance for the given Vertex AI model
    /// name (e.g. `"imagen-4.0-generate-001"` or `"gemini-2.5-flash-image"`).
    pub fn image(&self, model_id: &str) -> VertexImageModel {
        VertexImageModel::new(
            model_id.to_string(),
            VertexConfig {
                base_url: self.config.base_url.clone(),
                auth: self.config.auth.clone(),
            },
            self.client.clone(),
        )
    }

    /// Create a transcription (STT) model instance for the given Speech-to-Text
    /// model name (e.g. `"chirp_3"`). Requires project and location to be set
    /// in the config (use [`VertexProviderConfig::new`]).
    pub fn transcription(&self, model_id: &str) -> Result<VertexTranscriptionModel, AiMuxError> {
        let project = self.config.project.clone().ok_or_else(|| {
            AiMuxError::InvalidArgument(
                "Google Vertex transcription requires project and location. Use VertexProviderConfig::new() instead of with_api_key()."
                    .to_string(),
            )
        })?;
        let location = self
            .config
            .location
            .clone()
            .unwrap_or_else(|| "us-central1".to_string());
        Ok(VertexTranscriptionModel::new(
            model_id.to_string(),
            project,
            location,
            self.config.auth.clone(),
            self.config.base_url.clone(),
            self.client.clone(),
        ))
    }

    /// Create a video generation model instance for the given Vertex AI model
    /// name (e.g. `"veo-3.0-generate-001"`).
    pub fn video(&self, model_id: &str) -> Result<VertexVideoModel, AiMuxError> {
        let project = self.config.project.clone().ok_or_else(|| {
            AiMuxError::InvalidArgument(
                "Google Vertex video requires project and location. Use VertexProviderConfig::new() instead of with_api_key()."
                    .to_string(),
            )
        })?;
        let location = self
            .config
            .location
            .clone()
            .unwrap_or_else(|| "us-central1".to_string());
        Ok(VertexVideoModel::new(
            model_id.to_string(),
            project,
            location,
            self.config.auth.clone(),
            self.config.base_url.clone(),
            self.client.clone(),
        ))
    }
}

impl Provider for VertexProvider {
    fn name(&self) -> &str {
        "google.vertex"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)?))
    }
}
