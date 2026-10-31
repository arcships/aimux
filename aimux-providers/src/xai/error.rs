//! The xAI error and response handlers (`xai-error.ts`): the failed-response
//! handler, stream error events, and the JSON error bodies xAI sends with a
//! successful status.

use aimux_core::error::AiMuxError;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) fn xai_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        if let (Some(code), Some(message)) = (
            data.get("code").and_then(Value::as_str),
            data.get("error").and_then(Value::as_str),
        ) {
            return aimux_provider_utils::ProviderErrorParts {
                message: format!("{code}: {message}"),
                provider_code: Some(code.to_owned()),
            };
        }
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("code")
                .or_else(|| error.get("type"))
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Number(value) => Some(value.to_string()),
                    _ => None,
                }),
        }
    })
}

pub(crate) fn xai_stream_error(
    event: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let error = event
        .get("error")
        .or_else(|| {
            event
                .get("response")
                .and_then(|response| response.get("error"))
        })
        .unwrap_or(event);
    let message = error
        .as_str()
        .or_else(|| error.get("message").and_then(Value::as_str))
        .or_else(|| event.get("message").and_then(Value::as_str))
        .unwrap_or("xAI stream failed");
    let status_code = event
        .get("status")
        .or_else(|| event.get("code"))
        .or_else(|| error.get("status"))
        .or_else(|| error.get("code"))
        .and_then(|value| match value {
            Value::String(value)
                if value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                value.parse::<u16>().ok()
            }
            Value::Number(value) => value.as_u64().and_then(|status| u16::try_from(status).ok()),
            _ => None,
        })
        .filter(|status| (400..=599).contains(status));
    let provider_code = event
        .get("code")
        .or_else(|| error.get("code"))
        .or_else(|| error.get("type"))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
    let status_code = status_code.or(match provider_code.as_deref() {
        Some("rate_limit_exceeded" | "rate_limit_error" | "insufficient_quota") => Some(429),
        Some("api_error" | "internal_server_error" | "server_error") => Some(500),
        Some("overloaded_error" | "service_unavailable") => Some(503),
        Some("timeout" | "timeout_error") => Some(504),
        Some("authentication_error" | "invalid_api_key") => Some(401),
        Some("permission_error") => Some(403),
        Some("not_found_error" | "model_not_found") => Some(404),
        Some("bad_request" | "context_length_exceeded" | "invalid_request_error") => Some(400),
        _ => None,
    });
    let is_retryable = (provider_code.as_deref() == Some("insufficient_quota")).then_some(false);
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message: message.to_owned(),
            provider_code,
            status_code,
            is_retryable,
            data: event.clone(),
        },
        event,
        url,
        request_body_values,
        response_headers,
    )
}

pub(crate) fn xai_successful_response_handler<T>() -> aimux_provider_utils::ResponseHandler<T>
where
    T: DeserializeOwned + Send + 'static,
{
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let status = input.response.status().as_u16();
        let url = input.url.clone();
        let request_body_values = input.request_body_values.clone();
        let output = aimux_provider_utils::create_json_response_handler::<Value>()
            .handle(input)
            .await?;
        let headers = output.response_headers.clone();
        let raw = output.value;
        if let Some(error) = raw.get("error").filter(|value| !value.is_null()) {
            let message = error
                .as_str()
                .or_else(|| error.get("message").and_then(Value::as_str))
                .unwrap_or("xAI request failed");
            let provider_code = raw
                .get("code")
                .or_else(|| error.get("code"))
                .or_else(|| error.get("type"))
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Number(value) => Some(value.to_string()),
                    _ => None,
                });
            return Err(AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                status_code: Some(status),
                provider_code,
                response_body: Some(raw.to_string()),
                response_headers: Some(headers),
                origin: aimux_core::ApiCallErrorOrigin::HttpResponse,
                ..aimux_core::ApiCallError::new(message, url, request_body_values)
            })));
        }
        // Borrow `raw` instead of `from_value(raw.clone())`: the whole body is
        // already a `Value` here and is handed back as `raw_value`, so the
        // clone was a second full tree resident at peak.
        let value = serde::Deserialize::deserialize(&raw).map_err(|error: serde_json::Error| {
            AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                status_code: Some(status),
                response_body: Some(raw.to_string()),
                response_headers: Some(headers.clone()),
                cause: Some(aimux_core::ErrorCause::source(&error)),
                origin: aimux_core::ApiCallErrorOrigin::HttpResponse,
                ..aimux_core::ApiCallError::new(
                    format!("Invalid JSON response: {error}"),
                    url,
                    request_body_values,
                )
            }))
        })?;
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: Some(raw),
            response_headers: output.response_headers,
        })
    })
}

/// xAI occasionally returns a JSON error document with a successful status
/// where an SSE response was requested. Classify that response before handing
/// the body to the standard typed event-source handler.
pub(crate) fn xai_event_source_response_handler<T>()
-> aimux_provider_utils::ResponseHandler<futures::stream::BoxStream<'static, Result<T, AiMuxError>>>
where
    T: DeserializeOwned + Send + 'static,
{
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let is_json = input
            .response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"));
        if !is_json {
            return aimux_provider_utils::create_event_source_response_handler::<T>()
                .handle(input)
                .await;
        }

        let status = input.response.status().as_u16();
        let url = input.url.clone();
        let request_body_values = input.request_body_values.clone();
        let output = aimux_provider_utils::create_json_response_handler::<Value>()
            .handle(input)
            .await?;
        let headers = output.response_headers;
        let raw = output.value;
        let error = raw.get("error").filter(|value| !value.is_null());
        let message = error
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("message").and_then(Value::as_str))
            })
            .unwrap_or("Expected an event stream but received JSON");
        let provider_code = raw
            .get("code")
            .or_else(|| error.and_then(|value| value.get("code")))
            .or_else(|| error.and_then(|value| value.get("type")))
            .and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            });
        Err(AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
            status_code: Some(status),
            provider_code,
            response_body: Some(raw.to_string()),
            response_headers: Some(headers),
            origin: aimux_core::ApiCallErrorOrigin::HttpResponse,
            ..aimux_core::ApiCallError::new(message, url, request_body_values)
        })))
    })
    .streaming()
}
