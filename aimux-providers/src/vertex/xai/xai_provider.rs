//! The Vertex AI xAI provider (xai/google-vertex-xai-provider.ts and its
//! `-node` wrapper).

use crate::openai_compatible::internal::{
    CompatibleBodyRewrite, OpenAICompatibleChatModel, OpenAICompatibleProvider,
    OpenAICompatibleProviderSettings, build,
};
use crate::vertex::partner_provider::{VertexXaiProviderSettings, partner_parts};
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::provider::Provider;
use aimux_core::types::{InputTokenUsage, OutputTokenUsage, Usage};
use regex::Regex;
use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// Upstream's `convertGoogleVertexXaiUsage` (google-vertex-xai-provider.ts:89-129):
/// reasoning tokens are reported on top of `completion_tokens`.
fn xai_usage(usage: Option<&Value>) -> Usage {
    let Some(usage) = usage.filter(|usage| !usage.is_null()) else {
        return Usage::default();
    };
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX))
    };
    let prompt = count("/prompt_tokens");
    let completion = count("/completion_tokens");
    let cache_read = count("/prompt_tokens_details/cached_tokens");
    let reasoning = count("/completion_tokens_details/reasoning_tokens");
    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt),
            no_cache: Some(prompt.saturating_sub(cache_read)),
            cache_read: Some(cache_read),
            cache_write: None,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion.saturating_add(reasoning)),
            text: Some(completion),
            reasoning: Some(reasoning),
        },
        raw: usage.as_object().cloned(),
    }
}

/// Create a Vertex AI xAI provider: Grok partner models over the
/// OpenAI-compatible Chat Completions API. Models report
/// `googleVertex.xai.chat`.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` for an explicit `base_url` that is
/// not an `http(s)` URL with a host. The project, location and token are
/// resolved when a request is made.
pub fn create_google_vertex_xai(
    settings: VertexXaiProviderSettings,
) -> Result<VertexXaiProvider, AiMuxError> {
    let (base_url, fetch_headers) = partner_parts(&settings, false)?;
    let http = Regex::new(r"^https?://.*$").expect("valid URL pattern");
    let provider = build(
        OpenAICompatibleProviderSettings {
            name: "googleVertex.xai".into(),
            fetch: settings.fetch,
            include_usage: Some(true),
            supports_structured_outputs: Some(true),
            supported_urls: Some(Arc::new(move || {
                Box::pin(std::future::ready(SupportedUrls(
                    std::iter::once(("image/*".to_string(), vec![http.clone()])).collect(),
                )))
            })),
            convert_usage: Some(Arc::new(xai_usage)),
            ..Default::default()
        },
        base_url,
        crate::shared::ProviderHeaders::new(
            crate::shared::Credential::None,
            crate::shared::AuthScheme::Bearer,
            Vec::new(),
            None,
        ),
        Some(fetch_headers),
        Some(CompatibleBodyRewrite::VertexXai),
        crate::shared::aisdk_pins::GOOGLE_VERTEX_OPENAI_COMPATIBLE,
    )?;
    Ok(VertexXaiProvider(provider))
}

/// The default Vertex xAI provider. Creating it reads nothing from the
/// environment and cannot fail.
pub fn google_vertex_xai() -> &'static VertexXaiProvider {
    static DEFAULT: OnceLock<VertexXaiProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_vertex_xai(VertexXaiProviderSettings::default())
            .expect("default Vertex xAI settings are valid")
    })
}

/// The Vertex xAI model factory (the AI SDK's `GoogleVertexXaiProvider`):
/// chat models only.
pub struct VertexXaiProvider(OpenAICompatibleProvider);

impl VertexXaiProvider {
    /// A Grok chat model; `provider()` is `"googleVertex.xai.chat"`.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> OpenAICompatibleChatModel {
        self.0.chat(model_id)
    }

    /// The provider as a function: the chat model for an id.
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for VertexXaiProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}
