//! The Vertex AI MaaS provider (maas/google-vertex-maas-provider.ts and its
//! `-node` wrapper).

use crate::openai_compatible::{
    CompatibleBodyRewrite, OpenAICompatibleProvider, OpenAICompatibleProviderSettings,
};
use crate::vertex::partner_provider::{VertexMaasProviderSettings, partner_parts};
use aimux_core::error::AiMuxError;
use std::sync::OnceLock;

/// Create a Vertex AI MaaS (Model as a Service) provider: partner and open
/// models over the OpenAI-compatible Chat Completions API. Models report
/// `vertex.maas.<method>`.
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` for an explicit `base_url` that is
/// not an `http(s)` URL with a host. The project, location and token are
/// resolved when a request is made.
pub fn create_google_vertex_maas(
    settings: VertexMaasProviderSettings,
) -> Result<OpenAICompatibleProvider, AiMuxError> {
    let (base_url, fetch_headers) = partner_parts(&settings, true)?;
    crate::openai_compatible::build(
        OpenAICompatibleProviderSettings {
            name: "vertex.maas".into(),
            fetch: settings.fetch,
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
        Some(CompatibleBodyRewrite::VertexMaas),
        crate::shared::aisdk_pins::GOOGLE_VERTEX_OPENAI_COMPATIBLE,
    )
}

/// The default Vertex MaaS provider. Creating it reads nothing from the
/// environment and cannot fail.
pub fn google_vertex_maas() -> &'static OpenAICompatibleProvider {
    static DEFAULT: OnceLock<OpenAICompatibleProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_google_vertex_maas(VertexMaasProviderSettings::default())
            .expect("default Vertex MaaS settings are valid")
    })
}
