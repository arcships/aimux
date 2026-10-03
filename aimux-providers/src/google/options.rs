//! The providerOptions and providerMetadata namespaces of the Gemini family.
//!
//! `@ai-sdk/google` reads `providerOptions.google` and writes response
//! metadata under `google`; the Vertex package reuses the same models and
//! reads `googleVertex` (then `google`, because the shared Gemini model reads
//! it) and writes `googleVertex`. Only these canonical keys exist: the SDK's
//! historical `vertex` alias is not read and not written. This module is the
//! one place that knows the keys, so the models never spell them.

use serde_json::{Map, Value};

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
    pub(crate) fn read(self, provider_options: Option<&Value>) -> Option<&Value> {
        let provider_options = provider_options?;
        self.read_keys()
            .iter()
            .find_map(|key| provider_options.get(*key))
    }

    /// [`read`](Self::read) for the map form (`CallOptions::provider_options`).
    pub(crate) fn read_in<M>(self, provider_options: Option<&M>) -> Option<&Value>
    where
        M: ProviderOptionsMap,
    {
        let provider_options = provider_options?;
        self.read_keys()
            .iter()
            .find_map(|key| provider_options.lookup(key))
    }

    /// Wrap `payload` as response metadata under every write key.
    pub(crate) fn metadata(self, payload: Value) -> Value {
        let mut map = Map::new();
        for key in self.write_keys() {
            map.insert((*key).to_string(), payload.clone());
        }
        Value::Object(map)
    }
}

pub(crate) use crate::shared::ProviderOptionsMap;

/// The options under the public Gemini key only (`providerOptions.google`),
/// for the surfaces the AI SDK keys by `google` alone (embeddings, images,
/// files).
pub(crate) fn google_options<M: ProviderOptionsMap>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(GOOGLE)
}

/// `{ "google": payload }`.
pub(crate) fn google_metadata(payload: Value) -> Value {
    Namespace::Google.metadata(payload)
}

/// The options under the Vertex key only (`googleVertex`), for the Vertex
/// surfaces that never fall back to `google`.
pub(crate) fn vertex_options<M: ProviderOptionsMap>(
    provider_options: Option<&M>,
) -> Option<&Value> {
    provider_options?.lookup(GOOGLE_VERTEX)
}

/// Response metadata under the Vertex key, as a metadata map.
pub(crate) fn vertex_metadata_map(payload: &Value) -> std::collections::HashMap<String, Value> {
    Namespace::Vertex
        .write_keys()
        .iter()
        .map(|key| ((*key).to_string(), payload.clone()))
        .collect()
}
