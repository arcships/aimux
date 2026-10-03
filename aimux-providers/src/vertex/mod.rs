//! Google Vertex AI provider.
//!
//! [`create_google_vertex`] is the Rust form of the AI SDK's
//! `createGoogleVertex`: it takes [`VertexProviderSettings`], validates what
//! was given explicitly and returns a [`VertexProvider`]. Nothing is read from
//! the environment and no project, location, key or token is needed until a
//! request is made: each call resolves its mode, base URL and headers then.
//! [`google_vertex()`] is the default instance.
//!
//! Two modes, chosen per request:
//!
//! - **Express mode** when an API key is available (the `api_key` setting, else
//!   `GOOGLE_VERTEX_API_KEY`): requests carry `x-goog-api-key` and go to
//!   `https://aiplatform.googleapis.com/v1/publishers/google`.
//! - **Standard mode** otherwise: requests carry `Authorization: Bearer <token>`
//!   and go to the project/location-scoped host. The token is the
//!   `access_token` setting (any [`Resolvable`], so a host can refresh it) or
//!   `GOOGLE_VERTEX_ACCESS_TOKEN`; aimux does not implement Application
//!   Default Credentials. A host that authenticates some other way supplies an
//!   `Authorization` header through `headers` instead.
//!
//! Vertex serves the Gemini request format, so these models share the
//! conversion in [`crate::google::convert`]; they differ in endpoint,
//! authentication and the providerOptions/metadata namespace (options are
//! read from `googleVertex`, then `google`; metadata is written under
//! `googleVertex` only). Claude on
//! Vertex is the shared Anthropic Messages model (see
//! [`VertexProvider::anthropic_model`]).
//!
//! Reference: <https://cloud.google.com/vertex-ai/generative-ai/docs/multimodal/call-gemini>

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
    FetchFunction, HeaderMapOpt, Resolvable, combine_headers, load_api_key, load_setting,
    validate_base_url,
};
use std::sync::OnceLock;

use crate::shared::{Endpoint, EndpointConfig, TransformRequestBody, is_valid_hostname_part};

mod anthropic_model;
mod embedding;
pub mod image;
mod model;
mod transcription;
mod video;

pub use anthropic_model::VertexAnthropicModel;
pub use embedding::VertexEmbeddingModel;
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

/// The provider string of the language, embedding and image models.
const PROVIDER: &str = "google.vertex";

/// Tuned models are addressed by their `endpoints/{id}` resource.
const ENDPOINT_MODEL_PREFIX: &str = "endpoints/";

/// Settings of [`create_google_vertex`] (the AI SDK's
/// `GoogleVertexProviderSettings`, plus the token source aimux needs in place
/// of google-auth-library).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except an explicit `base_url` and `location`; everything else is
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct VertexProviderSettings {
    /// Express-mode API key, sent as `x-goog-api-key`. `None` loads
    /// `GOOGLE_VERTEX_API_KEY` when a request is made; with no key at all the
    /// request uses standard mode. An empty or whitespace-only key also means
    /// standard mode.
    pub api_key: Option<Resolvable<String>>,
    /// The location (region) of standard mode: a single DNS label such as
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
    /// The OAuth2 access token of standard mode, sent as
    /// `Authorization: Bearer`. `None` loads `GOOGLE_VERTEX_ACCESS_TOKEN` when
    /// a request is made and fails that request with `AiMuxError::LoadApiKey`
    /// if it is unset (unless `headers` carries an `Authorization` header). A
    /// [`Resolvable::AsyncFn`] is called on every request, so a host can hand
    /// out refreshed tokens.
    pub access_token: Option<Resolvable<String>>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
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
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

fn validate_location(location: &str) -> Result<(), AiMuxError> {
    if is_valid_hostname_part(location) {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(
            "Invalid Google Vertex location. Expected a single DNS label (letters, digits, and \
             hyphens). Use `base_url` for custom endpoints."
                .to_string(),
        ))
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
    location: Option<String>,
    project: Option<String>,
    base_url: Option<String>,
    headers: Option<Resolvable<HeaderMapOpt>>,
    access_token: Option<Resolvable<String>>,
}

impl Resolver {
    /// The Express-mode key, when there is a non-empty one.
    async fn express_key(&self) -> Result<Option<String>, AiMuxError> {
        let key = match &self.api_key {
            Some(key) => Some(key.resolve().await?),
            None => std::env::var(API_KEY_ENV_VAR).ok(),
        };
        Ok(key.filter(|key| !key.trim().is_empty()))
    }

    fn location(&self) -> Result<String, AiMuxError> {
        let location = load_setting(self.location.as_deref(), LOCATION_ENV_VAR, "location")?;
        validate_location(&location)?;
        Ok(location)
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
                // Claude is served from the same project/location under the
                // `anthropic` publisher; a configured base URL has its Google
                // publisher suffix swapped.
                let root = if let Some(url) = &self.base_url {
                    url.strip_suffix("/publishers/google")
                        .unwrap_or(url)
                        .to_string()
                } else if express {
                    EXPRESS_MODE_BASE_URL
                        .strip_suffix("/publishers/google")
                        .unwrap_or(EXPRESS_MODE_BASE_URL)
                        .to_string()
                } else {
                    let location = self.location()?;
                    let project = self.project()?;
                    format!(
                        "https://{}/v1/projects/{project}/locations/{location}",
                        location_host(&location)
                    )
                };
                Ok(format!("{root}/publishers/anthropic/models"))
            }
        }
    }

    /// The provider headers of a request.
    async fn headers(&self, express_key: Option<&str>) -> Result<HeaderMapOpt, AiMuxError> {
        let user = match &self.headers {
            Some(headers) => headers.resolve().await?,
            None => HeaderMapOpt::new(),
        };
        if let Some(key) = express_key {
            let mut fixed = HeaderMapOpt::new();
            fixed.insert("x-goog-api-key".to_string(), Some(key.to_string()));
            return Ok(combine_headers(&[&user, &fixed]));
        }
        let user_authorizes = user
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("authorization") && value.is_some());
        if user_authorizes {
            return Ok(user);
        }
        let token = match &self.access_token {
            Some(token) => token.resolve().await?,
            None => load_api_key(None, ACCESS_TOKEN_ENV_VAR, "Google Vertex access token")?,
        };
        let mut auth = HeaderMapOpt::new();
        auth.insert("Authorization".to_string(), Some(format!("Bearer {token}")));
        Ok(combine_headers(&[&auth, &user]))
    }

    /// The endpoint of `publisher`'s models for one request.
    async fn endpoint(&self, publisher: Publisher) -> Result<Endpoint, AiMuxError> {
        let express_key = self.express_key().await?;
        let base_url = self.base_url(express_key.is_some(), publisher)?;
        let headers = self.headers(express_key.as_deref()).await?;
        Ok(Endpoint { base_url, headers })
    }
}

