//! Bounded response reads.

use bytes::Bytes;
use futures::StreamExt;

use aimux_core::{AiMuxError, ApiCallError};

/// Default maximum buffered response size for binary downloads (2 GiB,
/// matching AI SDK).
pub const DEFAULT_MAX_DOWNLOAD_SIZE: usize = 2 * 1024 * 1024 * 1024;

/// Default maximum buffered response size for a successful JSON body (64
/// MiB). AI SDK reuses its 2 GiB download bound for JSON bodies too, but a
/// JSON success response is held simultaneously as raw bytes, a parsed
/// `serde_json::Value`, and a deserialized struct — a 2 GiB cap lets a single
/// response balloon to several times that in resident memory. This bound is
/// per-request configurable via `HttpRequest::max_json_response_bytes`.
pub const DEFAULT_MAX_JSON_RESPONSE_SIZE: usize = 64 * 1024 * 1024;

/// Read a response incrementally and fail before unbounded allocation.
///
/// # Errors
///
/// Returns an API-call error when the body exceeds `max_bytes` or cannot be
/// read, and an abort error when the caller cancels.
pub async fn read_response_with_size_limit(
    response: reqwest::Response,
    url: &str,
    request_body_values: &serde_json::Value,
    max_bytes: usize,
    abort_signal: Option<&aimux_core::AbortSignal>,
) -> Result<Bytes, AiMuxError> {
    let status = response.status().as_u16();
    let response_headers =
        crate::extract_response_headers::extract_response_headers(response.headers());
    if let Some(length) = response.content_length()
        && length > max_bytes as u64
    {
        return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            status_code: Some(status),
            response_headers: Some(response_headers),
            is_retryable: aimux_core::error::is_retryable_status(status),
            ..ApiCallError::new(
                format!("Response exceeded maximum size of {max_bytes} bytes"),
                url,
                request_body_values.clone(),
            )
        })));
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let next = match abort_signal {
            Some(signal) => tokio::select! {
                biased;
                () = signal.cancelled() => return Err(AiMuxError::from_abort_signal(signal)),
                next = stream.next() => next,
            },
            None => stream.next().await,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| {
            AiMuxError::ApiCall(Box::new(ApiCallError {
                // Transport failure mid-body: retryable, and carrying NO
                // status — the exchange died before a complete response, and
                // an "HTTP 200: failed to read body" would contradict the
                // transport-error contract (pre-response transport path and
                // the SSE body path both report status-less errors).
                status_code: None,
                response_headers: Some(response_headers.clone()),
                is_retryable: true,
                ..ApiCallError::new(
                    format!("Failed to read response body: {error}"),
                    url,
                    request_body_values.clone(),
                )
            }))
        })?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(status),
                response_headers: Some(response_headers),
                is_retryable: aimux_core::error::is_retryable_status(status),
                ..ApiCallError::new(
                    format!("Response exceeded maximum size of {max_bytes} bytes"),
                    url,
                    request_body_values.clone(),
                )
            })));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(bytes))
}

/// Best-effort read of an error body: keep the first `max_bytes`, mark
/// truncation instead of failing, and surface what was collected even when
/// the connection dies mid-read. The provider's error text is diagnosis
/// evidence — replacing it with a size-limit error would destroy the actual
/// failure reason.
///
/// # Errors
///
/// Returns an abort error when the caller cancels; never fails on read or
/// size problems.
pub(crate) async fn read_error_body_truncated(
    response: reqwest::Response,
    max_bytes: usize,
    abort_signal: Option<&aimux_core::AbortSignal>,
) -> Result<(Vec<u8>, bool), AiMuxError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        let next = match abort_signal {
            Some(signal) => tokio::select! {
                biased;
                () = signal.cancelled() => return Err(AiMuxError::from_abort_signal(signal)),
                next = stream.next() => next,
            },
            None => stream.next().await,
        };
        let Some(chunk) = next else {
            break;
        };
        // A dead connection mid-error-body: surface what we have.
        let Ok(chunk) = chunk else {
            break;
        };
        let remaining = max_bytes.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            // Stop reading: draining an unbounded error body is a DoS vector.
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, truncated))
}
