//! The providerOptions and providerMetadata namespaces of the Gemini family.
//!
//! `@ai-sdk/google` reads `providerOptions.google` and writes response
//! metadata under `google`; the Vertex package reuses the same models and
//! reads `googleVertex` (then `google`, because the shared Gemini model reads
//! it) and writes `googleVertex`. Only these canonical keys exist: the SDK's
//! historical `vertex` alias is not read and not written. This module is the
//! one place that knows the keys, so the models never spell them.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The Gemini API namespace.
pub(crate) const GOOGLE: &str = "google";

/// The Vertex AI namespace.
pub(crate) const GOOGLE_VERTEX: &str = "googleVertex";

/// Which provider's namespaces a model reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Namespace {
    /// The public Gemini API: reads and writes `google`.
    Google,
    /// Vertex AI: reads `googleVertex`, then `google`; writes `googleVertex`.
    Vertex,
}

impl Namespace {
    /// The keys to read, in order of precedence.
    pub(crate) fn read_keys(self) -> &'static [&'static str] {
        match self {
            Self::Google => &[GOOGLE],
            Self::Vertex => &[GOOGLE_VERTEX, GOOGLE],
        }
    }

    /// The keys response metadata is written under.
    pub(crate) fn write_keys(self) -> &'static [&'static str] {
        match self {
            Self::Google => &[GOOGLE],
            Self::Vertex => &[GOOGLE_VERTEX],
        }
    }

    /// The first present options object among the read keys.
    pub(crate) fn read(
        self,
        provider_options: Option<&SharedProviderOptions>,
    ) -> Option<&JsonObject> {
        let provider_options = provider_options?;
        self.read_keys()
            .iter()
            .find_map(|key| provider_options.get(*key))
    }

    /// [`read`](Self::read) for `CallOptions::provider_options`.
    pub(crate) fn read_in(
        self,
        provider_options: Option<&SharedProviderOptions>,
    ) -> Option<&JsonObject> {
        self.read(provider_options)
    }

    /// Wrap `payload` as response metadata under every write key.
    pub(crate) fn metadata(self, payload: Value) -> ProviderMetadata {
        provider_namespace(self.write_keys()[0], payload)
    }
}

/// The options under the public Gemini key only (`providerOptions.google`),
/// for the surfaces the AI SDK keys by `google` alone (embeddings, images,
/// files).
pub(crate) fn google_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(GOOGLE)
}

/// `{ "google": payload }`.
pub(crate) fn google_metadata(payload: Value) -> ProviderMetadata {
    Namespace::Google.metadata(payload)
}

/// The options under the Vertex key only (`googleVertex`), for the Vertex
/// surfaces that never fall back to `google`.
pub(crate) fn vertex_options(
    provider_options: Option<&SharedProviderOptions>,
) -> Option<&JsonObject> {
    provider_options?.get(GOOGLE_VERTEX)
}

/// Response metadata under the Vertex key, as a metadata map.
pub(crate) fn vertex_metadata_map(payload: &Value) -> ProviderMetadata {
    Namespace::Vertex.metadata(payload.clone())
}
