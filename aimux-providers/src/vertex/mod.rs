//! Google Vertex AI provider.
//!
//! API keys select Express mode when the provider is created. Standard mode
//! uses project/location endpoints and application default credentials.

use std::sync::Arc;

use futures::future::BoxFuture;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::transcription_model::TranscriptionModel;
use aimux_core::video_model::VideoModel;
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, Resolvable, combine_headers, load_setting,
};
use std::sync::OnceLock;

use crate::shared::{Endpoint, EndpointConfig, TransformRequestBody};

mod anthropic_model;
mod anthropic_provider;
mod auth;
mod auth_certificate;
mod auth_external;
mod embedding;
mod gemini_transcription;
pub mod image;
mod model;
mod transcription;
mod video;

pub use anthropic_model::VertexAnthropicModel;
pub use anthropic_provider::{
    VertexAnthropicProvider, VertexAnthropicProviderSettings, create_google_vertex_anthropic,
    google_vertex_anthropic,
};
pub use auth::{GoogleAuthOptions, GoogleAuthScopes};
pub use embedding::VertexEmbeddingModel;
pub use gemini_transcription::VertexGeminiTranscriptionModel;
pub use image::VertexImageModel;
pub use model::VertexModel;
pub use transcription::VertexTranscriptionModel;
pub use video::VertexVideoModel;

/// The Express-mode base URL.
const EXPRESS_MODE_BASE_URL: &str = "https://aiplatform.googleapis.com/v1/publishers/google";
const API_KEY_ENV_VAR: &str = "GOOGLE_VERTEX_API_KEY";
const ACCESS_TOKEN_ENV_VAR: &str = "GOOGLE_VERTEX_ACCESS_TOKEN";
const PROJECT_ENV_VAR: &str = "GOOGLE_VERTEX_PROJECT";
const LOCATION_ENV_VAR: &str = "GOOGLE_VERTEX_LOCATION";

/// Tuned models are addressed by their `endpoints/{id}` resource.
const ENDPOINT_MODEL_PREFIX: &str = "endpoints/";

