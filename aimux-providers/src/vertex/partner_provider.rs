//! aimux-only code shared by the `maas/` and `xai/` sub-providers (upstream
//! repeats it in both providers).
//!
//! Partner and open models on Vertex AI through its OpenAI-compatible
//! endpoint: the `maas` and `xai` sub-providers of `@ai-sdk/google-vertex`
//! 5.0.100 (`src/maas/google-vertex-maas-provider.ts`,
//! `src/xai/google-vertex-xai-provider.ts` and their `-node` wrappers).
//!
//! Each is `createOpenAICompatible` over
//! `…/projects/{project}/locations/{location}/endpoints/openapi`, with the
//! project from the setting or `GOOGLE_VERTEX_PROJECT` (required) and the
//! location from the setting or `GOOGLE_VERTEX_LOCATION` (default `global`),
//! authenticated with a Google OAuth token like the Anthropic sub-provider.
//! The base URL, the token and the headers are resolved per request.

use crate::openai_compatible::config::{BaseUrl, ResolveBaseUrl, ResolveFetchHeaders};
use crate::vertex::auth_google_auth_library::{GoogleAuth, GoogleAuthOptions};
use crate::vertex::provider_base::{
    LOCATION_ENV_VAR, PROJECT_ENV_VAR, check_location, location_host,
};
use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, combine_headers, load_optional_setting,
    load_setting, validate_base_url,
};
use std::sync::Arc;

/// Settings of [`create_google_vertex_maas`](super::create_google_vertex_maas)
/// and [`create_google_vertex_xai`](super::create_google_vertex_xai)
/// (the AI SDK's `GoogleVertexMaasProviderSettings` and
/// `GoogleVertexXaiProviderSettings`, Node variants).
#[derive(Clone, Default)]
pub struct VertexPartnerProviderSettings {
    /// Google Cloud project. `None` loads `GOOGLE_VERTEX_PROJECT` when a
    /// request is made; with neither the request fails with
    /// `AiMuxError::LoadSetting`.
    pub project: Option<String>,
    /// Google Cloud location. `None` loads `GOOGLE_VERTEX_LOCATION` when a
    /// request is made, else `global`.
    pub location: Option<String>,
    /// Base URL for the API calls; replaces the project/location URL. A
    /// trailing slash is removed and `""` counts as unset.
    pub base_url: Option<String>,
    /// Extra headers on every request, resolved per request. They win over
    /// the per-call headers, a `user-agent` among them is sent as the whole
    /// header, and the `Authorization` header is set after them.
    pub headers: Option<Resolvable<HeaderMapOpt>>,
    /// The transport. `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// How the OAuth token is obtained (application default credentials when
    /// `None`).
    pub google_auth_options: Option<GoogleAuthOptions>,
    /// Overrides the OAuth token generator. A missing token becomes `Bearer null`.
    pub generate_auth_token: Option<Resolvable<Option<String>>>,
}

/// Settings of [`create_google_vertex_maas`](super::create_google_vertex_maas).
pub type VertexMaasProviderSettings = VertexPartnerProviderSettings;
/// Settings of [`create_google_vertex_xai`](super::create_google_vertex_xai).
pub type VertexXaiProviderSettings = VertexPartnerProviderSettings;

impl std::fmt::Debug for VertexPartnerProviderSettings {
    /// Never prints header values or tokens.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexPartnerProviderSettings")
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

/// The base URL of a partner sub-provider with no explicit `base_url`, built
/// from the project and location read when a request is made.
#[derive(Clone, Debug)]
struct PartnerBaseUrl {
    project: Option<String>,
    location: Option<String>,
    /// MaaS picks the location's host (google-vertex-maas-provider.ts:79-93, :103-118);
    /// xAI always calls the global host (google-vertex-xai-provider.ts:159-164).
    regional_host: bool,
}

impl ResolveBaseUrl for PartnerBaseUrl {
    /// The base URL of one request.
    ///
    /// # Errors
    ///
    /// `AiMuxError::LoadSetting` when no project is set;
    /// `AiMuxError::InvalidArgument` when a MaaS location is not one DNS label.
    fn resolve(&self) -> Result<String, AiMuxError> {
        let project = load_setting(self.project.as_deref(), PROJECT_ENV_VAR, "project")?;
        let location = load_optional_setting(self.location.as_deref(), LOCATION_ENV_VAR)
            .unwrap_or_else(|| "global".to_string());
        let host = if self.regional_host {
            check_location(&location)?;
            location_host(&location)
        } else {
            "aiplatform.googleapis.com".to_string()
        };
        Ok(format!(
            "https://{host}/v1/projects/{project}/locations/{location}/endpoints/openapi"
        ))
    }
}

/// The headers the Node wrapper's fetch adds over the request's own: the
/// provider headers, then `Authorization` with the Google OAuth token. The
/// Node wrapper passes no headers to `createOpenAICompatible` and spreads the
/// resolved headers, then `Authorization`, over the request's headers in its
/// fetch, so they win over the per-call headers, and a `user-agent` among
/// them replaces the whole header (google-vertex-maas-provider-node.ts:34-58,
/// google-vertex-xai-provider-node.ts:33-55).
#[derive(Clone)]
struct PartnerHeaders {
    auth: Arc<GoogleAuth>,
    user: Option<HeadersFn>,
}

#[async_trait::async_trait]
impl ResolveFetchHeaders for PartnerHeaders {
    /// The headers of one request.
    ///
    /// # Errors
    ///
    /// The token or the caller's header producer error.
    async fn resolve(&self) -> Result<HeaderMapOpt, AiMuxError> {
        let token = self.auth.access_token().await?;
        let user = match &self.user {
            Some(headers) => headers.resolve().await?,
            None => HeaderMapOpt::new(),
        };
        let mut bearer = HeaderMapOpt::new();
        bearer.insert("Authorization".to_string(), Some(format!("Bearer {token}")));
        Ok(combine_headers(&[&user, &bearer]))
    }
}

/// The parts every partner sub-provider shares: the base URL source and the
/// headers the Node wrapper's fetch adds over the request's own (the
/// provider headers, then `Authorization`).
pub(super) fn partner_parts(
    settings: &VertexPartnerProviderSettings,
    regional_host: bool,
) -> Result<(BaseUrl, Arc<dyn ResolveFetchHeaders>), AiMuxError> {
    let explicit = settings
        .base_url
        .as_deref()
        .map(|url| url.trim_end_matches('/'))
        .filter(|url| !url.is_empty());
    let base_url = match explicit {
        Some(url) => BaseUrl::Fixed(validate_base_url(url)?),
        None => BaseUrl::Dynamic(Arc::new(PartnerBaseUrl {
            project: settings.project.clone(),
            location: settings.location.clone(),
            regional_host,
        })),
    };

    let mut auth_options = settings.google_auth_options.clone().unwrap_or_default();
    if let Some(generator) = settings.generate_auth_token.clone() {
        auth_options = GoogleAuthOptions {
            auth_client: Some(generator),
            ..Default::default()
        };
    }
    let headers: Arc<dyn ResolveFetchHeaders> = Arc::new(PartnerHeaders {
        auth: Arc::new(GoogleAuth::new(auth_options, settings.fetch.clone())),
        user: settings.headers.clone(),
    });
    Ok((base_url, headers))
}
