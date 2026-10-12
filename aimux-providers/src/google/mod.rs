//! Google Gemini provider.
//!
//! [`create_google`] is the Rust form of the AI SDK's `createGoogle`: it takes
//! [`GoogleProviderSettings`], validates the base URL, fixes the provider name
//! and returns a [`GoogleProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `GOOGLE_GENERATIVE_AI_API_KEY`. [`google()`] is the default instance.
//!
//! The models speak Google's Generative Language API
//! (`generativelanguage.googleapis.com/v1beta`). providerOptions are read from
//! `providerOptions.google` and response metadata is written under `google`
//! (see `options`). Vertex AI serves the same request format and shares the
//! conversion in [`convert`].
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
pub(crate) mod options;
pub mod types;
pub mod utils;
pub mod video;

pub use embedding::GoogleEmbeddingModel;
pub use files::GoogleFiles;
pub use image::{GoogleImageModel, GoogleImageSettings};
pub use model::GoogleModel;
pub use video::GoogleVideoModel;

use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use serde_json::Value;

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::error::AiMuxError;
use aimux_core::files_model::Files;
use aimux_core::image_model::ImageModel;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::model_catalogue::RuntimeModel;
use aimux_core::provider::{Provider, ProviderDiscovery};
use aimux_core::video_model::VideoModel;
use aimux_provider_utils::{FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, validate_base_url};

use crate::shared::{AuthScheme, Credential, Endpoint, EndpointConfig, credential_headers};

pub(crate) fn google_stream_error(
    error: &types::GoogleError,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let status_code = error
        .code
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let data = serde_json::json!({
        "error": {
            "code": error.code,
            "message": error.message,
            "status": error.status,
        }
    });
    aimux_provider_utils::stream_error_api_call(
        error.message.clone(),
        error.status.clone(),
        status_code,
        &data,
        url,
        request_body_values,
        response_headers,
    )
}

pub(crate) fn google_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const API_KEY_ENV_VAR: &str = "GOOGLE_GENERATIVE_AI_API_KEY";
const DEFAULT_NAME: &str = "google.generative-ai";

/// The media types the API fetches from an external `https` URL itself
/// (`supportedExternalUrlMediaTypes` in `createGoogle`).
const EXTERNAL_URL_MEDIA_TYPES: [&str; 22] = [
    "text/html",
    "text/css",
    "text/plain",
    "text/xml",
    "text/csv",
    "text/rtf",
    "text/javascript",
    "application/json",
    "application/pdf",
    "image/bmp",
    "image/jpeg",
    "image/png",
    "image/webp",
    "video/mp4",
    "video/mpeg",
    "video/quicktime",
    "video/avi",
    "video/x-flv",
    "video/mpg",
    "video/webm",
    "video/wmv",
    "video/3gpp",
];

/// `supportsExternalFileUrls`: `/(^|\/)gemini-/` matches and
/// `/(^|\/)gemini-2\.0/` does not.
fn supports_external_file_urls(model_id: &str) -> bool {
    let starts_a_segment =
        |prefix: &str| model_id.starts_with(prefix) || model_id.contains(&format!("/{prefix}"));
    starts_a_segment("gemini-") && !starts_a_segment("gemini-2.0")
}

/// The URL patterns the API fetches itself (`getSupportedUrls` in
/// `createGoogle`): Files API URLs and YouTube links for every media type,
/// plus external `https` URLs for the models that accept them.
fn supported_urls(base_url: &str, model_id: Option<&str>) -> SupportedUrls {
    let include_external = model_id.is_none_or(supports_external_file_urls);
    let pattern = |source: &str| regex::Regex::new(source).expect("static pattern");
    let mut urls = std::collections::HashMap::new();
    urls.insert(
        "*".to_string(),
        vec![
            pattern(r"^https://generativelanguage\.googleapis\.com/v1beta/files/.*$"),
            pattern(&format!("^{}/files/.*$", regex::escape(base_url))),
            pattern(r"^https://(?:www\.)?youtube\.com/watch\?v=[\w-]+(?:&[\w=&.-]*)?$"),
            pattern(r"^https://youtu\.be/[\w-]+(?:\?[\w=&.-]*)?$"),
        ],
    );
    if include_external {
        let https = pattern(r"^https://.*$");
        for media_type in EXTERNAL_URL_MEDIA_TYPES {
            urls.insert(media_type.to_string(), vec![https.clone()]);
        }
    }
    SupportedUrls(urls)
}

