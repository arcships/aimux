//! Response metadata from the common response fields
//! (`create-language-model-response-metadata.ts`).

use aimux_core::Timestamp;
use aimux_core::types::ResponseMetadata;

/// The response id, the model id and the creation time, given in Unix
/// seconds, as language model response metadata
/// (`createLanguageModelResponseMetadata`).
#[must_use]
pub fn create_language_model_response_metadata(
    id: Option<String>,
    model: Option<String>,
    created: Option<f64>,
) -> ResponseMetadata {
    ResponseMetadata {
        id,
        timestamp: created.and_then(Timestamp::from_unix_seconds),
        model_id: model,
    }
}
