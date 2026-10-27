//! The response metadata of a completion response (`get-response-metadata.ts`).

use aimux_core::types::ResponseMetadata;

use super::completion_language_model::CompletionResponse;

pub(super) fn metadata(response: &CompletionResponse) -> ResponseMetadata {
    aimux_provider_utils::create_language_model_response_metadata(
        response.id.clone(),
        response.model.clone(),
        response.created.map(|secs| secs as f64),
    )
}