/// Settings of [`create_google_vertex`] (the AI SDK's
/// `GoogleVertexProviderSettings`).
///
/// Every field is optional. The API-key environment setting is captured when
/// the provider is created; header and token producers run on each request.
#[derive(Clone, Default)]
pub struct VertexProviderSettings {
    /// Express-mode API key, sent as `x-goog-api-key`. `None` loads
    /// `GOOGLE_VERTEX_API_KEY` when the provider is created. An empty key means
    /// standard mode; whitespace is retained as in the upstream provider.
    pub api_key: Option<Resolvable<String>>,
    /// The location (region) of standard mode, such as
    /// `us-central1`, `global`, `us` or `eu`. `None` loads
    /// `GOOGLE_VERTEX_LOCATION` when a request is made and fails that request
    /// with `AiMuxError::LoadSetting` if it is unset. Use `base_url` for a
    /// custom endpoint.
    pub location: Option<String>,
    /// The Google Cloud project of standard mode. `None` loads
    /// `GOOGLE_VERTEX_PROJECT` when a request is made and fails that request
    /// with `AiMuxError::LoadSetting` if it is unset.
    pub project: Option<String>,
    /// Base URL for the API calls, replacing the host/project/location URL in
    /// both modes. A trailing slash is removed.
    pub base_url: Option<String>,
    /// Extra headers on every request, as a value or a producer called on
    /// every request. A `None` value removes the header. In standard mode they
    /// are layered over `Authorization`, so they can replace it; in Express
    /// mode `x-goog-api-key` wins.
    pub headers: Option<Resolvable<HeaderMapOpt>>,
    /// Overrides application default credentials with a token producer.
    /// `None` uses `GOOGLE_VERTEX_ACCESS_TOKEN` if set, then Google credentials.
    pub access_token: Option<Resolvable<String>>,
    /// Options for Google application default credentials.
    pub google_auth_options: Option<GoogleAuthOptions>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Custom WebSocket transport for Gemini live transcription.
    #[cfg(feature = "realtime")]
    pub web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for VertexProviderSettings {
    /// Never prints keys, tokens or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexProviderSettings")
            .field("api_key", &self.api_key)
            .field("location", &self.location)
            .field("project", &self.project)
            .field("base_url", &self.base_url)
            .field("headers", &self.headers.is_some())
            .field("access_token", &self.access_token)
            .field("google_auth_options", &self.google_auth_options)
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// The host of a location: `global` is the bare host, `us` and `eu` the
/// multi-region `rep` hosts, anything else a regional host.
fn location_host(location: &str) -> String {
    match location {
        "global" => "aiplatform.googleapis.com".to_string(),
        "eu" | "us" => format!("aiplatform.{location}.rep.googleapis.com"),
        _ => format!("{location}-aiplatform.googleapis.com"),
    }
}

/// Which publisher's models a base URL addresses.
#[derive(Clone, Copy)]
enum Publisher {
    Google,
    Anthropic,
}

/// A project and a location, resolved for one request.
pub(crate) struct ProjectLocation {
    pub(crate) project: String,
    pub(crate) location: String,
}

/// Resolves the project and location of a request (Speech-to-Text and the
/// operations that name them), failing for Express mode.
pub(crate) type ProjectLocationFn =
    Arc<dyn Fn() -> BoxFuture<'static, Result<ProjectLocation, AiMuxError>> + Send + Sync>;

/// The settings that decide, per request, which mode applies, where requests
/// go and what authenticates them.
struct Resolver {
    api_key: Option<Resolvable<String>>,
    express_mode: bool,
    location: Option<String>,
    project: Option<String>,
    base_url: Option<String>,
    headers: Option<Resolvable<HeaderMapOpt>>,
    access_token: Option<Resolvable<String>>,
    auth: auth::GoogleAuth,
    use_access_token_env: bool,
}

impl Resolver {
    /// The API key for the mode selected when the provider was created.
    async fn express_key(&self) -> Result<Option<String>, AiMuxError> {
        if !self.express_mode {
            return Ok(None);
        }
        match &self.api_key {
            Some(key) => Ok(Some(key.resolve().await?)),
            None => Ok(None),
        }
    }

    fn location(&self) -> Result<String, AiMuxError> {
        load_setting(self.location.as_deref(), LOCATION_ENV_VAR, "location")
    }

    fn project(&self) -> Result<String, AiMuxError> {
        load_setting(self.project.as_deref(), PROJECT_ENV_VAR, "project")
    }

    /// The base URL of `publisher`'s models.
    fn base_url(&self, express: bool, publisher: Publisher) -> Result<String, AiMuxError> {
        match publisher {
            Publisher::Google => {
                if let Some(url) = &self.base_url {
                    return Ok(url.clone());
                }
                if express {
                    return Ok(EXPRESS_MODE_BASE_URL.to_string());
                }
                let location = self.location()?;
                let project = self.project()?;
                Ok(format!(
                    "https://{}/v1beta1/projects/{project}/locations/{location}/publishers/google",
                    location_host(&location)
                ))
            }
            Publisher::Anthropic => {
                if let Some(url) = &self.base_url {
                    return Ok(url.clone());
                }
                let location = self.location()?;
                let project = self.project()?;
                let root = format!(
                    "https://{}/v1/projects/{project}/locations/{location}",
                    location_host(&location)
                );
                Ok(format!("{root}/publishers/anthropic/models"))
            }
        }
    }

    /// The provider headers of a request.
    async fn headers(&self, express_key: Option<&str>) -> Result<HeaderMapOpt, AiMuxError> {
        if let Some(key) = express_key {
            let user = match &self.headers {
                Some(headers) => headers.resolve().await?,
                None => HeaderMapOpt::new(),
            };
            let mut fixed = HeaderMapOpt::new();
            fixed.insert("x-goog-api-key".to_string(), Some(key.to_string()));
            return Ok(combine_headers(&[&user, &fixed]));
        }
        let token = match &self.access_token {
            Some(token) => token.resolve().await?,
            None if self.use_access_token_env => match std::env::var(ACCESS_TOKEN_ENV_VAR) {
                Ok(token) => token,
                Err(_) => self.auth.access_token().await?,
            },
            None => self.auth.access_token().await?,
        };
        let user = match &self.headers {
            Some(headers) => headers.resolve().await?,
            None => HeaderMapOpt::new(),
        };
        let mut auth = HeaderMapOpt::new();
        auth.insert("Authorization".to_string(), Some(format!("Bearer {token}")));
        Ok(combine_headers(&[&auth, &user]))
    }

    /// The endpoint of `publisher`'s models for one request.
    async fn endpoint(&self, publisher: Publisher) -> Result<Endpoint, AiMuxError> {
        let express_key = match publisher {
            Publisher::Google => self.express_key().await?,
            Publisher::Anthropic => None,
        };
        let base_url = self.base_url(express_key.is_some(), publisher)?;
        let headers = self.headers(express_key.as_deref()).await?;
        let headers = aimux_provider_utils::headers::with_user_agent_suffix_fn(
            Resolvable::Value(headers),
            "google-vertex",
            "5.0.98",
        )
        .resolve()
        .await?;
        Ok(Endpoint { base_url, headers })
    }
}

/// Create a Google Vertex AI provider.
///
/// # Errors
///
/// Credentials, project and location errors are reported by model requests.
pub fn create_google_vertex(
    settings: VertexProviderSettings,
) -> Result<VertexProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .map(|url| url.trim_end_matches('/').to_string());
    let api_key = settings
        .api_key
        .or_else(|| std::env::var(API_KEY_ENV_VAR).ok().map(Resolvable::Value));
    let express_mode = match &api_key {
        Some(Resolvable::Value(key)) => !key.is_empty(),
        Some(_) => true,
        None => false,
    };
    let mut google_auth_options = settings.google_auth_options.unwrap_or_default();
    if google_auth_options.project_id.is_none() {
        google_auth_options.project_id = settings.project.clone();
    }
    Ok(VertexProvider {
        resolver: Arc::new(Resolver {
            api_key,
            express_mode,
            location: settings.location,
            project: settings.project,
            base_url,
            headers: settings.headers,
            access_token: settings.access_token,
            use_access_token_env: true,
            auth: auth::GoogleAuth::new(google_auth_options, settings.fetch.clone()),
        }),
        fetch: settings.fetch,
        #[cfg(feature = "realtime")]
        web_socket: settings.web_socket,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_google_vertex` with default settings, created
/// on first use. It captures the API-key environment setting; a missing
/// token, project or location surfaces from the first
/// request instead.
pub fn google_vertex() -> &'static VertexProvider {
    static DEFAULT: OnceLock<VertexProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_vertex(VertexProviderSettings::default())
            .expect("default Vertex settings are always valid")
    })
}

/// A Google Vertex AI provider (the AI SDK's `GoogleVertexProvider`). Cheap to
/// clone the models out of; it holds no HTTP client.
pub struct VertexProvider {
    resolver: Arc<Resolver>,
    fetch: Option<FetchFunction>,
    #[cfg(feature = "realtime")]
    web_socket: Option<Arc<dyn aimux_provider_utils::ws::WsConnector>>,
    transform_request_body: Option<TransformRequestBody>,
}

/// The URL patterns the Vertex models fetch themselves
/// (`supportedUrls` in `createGoogleVertex`).
fn supported_urls() -> SupportedUrls {
    let pattern = |source: &str| regex::Regex::new(source).expect("static pattern");
    SupportedUrls(
        [(
            "*".to_string(),
            vec![pattern(r"^https?://.*$"), pattern(r"^gs://.*$")],
        )]
        .into_iter()
        .collect(),
    )
}

impl VertexProvider {
    /// The configuration of a Gemini-publisher model reporting `provider`.
    /// With `tuned`, the request fails in Express mode (tuned models are
    /// served from a deployed endpoint, which an API key cannot reach).
    fn model_config(&self, provider: &str, tuned: bool) -> EndpointConfig {
        let resolver = self.resolver.clone();
        EndpointConfig {
            provider: provider.to_string(),
            endpoint: Arc::new(move || {
                let resolver = resolver.clone();
                Box::pin(async move {
                    if tuned && resolver.express_key().await?.is_some() {
                        return Err(AiMuxError::InvalidArgument(
                            "Google Vertex tuned models do not support Express Mode API keys. \
                             Use standard Google Cloud credentials instead."
                                .to_string(),
                        ));
                    }
                    let mut endpoint = resolver.endpoint(Publisher::Google).await?;
                    if tuned && resolver.base_url.is_none() {
                        endpoint.base_url = endpoint
                            .base_url
                            .strip_suffix("/publishers/google")
                            .unwrap_or(&endpoint.base_url)
                            .to_string();
                    }
                    Ok(endpoint)
                })
            }),
            fetch: self.fetch.clone(),
            supported_urls: Arc::new(|_| supported_urls()),
            transform_request_body: self.transform_request_body.clone(),
        }
    }

    /// The project and location of a request, for the Speech-to-Text models.
    fn project_location(&self, what: &'static str) -> ProjectLocationFn {
        let resolver = self.resolver.clone();
        Arc::new(move || {
            let resolver = resolver.clone();
            Box::pin(async move {
                if resolver.express_key().await?.is_some() {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "Google Vertex {what} models do not support Express Mode API keys. \
                         Use standard Google Cloud credentials instead."
                    )));
                }
                Ok(ProjectLocation {
                    location: resolver.location()?,
                    project: resolver.project()?,
                })
            })
        })
    }

    /// A Gemini model (e.g. `"gemini-2.0-flash"`, or a tuned
    /// `"endpoints/{id}"`); `provider()` is `"google.vertex.chat"`.
    ///
    /// A tuned model fails its requests with `AiMuxError::InvalidArgument` in
    /// Express mode.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> VertexModel {
        VertexModel::from_config(
            model_id.to_string(),
            self.model_config(
                "google.vertex.chat",
                model_id.starts_with(ENDPOINT_MODEL_PREFIX),
            ),
        )
    }

    /// An embedding model (e.g. `"textembedding-gecko@001"`); `provider()` is
    /// `"google.vertex.embedding"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> VertexEmbeddingModel {
        VertexEmbeddingModel::from_config(
            model_id.to_string(),
            self.model_config("google.vertex.embedding", false),
        )
    }

    /// An image generation model (e.g. `"imagen-4.0-generate-001"` or
    /// `"gemini-2.5-flash-image"`); `provider()` is `"google.vertex.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> VertexImageModel {
        VertexImageModel::from_config(
            model_id.to_string(),
            self.model_config("google.vertex.image", false),
        )
    }

    /// A video generation model (e.g. `"veo-3.0-generate-001"`);
    /// `provider()` is `"google.vertex.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> VertexVideoModel {
        VertexVideoModel::from_config(
            model_id.to_string(),
            self.model_config("google.vertex.video", false),
        )
    }

    /// A Gemini or Speech-to-Text transcription model; `provider()`
    /// is `"google.vertex.transcription"`. Its requests need a project and a
    /// location and fail in Express mode.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> Arc<dyn TranscriptionModel> {
        if model_id.starts_with("gemini") {
            return Arc::new(VertexGeminiTranscriptionModel::from_config(
                model_id.to_string(),
                self.model_config("google.vertex.transcription", false),
                self.project_location("transcription"),
                #[cfg(feature = "realtime")]
                self.web_socket.clone(),
            ));
        }
        Arc::new(VertexTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("google.vertex.transcription", false),
            self.project_location("transcription"),
        ))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for VertexProvider {
    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        Some(self)
    }

    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Ok(Arc::new(self.embedding(model_id)))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(Arc::new(self.image(model_id)))
    }

    fn transcription_model(
        &self,
        model_id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        Some(Ok(self.transcription(model_id)))
    }

    fn video_model(&self, model_id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.video(model_id))))
    }
}

impl ProviderDiscovery for VertexProvider {
    /// `GET {base_url}/models` (Gemini native): one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config("google.vertex.models", false);
        Box::pin(async move { crate::google::list_models_once(&config).await })
    }
}
