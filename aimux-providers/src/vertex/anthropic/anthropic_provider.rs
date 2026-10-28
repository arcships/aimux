//! Anthropic provider on Vertex AI with application default credentials.
//!
//! Claude on Vertex is the Anthropic Messages API behind the
//! `publishers/anthropic/models/{model}:rawPredict` / `:streamRawPredict`
//! endpoints, so its model is the shared
//! [`AnthropicMessagesModel`](crate::anthropic::AnthropicMessagesModel), the
//! Rust form of `createVertexAnthropic`'s `AnthropicLanguageModel`. Only what
//! Vertex does differently is configured here:
//!
//! - the URL carries the model and the mode,
//! - the body drops `model` and gains `anthropic_version: "vertex-2023-10-16"`
//!   (the version is not a header),
//! - credentials and the base URL are the Vertex provider's, resolved on every
//!   request (OAuth bearer token; project and location),
//! - errors are Google-shaped,
//! - no URL sources, no structured-output beta and no `strict` tool
//!   definitions.
//!
//! Reference: <https://docs.cloud.google.com/claude-on-vertex-ai>

use crate::anthropic::AnthropicMessagesModel;
use crate::anthropic::config::{AnthropicConfig, AnthropicEndpoint, TransformRequestBody};
use crate::anthropic::options::options_name_of;
use crate::anthropic::request_rewrite::{AnthropicBodyRewrite, AnthropicRequestUrl};
use crate::shared::{Credential, Endpoint, ProviderHeaders, ResolveEndpoint};
use crate::vertex::auth_google_auth_library::GoogleAuth;
use crate::vertex::provider_base::{Publisher, Resolver};
use crate::vertex::{
    GoogleAuthOptions, VertexProvider, VertexProviderSettings, create_google_vertex,
};
use aimux_core::AiMuxError;
use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::provider::Provider;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, Resolvable};
use std::sync::{Arc, OnceLock};

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
        fetch: settings.fetch.clone(),
        google_auth_options: Some(auth.clone()),
        ..Default::default()
    })?;
    let resolver = Arc::get_mut(&mut provider.resolver).expect("new provider resolver is unique");
    resolver.use_access_token_env = false;
    resolver.auth = GoogleAuth::new(auth, settings.fetch);
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
        Arc::new(model(
            model_id,
            self.0.resolver.clone(),
            self.0.fetch.clone(),
            self.0.transform_request_body.clone(),
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

/// The provider string of every Anthropic model on Vertex.
const PROVIDER: &str = "googleVertex.anthropic.messages";

/// An Anthropic Claude language model served via Vertex AI.
pub type VertexAnthropicModel = AnthropicMessagesModel;

/// The model `model_id` on the Vertex endpoint `resolver` resolves for each
/// request.
pub(super) fn model(
    model_id: &str,
    resolver: Arc<Resolver>,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
) -> VertexAnthropicModel {
    // The model's own `base_url` and `headers` only describe its identity;
    // every request uses the endpoint resolved for it.
    let config = AnthropicConfig {
        provider: PROVIDER.to_string(),
        headers: ProviderHeaders::bearer(Credential::None, Vec::new(), None),
        fetch,
        supported_urls: SupportedUrls::default(),
        transform_request_body,
        base_url: String::new(),
        provider_options_name: options_name_of(PROVIDER),
        build_request_url: AnthropicRequestUrl::VertexRawPredict,
        body_rewrite: Some(AnthropicBodyRewrite::VertexRawPredict),
        supports_native_structured_output: false,
        supports_strict_tools: false,
        failed_response_handler: crate::google::google_failed_response_handler,
        endpoint: AnthropicEndpoint::Resolved(Arc::new(ClaudeEndpoint { resolver })),
    };
    AnthropicMessagesModel::with_config(model_id.to_string(), config)
}

/// The Vertex endpoint of Anthropic's models (the `anthropic` publisher),
/// resolved for every request.
struct ClaudeEndpoint {
    resolver: Arc<Resolver>,
}

#[async_trait::async_trait]
impl ResolveEndpoint for ClaudeEndpoint {
    async fn resolve(&self) -> Result<Endpoint, AiMuxError> {
        self.resolver.endpoint(Publisher::Anthropic).await
    }
}
