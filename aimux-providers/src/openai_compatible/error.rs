//! The error structure of compatible servers (`openai-compatible-error.ts`),
//! and the errors they send in a body with a success status.

use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::{ApiCallError, ApiCallErrorOrigin};
use aimux_provider_utils::{ProviderErrorParts, ResponseHandler, ResponseHandlerOutput};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Maps a provider error payload to the message and code of the API error
/// (the AI SDK's `errorStructure`).
pub(crate) type ErrorStructure = Arc<dyn Fn(&Value) -> ProviderErrorParts + Send + Sync>;

/// `{ error: { message, type?, param?, code? } }`, the structure nearly every
/// compatible server answers errors with (the AI SDK's default
/// `errorStructure`).
pub(crate) fn default_error_structure(data: &Value) -> ProviderErrorParts {
    let error = data.get("error").unwrap_or(data);
    ProviderErrorParts {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        provider_code: error.get("code").or_else(|| error.get("type")).and_then(
            |value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            },
        ),
    }
}

/// The successful-response handler of a non-streaming call: the JSON body as
/// `T`, unless it is an error the server sent with a success status. aimux
/// extension: the AI SDK rejects such a body as one without choices.
///
/// A server that starts a long response early (the status, then whitespace
/// that keeps the connection open) can only report a later failure inside
/// that body, as a top-level `error` object. A body with an `error` and no
/// `choices` is such a failure: the `ApiCallError` a failed status gives,
/// with the message and code `structure` reads, but the observed status. The
/// body has no failed status to judge by, so the error is retryable only when
/// its code is one of `retryable_codes`, the codes the vendor documents as
/// worth retrying.
///
/// `T` must read such a body without failing, as the chat response type
/// (every field optional) and `Value` do.
pub(crate) fn json_response_handler<T>(
    structure: ErrorStructure,
    retryable_codes: &'static [String],
) -> ResponseHandler<T>
where
    T: DeserializeOwned + Send + 'static,
{
    ResponseHandler::new_with_source(move |input, body_source| async move {
        let status = input.response.status().as_u16();
        let url = input.url.clone();
        let ResponseHandlerOutput {
            value,
            raw_value,
            response_headers,
        } = aimux_provider_utils::create_json_response_handler::<T>()
            .handle_with_source(input, Some(body_source.clone()))
            .await?;
        let raw = match raw_value {
            Some(raw) if is_error_body(&raw) => raw,
            raw_value => {
                return Ok(ResponseHandlerOutput {
                    value,
                    raw_value,
                    response_headers,
                });
            }
        };
        let parts = structure(&raw);
        let message = if parts.message.trim().is_empty() {
            raw["error"]
                .as_str()
                .unwrap_or("Response contained an error.")
                .to_owned()
        } else {
            parts.message
        };
        let is_retryable = parts
            .provider_code
            .as_ref()
            .is_some_and(|code| retryable_codes.contains(code));
        Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            provider_code: parts.provider_code,
            response_body: Some(raw.to_string()),
            response_headers: Some(response_headers),
            data: Some(aimux_provider_utils::redact_error_context(raw)),
            is_retryable,
            origin: ApiCallErrorOrigin::HttpResponse,
            ..ApiCallError::new(message, url, body_source.materialize())
        })))
    })
}

/// A body with a top-level `error` and no `choices`.
fn is_error_body(body: &Value) -> bool {
    body.get("error").is_some_and(|error| !error.is_null())
        && body
            .get("choices")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
}
