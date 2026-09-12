//! Single-exchange GET helper aligned with AI SDK.

use aimux_core::AiMuxError;

use crate::http::{HttpBody, HttpMethod, HttpRequest};
use crate::response_handler::{ResponseHandler, ResponseHandlerOutput};

/// GET one URL and dispatch its response to exactly one response handler.
///
/// # Errors
///
/// Returns transport, response-handler, or caller-abort failures.
pub async fn get_from_api<T>(
    request: HttpRequest,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    crate::post_to_api::call_to_api(
        request.prepare(HttpMethod::Get, HttpBody::Empty),
        serde_json::json!({}),
        successful_response_handler,
        failed_response_handler,
    )
    .await
}
