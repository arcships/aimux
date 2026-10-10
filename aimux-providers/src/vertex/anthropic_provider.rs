//! Anthropic provider on Vertex AI with application default credentials.

use std::sync::{Arc, OnceLock};

use aimux_core::{
    AiMuxError, embedding_model::EmbeddingModel, image_model::ImageModel,
    language_model::LanguageModel, provider::Provider,
};
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable};

use super::{GoogleAuthOptions, VertexProvider, VertexProviderSettings, create_google_vertex};

/// Settings for the Vertex Anthropic provider.
#[derive(Clone, Default)]
pub struct VertexAnthropicProviderSettings {
    pub project: Option<String>,
    pub location: Option<String>,
    pub base_url: Option<String>,
    pub headers: Option<Resolvable<HeaderMapOpt>>,
    pub fetch: Option<FetchFunction>,
    pub google_auth_options: Option<GoogleAuthOptions>,
    /// Overrides the OAuth token generator. A missing token becomes `Bearer null`.
    pub generate_auth_token: Option<Resolvable<Option<String>>>,
}

impl std::fmt::Debug for VertexAnthropicProviderSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexAnthropicProviderSettings")
            .field("project", &self.project)
            .field("location", &self.location)
            .field("base_url", &self.base_url)
            .field("headers", &self.headers.is_some())
            .field("fetch", &self.fetch.is_some())
            .field("google_auth_options", &self.google_auth_options)
            .field("generate_auth_token", &self.generate_auth_token.is_some())
            .finish()
    }
}

/// Create a Vertex Anthropic provider.
///
/// # Errors
///
/// Credentials are resolved when a model makes a request.
pub fn create_google_vertex_anthropic(
    settings: VertexAnthropicProviderSettings,
) -> Result<VertexAnthropicProvider, AiMuxError> {
    let mut auth = settings.google_auth_options.unwrap_or_default();
    if let Some(generator) = settings.generate_auth_token {
        auth = GoogleAuthOptions {
            auth_client: Some(generator),
            ..Default::default()
        };
    }
    let mut provider = create_google_vertex(VertexProviderSettings {
        project: settings.project,
        location: settings.location,
        base_url: settings.base_url,
        headers: settings.headers,
        fetch: settings.fetch,
        google_auth_options: Some(auth.clone()),
        ..Default::default()
    })?;
    let resolver = Arc::get_mut(&mut provider.resolver).expect("new provider resolver is unique");
    resolver.use_access_token_env = false;
    resolver.auth = super::auth::GoogleAuth::new(auth, provider.fetch.clone());
    Ok(VertexAnthropicProvider(provider))
}

/// Default Vertex Anthropic provider.
pub fn google_vertex_anthropic() -> &'static VertexAnthropicProvider {
    static DEFAULT: OnceLock<VertexAnthropicProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_vertex_anthropic(VertexAnthropicProviderSettings::default())
            .expect("default Vertex Anthropic settings are valid")
    })
}

/// The Vertex Anthropic model factory.
pub struct VertexAnthropicProvider(VertexProvider);

impl VertexAnthropicProvider {
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        let resolver = self.0.resolver.clone();
        Arc::new(super::anthropic_model::model(
            model_id,
            Arc::new(move || {
                let resolver = resolver.clone();
                Box::pin(async move { resolver.endpoint(super::Publisher::Anthropic).await })
            }),
            self.0.fetch.clone(),
        ))
    }
}

impl Provider for VertexAnthropicProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::NoSuchModel {
            model_id: model_id.into(),
            model_type: "embeddingModel".into(),
        })
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::NoSuchModel {
            model_id: model_id.into(),
            model_type: "imageModel".into(),
        })
    }
}
