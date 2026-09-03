//! Successful and failed response handlers, aligned with AI SDK.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use serde::de::DeserializeOwned;

use aimux_core::{AiMuxError, ApiCallError};

use crate::extract_response_headers::extract_response_headers;
use crate::read_response_with_size_limit::{
    DEFAULT_MAX_DOWNLOAD_SIZE, DEFAULT_MAX_JSON_RESPONSE_SIZE, read_response_with_size_limit,
};

/// Input supplied to one response handler.
pub struct ResponseHandlerInput {
    pub url: String,
    pub request_body_values: serde_json::Value,
    pub response: reqwest::Response,
    pub abort_signal: Option<aimux_core::AbortSignal>,
    /// Per-request override of the successful-JSON-body size cap, forwarded
    /// from `HttpRequest::max_json_response_bytes`. Only
    /// [`create_json_response_handler`] consults this; other handlers ignore
    /// it.
    pub max_json_response_bytes: Option<usize>,
}

/// A parsed value plus response metadata.
#[derive(Debug)]
pub struct ResponseHandlerOutput<T> {
    pub value: T,
    pub raw_value: Option<serde_json::Value>,
    pub response_headers: std::collections::HashMap<String, String>,
}

type HandlerFuture<T> =
    Pin<Box<dyn Future<Output = Result<ResponseHandlerOutput<T>, AiMuxError>> + Send>>;
type HandlerFn<T> = dyn FnOnce(ResponseHandlerInput) -> HandlerFuture<T> + Send;

/// Provider-specific fields extracted from a failed response body.
pub struct ProviderErrorParts {
    pub message: String,
    pub provider_code: Option<String>,
}

/// One-shot async response handler. Exactly one of the successful/failed
/// handlers passed to an API call is consumed.
pub struct ResponseHandler<T> {
    handler: Box<HandlerFn<T>>,
    /// Whether the handler keeps the response body open as a stream. A
    /// streaming exchange must not be bounded by the per-exchange response
    /// timeout in `call_to_api`.
    streaming: bool,
}

impl<T> ResponseHandler<T> {
    /// Create a handler from an async closure.
    pub fn new<F, Fut>(handler: F) -> Self
    where
        F: FnOnce(ResponseHandlerInput) -> Fut + Send + 'static,
        Fut: Future<Output = Result<ResponseHandlerOutput<T>, AiMuxError>> + Send + 'static,
    {
        Self {
            handler: Box::new(move |input| Box::pin(handler(input))),
            streaming: false,
        }
    }

    /// Mark the handler as streaming: it hands the response body onward as a
    /// stream instead of reading it to completion, so the per-exchange
    /// response timeout must not cover body consumption.
    #[must_use]
    pub fn streaming(mut self) -> Self {
        self.streaming = true;
        self
    }

    pub(crate) fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Consume the handler and process one response.
    ///
    /// # Errors
    ///
    /// Returns the handler's contextual API, parse, or caller-abort failure.
    pub async fn handle(
        self,
        input: ResponseHandlerInput,
    ) -> Result<ResponseHandlerOutput<T>, AiMuxError> {
        (self.handler)(input).await
    }
}

/// Parse a successful JSON response using the endpoint's response type.
///
/// The body is size-limited to [`DEFAULT_MAX_JSON_RESPONSE_SIZE`] (or
/// `HttpRequest::max_json_response_bytes`, when the caller overrides it) —
/// deliberately smaller than the binary-download bound, since a JSON success
/// body is deserialized straight into `T` and held alongside the raw bytes.
#[must_use]
pub fn create_json_response_handler<T>() -> ResponseHandler<T>
where
    T: DeserializeOwned + Send + 'static,
{
    ResponseHandler::new(|input| async move {
        let status = input.response.status().as_u16();
        let headers = extract_response_headers(input.response.headers());
        let max_bytes = input
            .max_json_response_bytes
            .unwrap_or(DEFAULT_MAX_JSON_RESPONSE_SIZE);
        let body = read_response_with_size_limit(
            input.response,
            &input.url,
            &input.request_body_values,
            max_bytes,
            input.abort_signal.as_ref(),
        )
        .await?;
        // Deserialize straight into `T` instead of parsing a
        // `serde_json::Value` first and converting that — the previous
        // two-step parse cloned the intermediate `Value` tree, holding
        // bytes + `Value` + `T` at once. `raw_value` (needed by callers such
        // as `Usage.raw`) is a best-effort second parse: `T`'s successful
        // deserialization already proves `body` is valid JSON, so this
        // cannot fail in a way that changes the outcome.
        let value = serde_json::from_slice::<T>(&body).map_err(|error| {
            AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(status),
                response_body: Some(String::from_utf8_lossy(&body).into_owned()),
                response_headers: Some(headers.clone()),
                ..ApiCallError::new(
                    format!("Invalid JSON response: {error}"),
                    input.url.clone(),
                    input.request_body_values.clone(),
                )
            }))
        })?;
        let raw_value = serde_json::from_slice::<serde_json::Value>(&body).ok();
        Ok(ResponseHandlerOutput {
            value,
            raw_value,
            response_headers: headers,
        })
    })
}

