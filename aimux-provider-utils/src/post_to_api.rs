//! Single-exchange POST helpers aligned with AI SDK.

use std::time::Duration;

use aimux_core::{AiMuxError, ApiCallError};

use crate::handle_fetch_error::handle_fetch_error;
use crate::http::{HttpBody, HttpMethod, HttpRequest, PreparedRequest};
use crate::multipart::MultipartForm;
use crate::response_handler::{ResponseHandler, ResponseHandlerInput, ResponseHandlerOutput};

/// POST a JSON request. The body type is fixed by this signature.
///
/// # Errors
///
/// Returns transport, response-handler, or caller-abort failures.
pub async fn post_json_to_api<T>(
    request: HttpRequest,
    body: serde_json::Value,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    let request_body_values = crate::logging::redact_error_context(body.clone());
    call_to_api(
        request.prepare(HttpMethod::Post, HttpBody::Json(body)),
        request_body_values,
        successful_response_handler,
        failed_response_handler,
    )
    .await
}

/// POST a multipart/form-data request. Binary fields are represented in
/// error context by filename, media type, and byte length rather than bytes.
///
/// # Errors
///
/// Returns transport, response-handler, or caller-abort failures.
pub async fn post_form_data_to_api<T>(
    request: HttpRequest,
    form_data: MultipartForm,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    let (content, content_type, values) = form_data.into_parts();
    call_to_api(
        request.prepare(HttpMethod::Post, HttpBody::Bytes(content, content_type)),
        crate::logging::redact_error_context(values),
        successful_response_handler,
        failed_response_handler,
    )
    .await
}

/// POST an explicitly prepared raw body.
///
/// # Errors
///
/// Returns transport, response-handler, or caller-abort failures.
pub async fn post_to_api<T>(
    request: HttpRequest,
    body: HttpBody,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    let request_body_values = crate::logging::redact_request_values(&body);
    call_to_api(
        request.prepare(HttpMethod::Post, body),
        request_body_values,
        successful_response_handler,
        failed_response_handler,
    )
    .await
}

/// Default whole-exchange response timeout for non-streaming exchanges
/// (connect through fully read body), matching the pre-0.4 shared-client
/// behavior. Streaming exchanges are exempt: their body outlives the call.
/// Tripping it yields a retryable error — a hung exchange is a transport
/// fault, and the Core operation deadline (`AiMuxError::Timeout`) remains
/// the non-retryable outer bound.
const EXCHANGE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn call_to_api<T>(
    request: PreparedRequest,
    request_body_values: serde_json::Value,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    // A provider whose endpoint legitimately holds the connection longer
    // (e.g. Replicate `prefer: wait`) declares its own bound on the request.
    let response_timeout = request
        .response_timeout
        .unwrap_or(EXCHANGE_RESPONSE_TIMEOUT);
    call_to_api_with_deadline(
        request,
        request_body_values,
        successful_response_handler,
        failed_response_handler,
        response_timeout,
    )
    .await
}

pub(crate) async fn call_to_api_with_deadline<T>(
    request: PreparedRequest,
    request_body_values: serde_json::Value,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
    response_timeout: Duration,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    let url = crate::http::sanitized_request_url(&request.url);
    if successful_response_handler.is_streaming() {
        return execute_exchange(
            request,
            request_body_values,
            successful_response_handler,
            failed_response_handler,
        )
        .await;
    }
    let exchange = execute_exchange(
        request,
        request_body_values.clone(),
        successful_response_handler,
        failed_response_handler,
    );
    match tokio::time::timeout(response_timeout, exchange).await {
        Ok(result) => result,
        Err(_) => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            is_retryable: true,
            ..ApiCallError::new(
                format!(
                    "Request exceeded the {}s exchange response timeout",
                    response_timeout.as_secs()
                ),
                url,
                request_body_values,
            )
        }))),
    }
}

async fn execute_exchange<T>(
    request: PreparedRequest,
    request_body_values: serde_json::Value,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    crate::logging::auto_init_from_env();
    let url = crate::http::sanitized_request_url(&request.url);
    let response = crate::http::send_request_once(&request)
        .await
        .map_err(|error| handle_fetch_error(error, &url, &request_body_values))?;
    let status = response.status().as_u16();
    let response_headers =
        crate::extract_response_headers::extract_response_headers(response.headers());
    let input = ResponseHandlerInput {
        url: url.clone(),
        request_body_values: request_body_values.clone(),
        response,
        abort_signal: request.abort_signal.clone(),
    };

    if !(200..300).contains(&status) {
        return match failed_response_handler.handle(input).await {
            Ok(output) => Err(output.value),
            Err(
                error @ (AiMuxError::ApiCall(_) | AiMuxError::Aborted(_) | AiMuxError::Timeout(_)),
            ) => Err(error),
            Err(error) => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(status),
                response_headers: Some(response_headers),
                // A handler that cannot parse a 429/503 body must not erase
                // the status's retryability — retry classification comes from
                // the HTTP status, not from the parse outcome.
                is_retryable: aimux_core::error::is_retryable_status(status),
                ..ApiCallError::new(
                    format!("Failed to process error response: {error}"),
                    url,
                    request_body_values,
                )
            }))),
        };
    }

    match successful_response_handler.handle(input).await {
        Ok(output) => Ok(output),
        Err(error @ (AiMuxError::ApiCall(_) | AiMuxError::Aborted(_) | AiMuxError::Timeout(_))) => {
            Err(error)
        }
        Err(error) => Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            response_headers: Some(response_headers),
            ..ApiCallError::new(
                format!("Failed to process successful response: {error}"),
                url,
                request_body_values,
            )
        }))),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::response_handler::{
        create_event_source_response_handler, create_json_response_handler,
        create_status_code_error_response_handler,
    };

    /// A response slower than the exchange deadline must fail as a retryable
    /// transport error — not hang, and not use the non-retryable
    /// `AiMuxError::Timeout` reserved for Core operation deadlines.
    #[tokio::test]
    async fn slow_response_trips_the_exchange_timeout_as_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true}))
                    .set_delay(Duration::from_millis(500)),
            )
            .mount(&server)
            .await;

        let request = HttpRequest {
            url: server.uri(),
            headers: vec![],
            abort_signal: None,
            call_id: None,
            recording_context: None,
            response_timeout: None,
            validate_url: false,
            trusted_origin: None,
            credentialed_origin: None,
        };
        let error = call_to_api_with_deadline(
            request.prepare(HttpMethod::Post, HttpBody::Json(serde_json::json!({}))),
            serde_json::json!({}),
            create_json_response_handler::<serde_json::Value>(),
            create_status_code_error_response_handler(),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();

        match error {
            AiMuxError::ApiCall(detail) => {
                assert!(detail.is_retryable, "exchange timeout must be retryable");
                assert!(detail.message.contains("exchange response timeout"));
            }
            other => panic!("expected retryable ApiCall, got {other:?}"),
        }
    }

    /// Streaming handlers are exempt from the exchange response timeout.
    #[test]
    fn event_source_handler_is_marked_streaming() {
        let handler = create_event_source_response_handler::<serde_json::Value>();
        assert!(handler.is_streaming());
        let plain = create_json_response_handler::<serde_json::Value>();
        assert!(!plain.is_streaming());
    }
}
