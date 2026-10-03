//! The `OpenAIConfig` builder and the provider built from it.
//!
//! Transitional. The native OpenAI package is configured through
//! `OpenAIProviderSettings` and `create_openai`; this builder remains for the
//! OpenAI-compatible consumers that have not moved to the compat package yet
//! (the registry, the thin wrappers, Codex, xAI, Hugging Face). It carries
//! plain data, evaluated when [`OpenAIConfig::into_model_config`] turns it
//! into the private model configuration, so the model implementations exist
//! once. It is removed together with those consumers.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::speech_model::SpeechModel;
use aimux_core::transcription_model::TranscriptionModel;
use aimux_provider_utils::{HeaderMapOpt, Resolvable, combine_headers, without_trailing_slash};

use crate::body_merge::deep_merge_json;
use crate::openai::config::{OpenAIModelConfig, TransformRequestBody};
use crate::openai::{
    OpenAICompatProfile, OpenAIEmbeddingModel, OpenAIImageModel, OpenAIModel, OpenAIResponsesModel,
    OpenAISpeechModel, OpenAITranscriptionModel, files::OpenAIFiles, model,
};

/// Configuration for an OpenAI-compatible endpoint.
#[derive(Debug, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub base_url: String,
    pub org_id: Option<String>,
    /// OpenAI project ID sent via the `OpenAI-Project` header.
    pub project: Option<String>,
    /// Extra headers merged into every request.
    pub headers: Option<HashMap<String, String>>,
    /// Provider name — controls provider-specific behaviour in the shared
    /// request builder (e.g. "groq" reads provider options from the "groq"
    /// key and applies a reasoning-effort map). Defaults to "openai".
    pub provider: String,
    /// 厂商能力差异描述。默认 `full()`（支持全部能力）。
    /// 薄封装用 `with_profile()` 设置差异。
    pub profile: OpenAICompatProfile,
    /// Provider 级请求体覆盖（RFC-0017），来自 registry 外部条目。
    /// `into_model_config` 把它转成 chat 模型的
    /// `transform_request_body` 闭包（deep-merge，发送前最后一步）。
    pub body_overrides: Option<Value>,
}

impl OpenAIConfig {
    /// Create from an API key (uses default OpenAI base URL).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            org_id: None,
            project: None,
            headers: None,
            provider: "openai".to_string(),
            profile: OpenAICompatProfile::full(),
            body_overrides: None,
        }
    }

    /// Use a custom base URL (for Azure, Groq, etc.).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = without_trailing_slash(&url.into());
        self
    }

    #[must_use]
    pub fn with_org_id(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }

    /// Set the OpenAI project ID (sent via the `OpenAI-Project` header).
    #[must_use]
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Attach extra headers merged into every request.
    #[must_use]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set the provider name (e.g. "groq") for provider-specific behaviour.
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    /// 设置厂商能力差异描述。
    #[must_use]
    pub fn with_profile(mut self, profile: OpenAICompatProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Turn the builder into the model configuration for `method` (`"chat"`,
    /// `"responses"`, `"embedding"`, `"image"`, `"speech"`, `"transcription"`,
    /// `"files"`, or `"models"` for discovery).
    ///
    /// The credential is the explicit key the builder was given, so the headers
    /// are a fixed map. Identity stays the flat provider name (`"groq"`) for
    /// every method but `"files"` (`"groq.files"`), because the shared request
    /// builder still keys vendor behaviour on that name; the compat package
    /// moves it to `"{name}.{method}"`. The body overrides apply to chat only,
    /// as they always have.
    pub(crate) fn into_model_config(self, method: &str) -> OpenAIModelConfig {
        let mut fixed = HeaderMapOpt::new();
        fixed.insert(
            "Authorization".to_string(),
            Some(format!("Bearer {}", self.api_key)),
        );
        if let Some(org) = self.org_id {
            fixed.insert("OpenAI-Organization".to_string(), Some(org));
        }
        if let Some(project) = self.project {
            fixed.insert("OpenAI-Project".to_string(), Some(project));
        }
        let headers = match self.headers {
            Some(user) => {
                let user: HeaderMapOpt = user.into_iter().map(|(k, v)| (k, Some(v))).collect();
                combine_headers(&[&fixed, &user])
            }
            None => fixed,
        };

        let transform_request_body: Option<TransformRequestBody> = match self.body_overrides {
            Some(overrides) if method == "chat" => Some(Arc::new(move |mut body: Value| {
                deep_merge_json(&mut body, &overrides);
                body
            })),
            _ => None,
        };

        let base = self.base_url.clone();
        OpenAIModelConfig {
            provider: if method == "files" {
                format!("{}.files", self.provider)
            } else {
                self.provider
            },
            url: Arc::new(move |path| format!("{base}{path}")),
            headers: Resolvable::Value(headers),
            fetch: None,
            supported_urls: SupportedUrls::default(),
            transform_request_body,
            base_url: self.base_url,
            profile: self.profile,
        }
    }
}

/// A provider built from an [`OpenAIConfig`]: what the registry and the thin
/// OpenAI-compatible wrappers hold. Transitional, like the builder.
pub struct OpenAIConfigProvider {
    config: OpenAIConfig,
}

impl OpenAIConfigProvider {
    #[must_use]
    pub fn new(config: OpenAIConfig) -> Self {
        Self { config }
    }

    /// Create a chat model instance for the given model name (e.g. `"gpt-4o"`).
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenAIModel {
        OpenAIModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Responses API model instance for the given model name. Uses
    /// the `/responses` endpoint instead of `/chat/completions`.
    #[must_use]
    pub fn responses_model(&self, model_id: &str) -> OpenAIResponsesModel {
        OpenAIResponsesModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a Files interface.
    #[must_use]
    pub fn files(&self) -> OpenAIFiles {
        OpenAIFiles::new(self.config.clone())
    }

    /// Create an embedding model instance.
    #[must_use]
    pub fn embedding_model(&self, model_id: &str) -> OpenAIEmbeddingModel {
        OpenAIEmbeddingModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a speech (TTS) model instance.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> OpenAISpeechModel {
        OpenAISpeechModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create an image generation model instance.
    #[must_use]
    pub fn image(&self, model_id: &str) -> OpenAIImageModel {
        OpenAIImageModel::new(model_id.to_string(), self.config.clone())
    }

    /// Create a transcription (STT) model instance.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> OpenAITranscriptionModel {
        OpenAITranscriptionModel::new(model_id.to_string(), self.config.clone())
    }
}

impl Provider for OpenAIConfigProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(Arc::new(self.model(model_id)))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding_model(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.transcription(model_id))))
    }

    fn speech_model(&self, model_id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.speech(model_id))))
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        Some(Arc::new(self.files()))
    }
}

impl ProviderDiscovery for OpenAIConfigProvider {
    /// List models via `GET {base_url}/models` (OpenAI-compatible, RFC-0027).
    fn list_models(
        &self,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError>,
    > {
        let config = self.config.clone().into_model_config("models");
        Box::pin(async move { model::execute_list_models(&config).await })
    }
}