/// Public error bodies are capped at this size: `ApiCallError` crosses the
/// FFI as a serialized string and is persisted by recordings, so the body
/// must stay bounded.
const ERROR_BODY_PUBLIC_CAP: usize = 64 * 1024;

/// Parse bound for error bodies: enough headroom that an oversize-but-valid
/// provider error JSON still reaches the mapper (a body truncated mid-JSON
/// can never parse), while staying bounded — draining an unbounded error
/// body is a DoS vector.
const ERROR_BODY_PARSE_CAP: usize = 1024 * 1024;

/// Decode an error body for the public `response_body`, enforcing
/// [`ERROR_BODY_PUBLIC_CAP`] *after* lossy UTF-8 decoding: U+FFFD expands an
/// invalid byte to 3 bytes, so capping raw bytes alone can still produce a
/// ~192 KiB string.
fn error_body_string(body: &[u8], read_truncated: bool) -> String {
    let mut value = String::from_utf8_lossy(body).into_owned();
    let mut truncated = read_truncated;
    if value.len() > ERROR_BODY_PUBLIC_CAP {
        let mut end = ERROR_BODY_PUBLIC_CAP;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        truncated = true;
    }
    if truncated {
        // A cut can land inside a replacement run; drop partial noise before
        // the marker.
        while value.ends_with('\u{FFFD}') {
            value.pop();
        }
        value.push_str("…(truncated)");
    }
    value
}

/// Handler for the `{ "error": { "message", "type" | "code" } }` error shape,
/// which several providers share verbatim. Providers whose error JSON differs
/// keep their own mapping via [`create_json_error_response_handler`].
#[must_use]
pub fn create_standard_json_error_response_handler() -> ResponseHandler<AiMuxError> {
    create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        ProviderErrorParts {
            message: error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("type")
                .or_else(|| error.get("code"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        }
    })
}

/// Parse an error JSON and map its data to a provider message and code.
#[must_use]
pub fn create_json_error_response_handler<F>(
    error_to_message_and_code: F,
) -> ResponseHandler<AiMuxError>
where
    F: Fn(&serde_json::Value) -> ProviderErrorParts + Send + 'static,
{
    ResponseHandler::new(move |input| async move {
        let status = input.response.status().as_u16();
        let status_text = input
            .response
            .status()
            .canonical_reason()
            .unwrap_or_default()
            .to_string();
        let headers = extract_response_headers(input.response.headers());
        // Best-effort, truncating read: an oversize or dying error body must
        // not replace the provider's actual error message.
        let (body, truncated) = crate::read_response_with_size_limit::read_error_body_truncated(
            input.response,
            ERROR_BODY_PARSE_CAP,
            input.abort_signal.as_ref(),
        )
        .await?;
        let response_body = error_body_string(&body, truncated);
        let parsed_data = serde_json::from_slice::<serde_json::Value>(&body).ok();
        let parts = parsed_data
            .as_ref()
            .map(error_to_message_and_code)
            .unwrap_or_else(|| ProviderErrorParts {
                message: status_text.clone(),
                provider_code: None,
            });
        let message = if parts.message.trim().is_empty() {
            status_text
        } else {
            parts.message
        };
        let error = AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            provider_code: parts.provider_code,
            response_body: Some(response_body),
            response_headers: Some(headers.clone()),
            data: parsed_data.map(crate::logging::redact_error_context),
            is_retryable: aimux_core::error::is_retryable_status(status),
            ..ApiCallError::new(message, input.url, input.request_body_values)
        }));
        Ok(ResponseHandlerOutput {
            value: error,
            raw_value: None,
            response_headers: headers,
        })
    })
}

/// Build the `ApiCallError` for an error event delivered *inside* a stream.
///
/// The event rides on an already-successful response, so no HTTP status is
/// fabricated: `status_code` is set only when the payload itself
/// carried a numeric HTTP status, and retryability derives from that status
/// alone. A `retry_after_ms` / `retry_after` field in the payload is
/// surfaced through the response-headers channel so
/// `AiMuxError::retry_after_hint()` observes it.
///
/// Callers reach this from inside a live stream holding the *raw* request
/// body and URL they built — unlike the exchange path, where `post_*_to_api`
/// redacts before any handler runs. Redaction and URL sanitization therefore
/// happen here, so credentials and data URLs never leak into the public
/// error (or the recordings that persist it).
#[must_use]
pub fn stream_error_api_call(
    message: impl Into<String>,
    provider_code: Option<String>,
    status_code: Option<u16>,
    error_payload: &serde_json::Value,
    url: impl Into<String>,
    request_body_values: serde_json::Value,
    mut response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    // Real headers win; the payload hint fills in only when absent (an
    // in-stream rate-limit error rides on a 200 whose headers carry none).
    if !response_headers.contains_key("retry-after-ms")
        && !response_headers.contains_key("retry-after")
    {
        if let Some(ms) = error_payload
            .get("retry_after_ms")
            .and_then(serde_json::Value::as_f64)
        {
            response_headers.insert("retry-after-ms".into(), ms.to_string());
        } else if let Some(seconds) = error_payload
            .get("retry_after")
            .and_then(serde_json::Value::as_f64)
        {
            response_headers.insert("retry-after".into(), seconds.to_string());
        }
    }
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code,
        provider_code,
        response_body: Some(error_payload.to_string()),
        response_headers: Some(response_headers),
        data: Some(crate::logging::redact_error_context(error_payload.clone())),
        is_retryable: status_code.is_some_and(aimux_core::error::is_retryable_status),
        ..ApiCallError::new(
            message,
            crate::http::sanitized_request_url(&url.into()),
            crate::logging::redact_error_context(request_body_values),
        )
    }))
}

