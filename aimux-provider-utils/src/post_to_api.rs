//! Single-exchange POST helpers aligned with AI SDK.

use std::time::Duration;

use aimux_core::{AiMuxError, ApiCallError};

use crate::handle_fetch_error::handle_fetch_error;
use crate::http::{HttpBody, HttpMethod, HttpRequest, PreparedRequest};
use crate::multipart::MultipartForm;
use crate::response_handler::{
    RequestBodySource, ResponseHandler, ResponseHandlerInput, ResponseHandlerOutput,
};

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
    let prepared = request.prepare(HttpMethod::Post, HttpBody::Json(body));
    let source = RequestBodySource::HttpBody(prepared.body.clone());
    call_to_api_source(
        prepared,
        source,
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
    let prepared = request.prepare(HttpMethod::Post, HttpBody::Bytes(content, content_type));
    let source = RequestBodySource::Redacted(std::sync::Arc::new(
        crate::logging::redact_error_context(values),
    ));
    call_to_api_source(
        prepared,
        source,
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
    let prepared = request.prepare(HttpMethod::Post, body);
    let source = RequestBodySource::HttpBody(prepared.body.clone());
    call_to_api_source(
        prepared,
        source,
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
    call_to_api_source(
        request,
        RequestBodySource::Redacted(std::sync::Arc::new(request_body_values)),
        successful_response_handler,
        failed_response_handler,
    )
    .await
}

pub(crate) async fn call_to_api_source<T>(
    request: PreparedRequest,
    request_body_source: RequestBodySource,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    // A provider whose endpoint legitimately holds the connection longer
    // (e.g. Replicate `prefer: wait`) declares its own bound on the request.
    let response_timeout = request
        .response_timeout
        .unwrap_or(EXCHANGE_RESPONSE_TIMEOUT);
    call_to_api_with_deadline_source(
        request,
        request_body_source,
        successful_response_handler,
        failed_response_handler,
        response_timeout,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn call_to_api_with_deadline<T>(
    request: PreparedRequest,
    request_body_values: serde_json::Value,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
    response_timeout: Duration,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    call_to_api_with_deadline_source(
        request,
        RequestBodySource::Redacted(std::sync::Arc::new(request_body_values)),
        successful_response_handler,
        failed_response_handler,
        response_timeout,
    )
    .await
}

pub(crate) async fn call_to_api_with_deadline_source<T>(
    request: PreparedRequest,
    request_body_source: RequestBodySource,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
    response_timeout: Duration,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    let url = crate::http::sanitized_request_url(&request.url);
    if successful_response_handler.is_streaming() {
        return execute_exchange(
            request,
            request_body_source,
            successful_response_handler,
            failed_response_handler,
        )
        .await;
    }
    let exchange = execute_exchange(
        request,
        request_body_source.clone(),
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
                request_body_source.materialize(),
            )
        }))),
    }
}

