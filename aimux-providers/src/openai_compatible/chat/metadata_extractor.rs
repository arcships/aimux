//! Vendor metadata of chat responses (`openai-compatible-metadata-extractor.ts`).

use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// Captures vendor-specific metadata from responses (the AI SDK's
/// `MetadataExtractor`). The returned map is merged into `provider_metadata`
/// next to the namespace entry.
pub trait MetadataExtractor: Send + Sync {
    /// Metadata of a non-streaming response body.
    fn extract_metadata(&self, parsed_body: &Value) -> Option<ProviderMetadata>;

    /// A fresh extractor for one streaming response.
    fn create_stream_extractor(&self) -> Box<dyn StreamMetadataExtractor>;
}

/// The per-stream half of [`MetadataExtractor`].
pub trait StreamMetadataExtractor: Send {
    /// Called with every parsed chunk (including the one carrying usage).
    fn process_chunk(&mut self, parsed_chunk: &Value);

    /// Metadata merged into the finish part's `provider_metadata`.
    fn build_metadata(&self) -> Option<ProviderMetadata>;
}