/// Settings of [`create_google`] (the AI SDK's `GoogleProviderSettings`).
///
/// Every field is optional. Nothing here is evaluated when the provider is
/// created except `base_url` and `name`; `api_key` and `headers` are
/// evaluated on every request.
#[derive(Clone, Default)]
pub struct GoogleProviderSettings {
    /// Base URL for the API calls. Default
    /// `https://generativelanguage.googleapis.com/v1beta`; a trailing slash is
    /// removed.
    pub base_url: Option<String>,
    /// The API key, sent as `x-goog-api-key`. `None` loads
    /// `GOOGLE_GENERATIVE_AI_API_KEY` when a request is made and fails that
    /// request with `AiMuxError::LoadApiKey` if it is unset. An explicit value
    /// is used as given, `""` included: it never falls back to the
    /// environment.
    pub api_key: Option<String>,
    /// Extra headers on every request. A `None` value removes the header,
    /// including `x-goog-api-key`. Per-call headers win over these.
    pub headers: Option<HeaderMapOpt>,
    /// The provider name, the `provider()` string of the language, embedding
    /// image, video and files interfaces. Default `"google.generative-ai"`.
    pub name: Option<String>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Generates IDs for tool calls and sources that have no provider ID.
    pub generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl std::fmt::Debug for GoogleProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleProviderSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.is_some())
            .field(
                "headers",
                &self.headers.as_ref().map(std::collections::HashMap::len),
            )
            .field("name", &self.name)
            .field("fetch", &self.fetch.is_some())
            .field("generate_id", &self.generate_id.is_some())
            .finish()
    }
}

/// Create a Google Generative AI provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `base_url` is not an `http(s)`
/// URL with a host. That is the only way this fails: the key is loaded per
/// request, not here.
pub fn create_google(settings: GoogleProviderSettings) -> Result<GoogleProvider, AiMuxError> {
    let base_url = match settings.base_url.as_deref() {
        Some(url) => validate_base_url(url)?,
        None => DEFAULT_BASE_URL.to_string(),
    };
    Ok(GoogleProvider {
        name: settings.name.unwrap_or_else(|| DEFAULT_NAME.to_string()),
        base_url,
        headers: aimux_provider_utils::headers::with_user_agent_suffix_fn(
            credential_headers(
                Credential::explicit_or_env(
                    settings.api_key.map(Resolvable::Value),
                    API_KEY_ENV_VAR,
                    "Google Generative AI",
                ),
                AuthScheme::Header("x-goog-api-key"),
                Vec::new(),
                settings.headers,
            ),
            options::GOOGLE,
            "4.0.85",
        ),
        fetch: settings.fetch,
        generate_id: settings.generate_id,
    })
}

/// The default provider: `create_google` with default settings, created on
/// first use. Creating it reads nothing from the environment and cannot fail;
/// a missing key surfaces from the first request instead.
pub fn google() -> &'static GoogleProvider {
    static DEFAULT: OnceLock<GoogleProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google(GoogleProviderSettings::default())
            .expect("default Google settings are always valid")
    })
}

/// A Google Generative AI provider (the AI SDK's `GoogleProvider`). Cheap to
/// clone the models out of; it holds no HTTP client.
pub struct GoogleProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
}

impl GoogleProvider {
    /// The model configuration reporting `provider` as its identity.
    fn model_config(&self, provider: String) -> EndpointConfig {
        let base_url = self.base_url.clone();
        let headers = self.headers.clone();
        let urls_base = self.base_url.clone();
        EndpointConfig {
            provider,
            endpoint: Arc::new(move || {
                let base_url = base_url.clone();
                let headers = headers.clone();
                Box::pin(async move {
                    Ok(Endpoint {
                        base_url,
                        headers: headers.resolve().await?,
                    })
                })
            }),
            fetch: self.fetch.clone(),
            supported_urls: Arc::new(move |model_id| supported_urls(&urls_base, Some(model_id))),
            transform_request_body: None,
        }
    }