/// Create a Google Vertex AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when an explicit `base_url` is not an
/// `http(s)` URL with a host, or an explicit `location` is not a single DNS
/// label. Those are the only ways this fails: keys, tokens, and a project or
/// location taken from the environment are loaded per request, not here.
pub fn create_google_vertex(
    settings: VertexProviderSettings,
) -> Result<VertexProvider, AiMuxError> {
    let base_url = settings
        .base_url
        .as_deref()
        .map(validate_base_url)
        .transpose()?;
    if let Some(location) = settings.location.as_deref() {
        validate_location(location)?;
    }
    Ok(VertexProvider {
        resolver: Arc::new(Resolver {
            api_key: settings.api_key,
            location: settings.location,
            project: settings.project,
            base_url,
            headers: settings.headers,
            access_token: settings.access_token,
        }),
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

/// The default provider: `create_google_vertex` with default settings, created
/// on first use. Creating it reads nothing from the environment and cannot
/// fail; a missing key, token, project or location surfaces from the first
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
                    resolver.endpoint(Publisher::Google).await
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
    /// `"endpoints/{id}"`); `provider()` is `"google.vertex"`.
    ///
    /// A tuned model fails its requests with `AiMuxError::InvalidArgument` in
    /// Express mode.
    #[must_use]
    pub fn chat(&self, model_id: &str) -> VertexModel {
        VertexModel::from_config(
            model_id.to_string(),
            self.model_config(PROVIDER, model_id.starts_with(ENDPOINT_MODEL_PREFIX)),
        )
    }

    /// An embedding model (e.g. `"textembedding-gecko@001"`); `provider()` is
    /// `"google.vertex"`.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> VertexEmbeddingModel {
        VertexEmbeddingModel::from_config(model_id.to_string(), self.model_config(PROVIDER, false))
    }

    /// An image generation model (e.g. `"imagen-4.0-generate-001"` or
    /// `"gemini-2.5-flash-image"`); `provider()` is `"google.vertex"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> VertexImageModel {
        VertexImageModel::from_config(model_id.to_string(), self.model_config(PROVIDER, false))
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

    /// A Speech-to-Text transcription model (e.g. `"chirp_3"`); `provider()`
    /// is `"google.vertex.transcription"`. Its requests need a project and a
    /// location and fail in Express mode.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> VertexTranscriptionModel {
        VertexTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("google.vertex.transcription", false),
            self.project_location("transcription"),
        )
    }

    /// A Claude model served by Vertex AI through `rawPredict` (e.g.
    /// `"claude-sonnet-4-20250514"`); `provider()` is
    /// `"googleVertex.anthropic.messages"`. It uses this provider's
    /// credentials, transport and location, and the shared Anthropic Messages
    /// model.
    #[must_use]
    pub fn anthropic_model(&self, model_id: &str) -> VertexAnthropicModel {
        let resolver = self.resolver.clone();
        anthropic_model::model(
            model_id,
            Arc::new(move || {
                let resolver = resolver.clone();
                Box::pin(async move { resolver.endpoint(Publisher::Anthropic).await })
            }),
            self.fetch.clone(),
            self.transform_request_body.clone(),
        )
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for VertexProvider {
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
        Some(Ok(Arc::new(self.transcription(model_id))))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_must_be_one_dns_label() {
        for ok in ["us-central1", "global", "us", "eu", "europe-west4"] {
            assert!(is_valid_hostname_part(ok), "{ok}");
        }
        for bad in [
            "",
            "evil.example",
            "a/b",
            "-x",
            "x-",
            "a b",
            "us:443",
            "a@b",
        ] {
            assert!(!is_valid_hostname_part(bad), "{bad}");
        }
    }

    #[test]
    fn hosts_follow_the_location() {
        assert_eq!(location_host("global"), "aiplatform.googleapis.com");
        assert_eq!(location_host("us"), "aiplatform.us.rep.googleapis.com");
        assert_eq!(location_host("eu"), "aiplatform.eu.rep.googleapis.com");
        assert_eq!(
            location_host("us-central1"),
            "us-central1-aiplatform.googleapis.com"
        );
    }
}