/// Read a successful response as bytes.
#[must_use]
pub fn create_binary_response_handler() -> ResponseHandler<Bytes> {
    ResponseHandler::new(|input| async move {
        let headers = extract_response_headers(input.response.headers());
        let value = read_response_with_size_limit(
            input.response,
            &input.url,
            &input.request_body_values,
            DEFAULT_MAX_DOWNLOAD_SIZE,
            input.abort_signal.as_ref(),
        )
        .await?;
        Ok(ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: headers,
        })
    })
}

/// Parse an SSE response and deserialize each `data:` field as `T`.
#[must_use]
pub fn create_event_source_response_handler<T>()
-> ResponseHandler<BoxStream<'static, Result<T, AiMuxError>>>
where
    T: DeserializeOwned + Send + 'static,
{
    ResponseHandler::new(|input| async move {
        let headers = extract_response_headers(input.response.headers());
        let output_headers = headers.clone();
        let url = input.url;
        let request_body_values = input.request_body_values;
        let signal = input.abort_signal;
        let body_url = url.clone();
        let body_request_body_values = request_body_values.clone();
        let body = input.response.bytes_stream().map(move |result| {
            result.map_err(|error| {
                AiMuxError::ApiCall(Box::new(ApiCallError {
                    response_headers: Some(headers.clone()),
                    is_retryable: true,
                    ..ApiCallError::new(
                        error.to_string(),
                        body_url.clone(),
                        body_request_body_values.clone(),
                    )
                }))
            })
        });

        let stream_headers = output_headers.clone();
        let stream_url = url.clone();
        let stream_request_body_values = request_body_values.clone();
        let value: BoxStream<'static, Result<T, AiMuxError>> = Box::pin(async_stream::stream! {
            let events = aimux_stream::SseStream::new(body);
            futures::pin_mut!(events);
            loop {
                let item = match signal.as_ref() {
                    Some(signal) => tokio::select! {
                        biased;
                        () = signal.cancelled() => {
                            yield Err(AiMuxError::from_abort_signal(signal));
                            break;
                        }
                        item = events.next() => item,
                    },
                    None => events.next().await,
                };
                let Some(item) = item else {
                    break;
                };
                match item {
                    Ok(event) if event.data == "[DONE]" => continue,
                    Ok(event) => yield serde_json::from_str::<T>(&event.data).map_err(AiMuxError::from),
                    Err(aimux_stream::SseError::Stream(message)) => {
                        // Preserve response transport failures as ApiCallError
                        // items. Framing/parser failures remain JsonParse below.
                        yield Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                            response_headers: Some(stream_headers.clone()),
                            is_retryable: true,
                            ..ApiCallError::new(
                                message,
                                stream_url.clone(),
                                stream_request_body_values.clone(),
                            )
                        })));
                        break;
                    }
                    Err(error) => yield Err(AiMuxError::JsonParse(error.to_string())),
                }
            }
        });
        Ok(ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: output_headers,
        })
    })
    .streaming()
}

/// Build an error using only status, headers and body.
#[must_use]
pub fn create_status_code_error_response_handler() -> ResponseHandler<AiMuxError> {
    ResponseHandler::new(|input| async move {
        let status = input.response.status().as_u16();
        let message = input
            .response
            .status()
            .canonical_reason()
            .unwrap_or_default()
            .to_string();
        let headers = extract_response_headers(input.response.headers());
        // Same best-effort contract as the JSON error handler: an oversize
        // or dying body must not replace the provider's actual error text
        // with a size-limit failure.
        let (body, truncated) = crate::read_response_with_size_limit::read_error_body_truncated(
            input.response,
            ERROR_BODY_PUBLIC_CAP,
            input.abort_signal.as_ref(),
        )
        .await?;
        let error = AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            response_body: Some(error_body_string(&body, truncated)),
            response_headers: Some(headers.clone()),
            is_retryable: aimux_core::error::is_retryable_status(status),
            ..ApiCallError::new(message, input.url, input.request_body_values)
        }));
        Ok(ResponseHandlerOutput {
            value: error,
            raw_value: None,
            response_headers: headers,
        })
    })
}