async fn execute_exchange<T>(
    request: PreparedRequest,
    request_body_source: RequestBodySource,
    successful_response_handler: ResponseHandler<T>,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
    crate::logging::auto_init_from_env();
    let url = crate::http::sanitized_request_url(&request.url);
    let response = crate::http::send_request_once(&request)
        .await
        .map_err(|error| handle_fetch_error(error, &url, &request_body_source.materialize()))?;
    let status = response.status().as_u16();
    let response_headers =
        crate::extract_response_headers::extract_response_headers(response.headers());
    let abort_signal = request.abort_signal.clone();
    let max_json_response_bytes = request.max_json_response_bytes;
    // Drop `request` immediately after the wire send so `request.body` (`Arc<HttpBody>`)
    // and `request.wire_body` are released before response body handling / streaming.
    drop(request);

    if !(200..300).contains(&status) {
        let failed_lazy = failed_response_handler.uses_lazy_request_body();
        let input = ResponseHandlerInput {
            url: url.clone(),
            request_body_values: if failed_lazy {
                serde_json::Value::Null
            } else {
                request_body_source.materialize()
            },
            response,
            abort_signal,
            max_json_response_bytes,
        };
        return match failed_response_handler
            .handle_with_source(input, Some(request_body_source.clone()))
            .await
        {
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
                    request_body_source.materialize(),
                )
                .caused_by(error)
            }))),
        };
    }

    // For streaming handlers, materialize/redact once up front while `request_body_source`
    // is the sole remaining owner of `Arc<HttpBody>` (so `HttpBody::Json` is redacted
    // in place and large Base64/image strings are freed before stream consumption).
    let request_body_source = if successful_response_handler.is_streaming() {
        request_body_source.into_stream_source()
    } else {
        request_body_source
    };
    let lazy = successful_response_handler.uses_lazy_request_body();
    let input = ResponseHandlerInput {
        url: url.clone(),
        request_body_values: if lazy {
            serde_json::Value::Null
        } else {
            request_body_source.materialize()
        },
        response,
        abort_signal,
        max_json_response_bytes,
    };

    match successful_response_handler
        .handle_with_source(input, Some(request_body_source.clone()))
        .await
    {
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
                request_body_source.materialize(),
            )
            .caused_by(error)
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
            ..Default::default()
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

    /// A handler error on a 2xx response stays whole as the `ApiCallError`
    /// cause: a nested aimux error, as `contract-tests/dto/AiMuxError/005.json`.
    #[tokio::test]
    async fn a_failed_success_handler_keeps_its_error_as_the_cause() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{"))
            .mount(&server)
            .await;
        let request = HttpRequest {
            url: server.uri(),
            ..Default::default()
        };
        let handler = ResponseHandler::<serde_json::Value>::new(|_| async {
            Err(AiMuxError::json_parse("Unexpected end of JSON input"))
        });
        let error = call_to_api_with_deadline(
            request.prepare(HttpMethod::Post, HttpBody::Json(serde_json::json!({}))),
            serde_json::json!({}),
            handler,
            create_status_code_error_response_handler(),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();

        let vector: serde_json::Value =
            serde_json::from_str(include_str!("../../contract-tests/dto/AiMuxError/005.json"))
                .unwrap();
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["message"], vector["message"]);
        assert_eq!(json["cause"], vector["cause"]);
        assert!(matches!(
            &error,
            AiMuxError::ApiCall(detail)
                if matches!(&detail.cause, Some(aimux_core::ErrorCause::AiMux(_)))
        ));
    }

    /// Streaming handlers are exempt from the exchange response timeout.
    #[test]
    fn event_source_handler_is_marked_streaming() {
        let handler = create_event_source_response_handler::<serde_json::Value>();
        assert!(handler.is_streaming());
        let plain = create_json_response_handler::<serde_json::Value>();
        assert!(!plain.is_streaming());
    }

    /// Streaming responses must release the raw unredacted `Arc<HttpBody>` (and
    /// any large >4 KiB Base64/image prompt strings inside it) at stream start
    /// instead of retaining it across the lifetime of the SSE stream closures.
    #[tokio::test]
    async fn streaming_response_drops_raw_http_body_before_stream_consumption_and_truncates_large_strings()
     {
        use futures::StreamExt;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: {\"chunk\":1}\n\ndata: [DONE]\n\n"),
            )
            .mount(&server)
            .await;

        let request = HttpRequest {
            url: server.uri(),
            ..Default::default()
        };
        let large_b64 = "A".repeat(64 * 1024);
        let body = serde_json::json!({
            "api_key": "sk-secret-must-not-leak",
            "image_b64": large_b64,
        });
        let prepared = request.prepare(HttpMethod::Post, HttpBody::Json(body));
        let weak_raw_body = std::sync::Arc::downgrade(&prepared.body);
        let source = RequestBodySource::HttpBody(prepared.body.clone());

        let mut output = call_to_api_source(
            prepared,
            source,
            create_event_source_response_handler::<serde_json::Value>(),
            create_status_code_error_response_handler(),
        )
        .await
        .expect("200 SSE response should succeed");

        assert!(
            weak_raw_body.upgrade().is_none(),
            "raw Arc<HttpBody> (and its 64 KiB Base64 string) must be dropped before polling SSE stream"
        );

        let first = output
            .value
            .next()
            .await
            .expect("one SSE event")
            .expect("valid JSON event");
        assert_eq!(first["chunk"], 1);
        assert!(output.value.next().await.is_none());
    }

    /// Lazy (`ResponseHandler::new_with_source`) and eager (`ResponseHandler::new`)
    /// error handlers must produce identical redacted `request_body_values`.
    #[tokio::test]
    async fn lazy_and_eager_error_handlers_produce_identical_redacted_request_body_values() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&server)
            .await;

        let body = serde_json::json!({
            "authorization": "Bearer secret-token",
            "prompt": "B".repeat(8192),
            "temperature": 0.5,
        });

        let req_lazy = HttpRequest {
            url: server.uri(),
            ..Default::default()
        };
        let err_lazy = post_json_to_api(
            req_lazy,
            body.clone(),
            create_json_response_handler::<serde_json::Value>(),
            create_status_code_error_response_handler(),
        )
        .await
        .unwrap_err();

        let req_eager = HttpRequest {
            url: server.uri(),
            ..Default::default()
        };
        let eager_failed_handler = ResponseHandler::new(|input| async move {
            Ok(ResponseHandlerOutput {
                value: AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(input.response.status().as_u16()),
                    ..ApiCallError::new("bad request", input.url, input.request_body_values)
                })),
                raw_value: None,
                response_headers: std::collections::HashMap::new(),
            })
        });
        let err_eager = post_json_to_api(
            req_eager,
            body,
            create_json_response_handler::<serde_json::Value>(),
            eager_failed_handler,
        )
        .await
        .unwrap_err();

        let (AiMuxError::ApiCall(lazy_detail), AiMuxError::ApiCall(eager_detail)) =
            (err_lazy, err_eager)
        else {
            panic!("expected ApiCall errors");
        };
        assert_eq!(
            lazy_detail.request_body_values,
            eager_detail.request_body_values
        );
        assert_eq!(
            lazy_detail.request_body_values["authorization"],
            serde_json::json!("[REDACTED]")
        );
        assert_eq!(
            lazy_detail.request_body_values["prompt"],
            serde_json::json!("[STRING 8192 bytes]")
        );
    }
}
