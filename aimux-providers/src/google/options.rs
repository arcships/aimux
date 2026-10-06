//! The providerOptions and providerMetadata namespaces of the Gemini family.
//!
//! `@ai-sdk/google` reads `providerOptions.google` and writes response
//! metadata under `google`; the Vertex package reuses the same models and
//! reads `googleVertex`, then legacy `vertex`, then `google`, and writes
//! metadata under both Vertex keys. Part metadata additionally supports the
//! cross-provider namespace fallback used by the upstream message converter.

use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The Gemini API namespace.
pub(crate) const GOOGLE: &str = "google";

/// The Vertex AI namespace.
pub(crate) const GOOGLE_VERTEX: &str = "googleVertex";
/// Keys the Vertex video model writes its metadata under: the current
/// namespace plus the two legacy keys upstream keeps for compatibility.
pub(crate) const VERTEX_VIDEO_METADATA_KEYS: [&str; 3] = [GOOGLE_VERTEX, "google-vertex", "vertex"];

/// Which provider's namespaces a model reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Namespace {
    /// The public Gemini API: reads and writes `google`.
    Google,
    /// Vertex AI: reads `googleVertex`, `vertex`, then `google`; writes both Vertex keys.
    Vertex,
}

impl Namespace {
    /// The keys to read, in order of precedence.
    pub(crate) fn read_keys(self) -> &'static [&'static str] {
        match self {
            Self::Google => &[GOOGLE],
            Self::Vertex => &[GOOGLE_VERTEX, "vertex", GOOGLE],
        }
    }

    /// The keys response metadata is written under.
    pub(crate) fn write_keys(self) -> &'static [&'static str] {
        match self {
            Self::Google => &[GOOGLE],
            Self::Vertex => &[GOOGLE_VERTEX, "vertex"],
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

    pub(crate) fn read_part(self, options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
        let options = options?;
        let keys: &[&str] = match self {
            Self::Google => &[GOOGLE, GOOGLE_VERTEX, "vertex"],
            Self::Vertex => &[GOOGLE_VERTEX, "vertex", GOOGLE],
        };
        keys.iter().find_map(|key| options.get(*key))
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
        let mut metadata = provider_namespace(self.write_keys()[0], payload.clone())
            .expect("provider metadata must be an object");
        for key in &self.write_keys()[1..] {
            metadata.extend(
                provider_namespace(key, payload.clone())
                    .expect("provider metadata must be an object"),
            );
        }
        metadata
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

/// `deserialize_with` for a zod `.optional()` field: absence is `None`, an
/// explicit `null` is an error.
pub(crate) fn no_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