    /// A language model; `provider()` is the provider name
    /// (`"google.generative-ai"` by default).
    #[must_use]
    pub fn chat(&self, model_id: &str) -> GoogleModel {
        GoogleModel::from_config(model_id.to_string(), self.model_config(self.name.clone()))
            .with_generate_id(self.generate_id.clone())
    }

    /// Alias of [`chat`](Self::chat), matching `generativeAI`.
    #[must_use]
    pub fn generative_ai(&self, model_id: &str) -> GoogleModel {
        self.chat(model_id)
    }

    /// A text embedding model (e.g. `"gemini-embedding-001"`); `provider()` is
    /// the provider name.
    #[must_use]
    pub fn embedding(&self, model_id: &str) -> GoogleEmbeddingModel {
        GoogleEmbeddingModel::from_config(
            model_id.to_string(),
            self.model_config(self.name.clone()),
        )
    }

    /// An image generation model (e.g. `"gemini-2.5-flash-image"`); `provider()` is the provider name.
    #[must_use]
    pub fn image(&self, model_id: &str) -> GoogleImageModel {
        self.image_with_settings(model_id, GoogleImageSettings::default())
    }

    /// [`image`](Self::image) with model settings.
    #[must_use]
    pub fn image_with_settings(
        &self,
        model_id: &str,
        settings: GoogleImageSettings,
    ) -> GoogleImageModel {
        GoogleImageModel::from_config(
            model_id.to_string(),
            settings,
            self.model_config(self.name.clone()),
        )
    }

    /// A video generation model (e.g. `"veo-3.0-generate-001"`);
    /// `provider()` is the provider name.
    #[must_use]
    pub fn video(&self, model_id: &str) -> GoogleVideoModel {
        GoogleVideoModel::from_config(model_id.to_string(), self.model_config(self.name.clone()))
    }

    /// The files interface; `provider()` is the provider name.
    #[must_use]
    pub fn files(&self) -> GoogleFiles {
        GoogleFiles::from_config(self.model_config(self.name.clone()))
    }

    /// The provider as a function: the default language model for an id. The
    /// AI SDK's callable provider; the same model as [`chat`](Self::chat) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.chat(model_id))
    }
}

impl Provider for GoogleProvider {
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

    fn video_model(&self, model_id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        Some(Ok(Arc::new(self.video(model_id))))
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        Some(Arc::new(self.files()))
    }
}

impl ProviderDiscovery for GoogleProvider {
    /// `GET {base_url}/models` (Gemini native): one exchange, no retry.
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>> {
        let config = self.model_config(format!("{}.models", self.name));
        Box::pin(async move { list_models_once(&config).await })
    }
}

/// One `GET {base_url}/models` exchange against a Gemini-shaped API: no retry,
/// no recording. Discovery is not a Core operation and the AI SDK has no
/// equivalent, so a failure is reported to the caller as it happened. Shared
/// by Vertex.
///
/// # Errors
///
/// Returns the header-resolution error (a missing key is `LoadApiKey`),
/// `ApiCall` for HTTP/transport failures and `JsonParse` when the body does
/// not deserialize into the models list.
pub(crate) async fn list_models_once(
    config: &EndpointConfig,
) -> Result<Vec<RuntimeModel>, AiMuxError> {
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

    let exchange = config.exchange(None).await?;
    let resp = aimux_provider_utils::get_from_api(
        exchange.with_transport(aimux_provider_utils::HttpRequest {
            url: exchange.url("/models"),
            headers: exchange.headers(),
            ..Default::default()
        }),
        aimux_provider_utils::create_json_response_handler(),
        google_failed_response_handler(),
    )
    .await?;
    let parsed: Resp = resp.value;
    Ok(parsed
        .models
        .into_iter()
        .map(|entry| RuntimeModel {
            // Strip "models/" prefix -> bare model id.
            id: entry
                .name
                .strip_prefix("models/")
                .unwrap_or(&entry.name)
                .to_string(),
            owned_by: entry.display_name,
            created: None,
        })
        .collect())
}
