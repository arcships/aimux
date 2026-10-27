//! The response metadata of a completion response (`get-response-metadata.ts`).

use aimux_core::types::ResponseMetadata;

use super::completion_language_model::CompletionResponse;

pub(super) fn metadata(response: &CompletionResponse) -> ResponseMetadata {
    ResponseMetadata {
        id: response.id.clone(),
        timestamp: response
            .created
            .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
        model_id: response.model.clone(),
    }
}
