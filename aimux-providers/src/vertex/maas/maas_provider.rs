//! The Vertex AI MaaS provider (maas/google-vertex-maas-provider.ts and its
//! `-node` wrapper).

use crate::openai_compatible::{OpenAICompatibleProvider, OpenAICompatibleProviderSettings};
use crate::vertex::partner_provider::{VertexMaasProviderSettings, partner_parts};
use aimux_core::error::AiMuxError;
use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// Upstream's `maxOutputTokensByModel` (google-vertex-maas-provider.ts:16-27):
/// a default `max_tokens` for the models that need one.
fn maas_request_body(mut body: Value) -> Value {
    let default = match body.get("model").and_then(Value::as_str) {
        Some(
            "meta/llama-4-maverick-17b-128e-instruct-maas"
            | "meta/llama-4-scout-17b-16e-instruct-maas",
        ) => 8192,
        _ => return body,
    };
    if let Some(object) = body.as_object_mut()
        && !object.contains_key("max_tokens")
    {
        object.insert("max_tokens".into(), default.into());
    }
    body
}

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
            transform_request_body: Some(Arc::new(maas_request_body)),
            ..Default::default()
        },
        base_url,
        aimux_provider_utils::ProviderHeaders::new(
            aimux_provider_utils::Credential::None,
            aimux_provider_utils::AuthScheme::Bearer,
            Vec::new(),
            None,
        ),
        Some(fetch_headers),
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
