//! Google Gemini provider.
//!
//! Implements the `LanguageModel` trait against Google's Generative Language
//! API (`generativelanguage.googleapis.com/v1beta`).
//!
//! Gemini's API shape is fundamentally different from OpenAI/Anthropic:
//! - The model id is part of the URL path, not the request body.
//! - Authentication is via the `x-goog-api-key` header (or `?key=…`).
//! - System messages become a top-level `systemInstruction` field.
//! - The model role is `"model"` (not `"assistant"`).
//! - Tool calls are `functionCall` parts; tool results are
//!   `functionResponse` parts.
//! - Streaming is SSE with one JSON object per `data:` line.

pub mod convert;
pub mod embedding;
pub mod files;
pub mod image;
mod model;
pub mod types;
pub mod utils;
pub mod video;

pub use embedding::GoogleEmbeddingModel;
pub use image::{GoogleImageModel, GoogleImageSettings};
pub use video::GoogleVideoModel;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{load_api_key, without_trailing_slash};

/// Configuration for the Google Gemini provider.
#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub api_key: String,
    pub base_url: String,
}

impl GoogleConfig {
    /// Create from an API key (uses the default Gemini API base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        }
    }

    /// Use a custom base URL (e.g. for Vertex AI or a proxy).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    /// Create from the `GOOGLE_GENERATIVE_AI_API_KEY` environment variable.
    pub fn from_env() -> Result<Self, AiMuxError> {
        let api_key = load_api_key(None, "GOOGLE_GENERATIVE_AI_API_KEY", "Google Generative AI")?;
        Ok(Self::new(api_key))
    }
}

/// Google Gemini provider — creates `GoogleModel` instances.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct GoogleProvider {
    config: GoogleConfig,
}

impl GoogleProvider {
    pub fn new(config: GoogleConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model name (e.g. `"gemini-2.0-flash"`).
    pub fn model(&self, model_id: &str) -> model::GoogleModel {
        model::GoogleModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Files interface for uploading files to Google.
    pub fn files(&self) -> files::GoogleFiles {
        files::GoogleFiles::new(self.config.clone())
    }

    /// Create an embedding model instance for the given model name (e.g.
    /// `"gemini-embedding-001"`).
    pub fn embedding_model(&self, model_id: &str) -> embedding::GoogleEmbeddingModel {
        embedding::GoogleEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an image generation model instance for the given model name
    /// (e.g. `"imagen-3.0-generate-002"` or `"gemini-2.5-flash-image"`).
    pub fn image(&self, model_id: &str) -> image::GoogleImageModel {
        image::GoogleImageModel::new(
            model_id.to_string(),
            image::GoogleImageSettings::default(),
            self.config.clone(),
        )
    }

    /// Create a video generation model instance for the given model name
    /// (e.g. `"veo-3.0-generate-001"`).
    pub fn video(&self, model_id: &str) -> video::GoogleVideoModel {
        video::GoogleVideoModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an image generation model instance with custom settings.
    pub fn image_with_settings(
        &self,
        model_id: &str,
        settings: image::GoogleImageSettings,
    ) -> image::GoogleImageModel {
        image::GoogleImageModel::new(model_id.to_string(), settings, self.config.clone())
    }
}

impl Provider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }

    /// List models via `GET {base_url}/models` (Gemini native), enriched with
    /// the community catalogue portrait when available (RFC-0027).
    fn list_models(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        Box::pin(async move {
            let base = config.base_url.trim_end_matches('/');
            let url = format!("{base}/models");
            let mut headers = std::collections::HashMap::new();
            headers.insert("x-goog-api-key".to_string(), config.api_key.clone());
            let mut header_list: Vec<(String, String)> = headers.into_iter().collect();
            header_list.push(("Content-Type".to_string(), "application/json".to_string()));

            use aimux_provider_utils::{
                DEFAULT_ERROR_STRUCTURE, HttpBody, HttpMethod, HttpRequest, send_timed,
            };
            let resp = send_timed(
                HttpRequest {
                    method: HttpMethod::Get,
                    url,
                    headers: header_list,
                    body: HttpBody::Empty,
                    abort_signal: None,
                    call_id: None,
                    recording_context: None,
                },
                aimux_provider_utils::RetryConfig::default(),
                &DEFAULT_ERROR_STRUCTURE,
                None,
            )
            .await?;

            // Gemini response: { models: [{ name: "models/gemini-...", displayName, ... }] }
            #[derive(serde::Deserialize)]
            struct Resp {
                #[serde(default)]
                models: Vec<Entry>,
            }
            #[derive(serde::Deserialize)]
            struct Entry {
                name: String,
                #[serde(default)]
                display_name: Option<String>,
            }
            let parsed: Resp = serde_json::from_slice(&resp.body)
                .map_err(|e| AiMuxError::Json(format!("google list_models: parse: {e}")))?;
            let runtime: Vec<aimux_core::model_catalogue::RuntimeModel> = parsed
                .models
                .into_iter()
                .map(|e| {
                    // Strip "models/" prefix → bare model id.
                    let id = e
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&e.name)
                        .to_string();
                    aimux_core::model_catalogue::RuntimeModel {
                        id,
                        owned_by: e.display_name,
                        created: None,
                    }
                })
                .collect();
            Ok(runtime)
        })
    }
}
