//! One-exchange request transport.
//!
//! Retry and operation/stream timeouts belong to `aimux-core`. This module
//! sends exactly one request through a [`Fetch`] (the request's own, or the
//! process default) and leaves successful/failed body interpretation to the
//! response handlers selected by the provider.

use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::{Stream, StreamExt, stream::BoxStream};
use serde::{Deserialize, Serialize};

use aimux_core::recording::{
    HttpExchange, HttpRecord, RecordingContext, ResponseRecord, TimingRecord,
};
use aimux_core::{AiMuxError, ApiCallError};

use crate::fetch::{
    Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse, PinnedFetch, RedirectPolicy,
    default_fetch,
};
use crate::handle_fetch_error::fetch_error_to_ai_mux_error;
use crate::logging::{body_logging_enabled, redact_body};

/// Process-wide proxy configuration, fixed before the first HTTP client is built.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub http_url: Option<String>,
    pub https_url: Option<String>,
    pub all_url: Option<String>,
    pub no_proxy: Option<String>,
}

static GLOBAL_PROXY: OnceLock<ProxyConfig> = OnceLock::new();

/// Set proxy configuration before the first HTTP operation.
///
/// The HTTP client snapshots this config when it is first built; the
/// WebSocket connect path (RFC-0034 §2) re-reads it live on every connect,
/// so a late `init_proxy` can affect WS while HTTP keeps its first-built
/// client. Set it before any traffic either way.
pub fn init_proxy(config: ProxyConfig) -> bool {
    GLOBAL_PROXY.set(config).is_ok()
}

/// Return the process-wide proxy configuration (empty default when
/// `init_proxy` was never called). Read by both the HTTP client builder and
/// the WebSocket connect path (RFC-0034 §2), so `ws://`/`wss://` traffic
/// honors the same proxy/no_proxy settings as HTTP.
pub(crate) fn global_proxy() -> ProxyConfig {
    GLOBAL_PROXY.get().cloned().unwrap_or_default()
}

/// Redirect ceiling for the manual validated-download loop.
const MAX_REDIRECTS: usize = 10;

/// Explicit raw request body used only by `post_to_api`.
#[derive(Debug, Clone)]
pub enum HttpBody {
    Json(serde_json::Value),
    Bytes(Vec<u8>, String),
    Empty,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// Metadata shared by `post_json_to_api`, `post_form_data_to_api`,
/// `post_to_api`, and `get_from_api`. Method and body are fixed by the helper
/// signature rather than supplied as runtime fields.
#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub abort_signal: Option<aimux_core::AbortSignal>,
    pub call_id: Option<String>,
    pub recording_context: Option<RecordingContext>,
    /// Per-exchange whole-response hang guard. `None` uses the 30s default;
    /// a provider whose endpoint legitimately holds the connection longer
    /// (e.g. Replicate `prefer: wait`) declares its own bound here. Streaming
    /// exchanges are exempt regardless.
    pub response_timeout: Option<std::time::Duration>,
    /// Per-request override of the successful-JSON-body size cap (default
    /// [`crate::read_response_with_size_limit::DEFAULT_MAX_JSON_RESPONSE_SIZE`]).
    /// Only consulted by [`crate::response_handler::create_json_response_handler`];
    /// binary downloads keep the separate, larger
    /// `DEFAULT_MAX_DOWNLOAD_SIZE` bound regardless of this field.
    pub max_json_response_bytes: Option<usize>,
    /// AI SDK `validateUrl`: set for URLs taken from provider responses
    /// (generated assets, polling and result URLs). The exchange then goes
    /// through the SSRF download guard — target and every DNS answer
    /// validated and pinned, redirects followed manually with each hop
    /// re-validated.
    pub validate_url: bool,
    /// AI SDK `trustedOrigin`: a developer-configured origin (normally the
    /// provider's `base_url`) whose same-origin URLs are exempt from the
    /// address blocklist, so self-hosted deployments serving assets from
    /// their own (possibly private) origin keep working. Reachability only —
    /// never about headers, and never derived from response data.
    pub trusted_origin: Option<String>,
    /// AI SDK `credentialedOrigin`: caller headers (which may carry the
    /// provider API key) are sent only while the target is same-origin with
    /// this value — from the first request and on every redirect hop. `None`
    /// means the caller gates its own headers (e.g. BFL's host allowlist).
    pub credentialed_origin: Option<String>,
    /// Transport for this exchange. `None` resolves [`default_fetch`] when the
    /// request is sent. Ignored when `validate_url` is set: downloads of
    /// provider-supplied URLs always use the SSRF guard's pinned transport,
    /// never a provider-injected one.
    pub fetch: Option<FetchFunction>,
}

/// The per-operation context every model's call options carry into an
/// exchange: cancellation, and (for language models) the call id and
/// recording context the RFC-0031 pipeline threads through.
pub trait ExchangeContext {
    fn abort_signal(&self) -> Option<aimux_core::AbortSignal>;
    /// Only language-model `CallOptions` participate in recording; the other
    /// modalities have no call id to correlate on.
    fn call_id(&self) -> Option<String> {
        None
    }
    fn recording_context(&self) -> Option<RecordingContext> {
        None
    }
}

impl ExchangeContext for aimux_core::options::CallOptions {
    fn abort_signal(&self) -> Option<aimux_core::AbortSignal> {
        self.abort_signal.clone()
    }
    fn call_id(&self) -> Option<String> {
        self.call_id.clone()
    }
    fn recording_context(&self) -> Option<RecordingContext> {
        self.recording_context.clone()
    }
}

macro_rules! exchange_context_abort_only {
    ($($ty:path),* $(,)?) => {
        $(impl ExchangeContext for $ty {
            fn abort_signal(&self) -> Option<aimux_core::AbortSignal> {
                self.abort_signal.clone()
            }
        })*
    };
}

exchange_context_abort_only!(
    aimux_core::search_model::SearchCallOptions,
    aimux_core::speech_model::SpeechCallOptions,
    aimux_core::image_model::ImageCallOptions,
    aimux_core::video_model::VideoCallOptions,
    aimux_core::transcription_model::TranscriptionCallOptions,
    aimux_core::embedding_model::EmbeddingCallOptions,
    aimux_core::reranking_model::RerankingCallOptions,
    aimux_core::files_model::UploadFileCallOptions,
);

impl HttpRequest {
    /// A plain exchange that inherits the operation's cancellation and
    /// recording context. Downloads of provider-supplied URLs set the guard
    /// fields explicitly instead.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        headers: Vec<(String, String)>,
        options: &impl ExchangeContext,
    ) -> Self {
        Self {
            url: url.into(),
            headers,
            abort_signal: options.abort_signal(),
            call_id: options.call_id(),
            recording_context: options.recording_context(),
            ..Self::default()
        }
    }
}

/// SSRF guard configuration carried by a validated exchange
/// (`HttpRequest.validate_url`).
#[derive(Debug, Clone)]
pub(crate) struct DownloadValidation {
    pub(crate) trusted_origin: Option<String>,
    pub(crate) credentialed_origin: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedRequest {
    method: HttpMethod,
    body: HttpBody,
    pub(crate) url: String,
    headers: Vec<(String, String)>,
    pub(crate) abort_signal: Option<aimux_core::AbortSignal>,
    call_id: Option<String>,
    recording_context: Option<RecordingContext>,
    pub(crate) response_timeout: Option<std::time::Duration>,
    pub(crate) max_json_response_bytes: Option<usize>,
    pub(crate) validation: Option<DownloadValidation>,
    pub(crate) fetch: Option<FetchFunction>,
}

impl HttpRequest {
    pub(crate) fn prepare(self, method: HttpMethod, body: HttpBody) -> PreparedRequest {
        // AI SDK gates credentialed headers independently of validateUrl:
        // when the target is off the credentialed origin they never leave,
        // whichever transport the exchange takes.
        let mut headers = self.headers;
        if let Some(origin) = self.credentialed_origin.as_deref()
            && !crate::download_guard::same_origin(&self.url, origin)
        {
            crate::download_guard::retain_user_agent(&mut headers);
        }
        PreparedRequest {
            method,
            body,
            url: self.url,
            headers,
            abort_signal: self.abort_signal,
            call_id: self.call_id,
            recording_context: self.recording_context,
            response_timeout: self.response_timeout,
            max_json_response_bytes: self.max_json_response_bytes,
            validation: self.validate_url.then_some(DownloadValidation {
                trusted_origin: self.trusted_origin,
                credentialed_origin: self.credentialed_origin,
            }),
            fetch: self.fetch,
        }
    }
}

fn api_call_error(request: &PreparedRequest, message: impl Into<String>) -> ApiCallError {
    ApiCallError::new(
        message,
        sanitized_request_url(&request.url),
        crate::logging::redact_request_values(&request.body),
    )
}

struct ExchangeGuard<'a> {
    request: &'a PreparedRequest,
    attempt: u32,
    exchange_index: u32,
    started: Instant,
    armed: bool,
}

impl<'a> ExchangeGuard<'a> {
    fn new(
        request: &'a PreparedRequest,
        attempt: u32,
        exchange_index: u32,
        started: Instant,
    ) -> Self {
        Self {
            request,
            attempt,
            exchange_index,
            started,
            armed: true,
        }
    }

    fn fail(&mut self, error: &str) {
        if self.armed {
            record_failed_exchange(
                self.request,
                self.attempt,
                self.exchange_index,
                self.started.elapsed().as_millis() as u64,
                error,
            );
            record_transport_closed(self.request);
            self.armed = false;
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ExchangeGuard<'_> {
    fn drop(&mut self) {
        self.fail("exchange future cancelled");
    }
}

/// Execute exactly one HTTP exchange. Core owns retry and operation timeout.
pub(crate) async fn send_request_once(
    request: &PreparedRequest,
) -> Result<FetchResponse, AiMuxError> {
    let started = Instant::now();
    let (attempt, exchange_index) = request
        .recording_context
        .as_ref()
        .map(RecordingContext::next_exchange)
        .unwrap_or((1, 1));
    let mut exchange = ExchangeGuard::new(request, attempt, exchange_index, started);
    tracing::debug!(
        target: "aimux_provider_utils::http",
        method = request.method.as_str(),
        url = %sanitized_request_url(&request.url),
        host = %request_host(&request.url),
        body_size = request_body_size(request),
        header_count = request.headers.len(),
        call_id = request.call_id.as_deref().unwrap_or(""),
        "request"
    );
    if body_logging_enabled()
        && let HttpBody::Json(value) = &request.body
    {
        tracing::trace!(
            target: "aimux_provider_utils::http",
            body = %redact_body(&value.to_string()),
            "request_body"
        );
    }

    let sent = if request.validation.is_some() {
        // D26: the download transport is never provider-injected.
        send_validated_redirects(request).await
    } else {
        // Resolved per request, so a late change of the process default is
        // honored by providers created before it.
        let fetch = request.fetch.clone().unwrap_or_else(default_fetch);
        send_one_request(fetch.as_ref(), request, RedirectPolicy::Follow).await
    };
    let response = match sent {
        Ok(response) => response,
        Err(error) => {
            exchange.fail(&error.to_string());
            return Err(error);
        }
    };
    exchange.disarm();
    let latency_ms = started.elapsed().as_millis() as u64;
    tracing::debug!(
        target: "aimux_provider_utils::http",
        status = response.status.as_u16(),
        latency_ms,
        "response"
    );
    Ok(observe_response(
        response,
        request,
        started,
        latency_ms,
        attempt,
        exchange_index,
    ))
}

async fn send_one_request(
    fetch: &dyn Fetch,
    request: &PreparedRequest,
    redirect: RedirectPolicy,
) -> Result<FetchResponse, AiMuxError> {
    let mut fetch_request = build_fetch_request(request, redirect)?;
    fetch_request.signal = request.abort_signal.clone();
    let sent = match &request.abort_signal {
        Some(signal) => {
            if signal.is_aborted() {
                return Err(AiMuxError::from_abort_signal(signal));
            }
            tokio::select! {
                biased;
                () = signal.cancelled() => return Err(AiMuxError::from_abort_signal(signal)),
                sent = fetch.fetch(fetch_request) => sent,
            }
        }
        None => fetch.fetch(fetch_request).await,
    };
    sent.map_err(|error| {
        fetch_error_to_ai_mux_error(
            error,
            &sanitized_request_url(&request.url),
            &crate::logging::redact_request_values(&request.body),
        )
    })
}

fn is_redirect_status(status: http::StatusCode) -> bool {
    matches!(
        status,
        http::StatusCode::MOVED_PERMANENTLY
            | http::StatusCode::FOUND
            | http::StatusCode::SEE_OTHER
            | http::StatusCode::TEMPORARY_REDIRECT
            | http::StatusCode::PERMANENT_REDIRECT
    )
}

fn redirect_error(
    request: &PreparedRequest,
    message: impl Into<String>,
    status: http::StatusCode,
) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(status.as_u16()),
        ..api_call_error(request, message)
    }))
}

/// Send one validated-download exchange, following redirects manually so
/// every hop is re-validated and its connection pinned to the DNS answers
/// that passed the guard (see `download_guard`). The whole chain is one
/// exchange to the caller, matching how an auto-following client behaves.
async fn send_validated_redirects(request: &PreparedRequest) -> Result<FetchResponse, AiMuxError> {
    let validation = request
        .validation
        .clone()
        .expect("send_validated_redirects requires a validation config");
    let trusted_origin = validation.trusted_origin.as_deref();
    let credentialed_origin = validation.credentialed_origin.as_deref();
    let mut current = request.clone();
    // First-request credential gating already ran in `prepare`; this strips
    // hop-by-hop, forwarding, and metadata-service headers for every hop.
    crate::download_guard::sanitize_download_headers(&mut current.headers);
    // Headers never travel past this origin on a redirect; stripping is
    // one-way, so a hop back onto it cannot restore them.
    let credential_anchor = credentialed_origin.unwrap_or(&request.url);
    let mut pinned =
        crate::download_guard::validate_download_target(&current.url, trusted_origin).await?;
    for redirect_count in 0..=MAX_REDIRECTS {
        // Empty pins mean the hop is on the trusted origin.
        let hop = PinnedFetch::new(pinned.clone());
        let response = send_one_request(&hop, &current, RedirectPolicy::Manual).await?;
        let status = response.status;
        if !is_redirect_status(status) {
            return Ok(response);
        }
        let Some(location) = response.headers.get(http::header::LOCATION) else {
            return Ok(response);
        };
        if redirect_count == MAX_REDIRECTS {
            return Err(redirect_error(request, "too many redirects", status));
        }
        let location = location
            .to_str()
            .map(str::to_owned)
            .map_err(|_| redirect_error(request, "redirect location is not valid UTF-8", status))?;
        // Dropping the unconsumed 3xx body releases its connection before a
        // potentially slow DNS check for the next hop.
        drop(response);
        let base = url::Url::parse(&current.url)
            .map_err(|e| AiMuxError::InvalidArgument(format!("invalid request URL: {e}")))?;
        let next = base
            .join(&location)
            .map_err(|e| redirect_error(request, format!("invalid redirect URL: {e}"), status))?;
        // Fetch treats a redirect to a non-HTTP(S) scheme (data:, file:, ...)
        // as a network error; following one would let the redirecting server
        // fabricate a response outside the transport.
        if !matches!(next.scheme(), "http" | "https") {
            return Err(redirect_error(
                request,
                format!("redirect to non-HTTP scheme: {}", next.scheme()),
                status,
            ));
        }
        let next = next.to_string();
        let hop_trusted = crate::download_guard::hop_trusted_origin(trusted_origin, &current.url);
        pinned = crate::download_guard::validate_download_target(&next, hop_trusted).await?;
        // Credentials are scoped to the credential anchor, not the previous
        // hop: once a redirect leaves it, headers cannot come back.
        if !crate::download_guard::same_origin(&next, credential_anchor) {
            crate::download_guard::retain_user_agent(&mut current.headers);
        }
        if status == http::StatusCode::SEE_OTHER
            || (matches!(
                status,
                http::StatusCode::MOVED_PERMANENTLY | http::StatusCode::FOUND
            ) && matches!(current.method, HttpMethod::Post))
        {
            current.method = HttpMethod::Get;
            current.body = HttpBody::Empty;
        }
        current.url = next;
    }
    unreachable!("redirect loop returns on response or error")
}

/// Materialise one prepared request for a [`Fetch`]: validated URL, final body
/// bytes and an ordered-by-name header map. Headers are *inserted*, so a
/// later entry for the same (case-insensitive) name replaces an earlier one —
/// a request never goes out with duplicate `Authorization` headers.
fn build_fetch_request(
    request: &PreparedRequest,
    redirect: RedirectPolicy,
) -> Result<FetchRequest, AiMuxError> {
    let url = url::Url::parse(&request.url)
        .map_err(|error| AiMuxError::InvalidArgument(format!("invalid request URL: {error}")))?;
    let mut headers = http::HeaderMap::new();
    for (name, value) in &request.headers {
        let header_name = http::HeaderName::try_from(name)
            .map_err(|_| AiMuxError::InvalidArgument(format!("invalid header name: {name}")))?;
        let header_value = http::HeaderValue::try_from(value).map_err(|_| {
            AiMuxError::InvalidArgument(format!("invalid header value for {name}: {value}"))
        })?;
        headers.insert(header_name, header_value);
    }
    let body = match &request.body {
        HttpBody::Json(value) => {
            if !headers.contains_key(http::header::CONTENT_TYPE) {
                headers.insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                );
            }
            Bytes::from(serde_json::to_vec(value).map_err(|error| {
                AiMuxError::InvalidArgument(format!("request body is not serializable: {error}"))
            })?)
        }
        HttpBody::Bytes(bytes, content_type) => {
            let content_type = http::HeaderValue::try_from(content_type).map_err(|_| {
                AiMuxError::InvalidArgument(format!("invalid content type: {content_type}"))
            })?;
            headers.insert(http::header::CONTENT_TYPE, content_type);
            Bytes::copy_from_slice(bytes)
        }
        HttpBody::Empty => Bytes::new(),
    };
    Ok(FetchRequest {
        method: match request.method {
            HttpMethod::Get => http::Method::GET,
            HttpMethod::Post => http::Method::POST,
        },
        url,
        headers,
        body,
        redirect,
        signal: None,
        timeout: None,
    })
}

/// Abort-aware provider polling delay. This is not the operation retry delay;
/// Core owns exponential backoff.
///
/// # Errors
///
/// Returns [`AiMuxError::Aborted`] if the caller cancels during the delay.
pub async fn sleep_or_abort(
    duration: Duration,
    abort_signal: Option<&aimux_core::AbortSignal>,
) -> Result<(), AiMuxError> {
    match abort_signal {
        Some(signal) => tokio::select! {
            biased;
            () = signal.cancelled() => Err(AiMuxError::from_abort_signal(signal)),
            () = tokio::time::sleep(duration) => Ok(()),
        },
        None => {
            tokio::time::sleep(duration).await;
            Ok(())
        }
    }
}

fn observe_response(
    response: FetchResponse,
    request: &PreparedRequest,
    started: Instant,
    latency_ms: u64,
    attempt: u32,
    exchange_index: u32,
) -> FetchResponse {
    let status = response.status;
    let headers = response.headers.clone();
    let recording = request.recording_context.as_ref().map(|context| {
        let redacted_headers = redacted_response_headers(&headers);
        let response = ResponseRecord {
            status: status.as_u16(),
            headers: redacted_headers.clone(),
            body: None,
            stream_chunks: Some(0),
            ttfb_ms: Some(latency_ms),
        };
        record_exchange(
            request,
            HttpExchange {
                step: request
                    .recording_context
                    .as_ref()
                    .and_then(RecordingContext::step),
                attempt,
                exchange_index,
                request: to_http_record(request),
                response: Some(response),
                timing: TimingRecord {
                    latency_ms,
                    ttfb_ms: Some(latency_ms),
                },
                error: None,
                finalized: false,
            },
        );
        ResponseRecording {
            context: context.clone(),
            attempt,
            exchange_index,
            status: status.as_u16(),
            headers: redacted_headers,
            ttfb_ms: Some(latency_ms),
            body: ByteAccumulator::new(RECORD_BODY_CAP),
            chunks: 0,
            finalized: false,
        }
    });
    let observed = ObservedBody {
        inner: response.body,
        started,
        chunks: 0,
        done: false,
        recording,
    };
    FetchResponse {
        status,
        headers,
        url: response.url,
        body: observed.boxed(),
    }
}

struct ObservedBody {
    inner: BoxStream<'static, Result<Bytes, FetchError>>,
    started: Instant,
    chunks: usize,
    done: bool,
    recording: Option<ResponseRecording>,
}

impl Stream for ObservedBody {
    type Item = Result<Bytes, FetchError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        let result = self.inner.as_mut().poll_next(context);
        match &result {
            Poll::Ready(Some(Ok(bytes))) => {
                self.chunks += 1;
                let chunks = self.chunks;
                if let Some(recording) = &mut self.recording {
                    recording.body.push(bytes);
                    recording.chunks = chunks;
                }
            }
            Poll::Ready(Some(Err(error))) => {
                self.done = true;
                if let Some(recording) = &mut self.recording {
                    recording.finalize(Some(error.to_string()));
                }
            }
            Poll::Ready(None) => {
                self.done = true;
                tracing::debug!(
                    target: "aimux_provider_utils::http",
                    chunks = self.chunks,
                    duration_ms = self.started.elapsed().as_millis() as u64,
                    "response_body_end"
                );
                if let Some(recording) = &mut self.recording {
                    recording.finalize(None);
                }
            }
            Poll::Pending => {}
        }
        result
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        if !self.done
            && let Some(recording) = &mut self.recording
        {
            recording.finalize(Some("response body abandoned".into()));
        }
    }
}

struct ResponseRecording {
    context: RecordingContext,
    attempt: u32,
    exchange_index: u32,
    status: u16,
    headers: Vec<(String, String)>,
    ttfb_ms: Option<u64>,
    body: ByteAccumulator,
    chunks: usize,
    finalized: bool,
}

impl ResponseRecording {
    fn finalize(&mut self, error: Option<String>) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        self.context.recorder.record_exchange_update(
            &self.context.call_id,
            self.attempt,
            self.exchange_index,
            &ResponseRecord {
                status: self.status,
                headers: self.headers.clone(),
                body: self.body.decode(),
                stream_chunks: Some(self.chunks),
                ttfb_ms: self.ttfb_ms,
            },
            error,
        );
        self.context
            .recorder
            .record_transport_closed(&self.context.call_id);
    }
}

const RECORD_BODY_CAP: usize = 1 << 20;

struct ByteAccumulator {
    bytes: Vec<u8>,
    truncated: bool,
}

impl ByteAccumulator {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity.min(8 * 1024)),
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let remaining = RECORD_BODY_CAP.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            self.bytes.extend_from_slice(&bytes[..remaining]);
            self.truncated = true;
        } else {
            self.bytes.extend_from_slice(bytes);
        }
    }

    fn decode(&self) -> Option<String> {
        if self.bytes.is_empty() {
            return None;
        }
        let mut value = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            while value.ends_with('\u{FFFD}') {
                value.pop();
            }
            value.push_str("…(truncated)");
        }
        Some(value)
    }
}

fn to_http_record(request: &PreparedRequest) -> HttpRecord {
    use aimux_core::recording::is_sensitive_key;
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                if is_sensitive_key(name) {
                    "[REDACTED]".into()
                } else {
                    value.clone()
                },
            )
        })
        .collect();
    let body = match &request.body {
        HttpBody::Json(value) => {
            serde_json::to_string(&crate::logging::redact_error_context(value.clone())).ok()
        }
        HttpBody::Bytes(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        HttpBody::Empty => None,
    }
    .map(|body| truncate_utf8(&body, RECORD_BODY_CAP).to_owned());
    HttpRecord {
        method: request.method.as_str().into(),
        url: sanitized_request_url(&request.url),
        headers,
        body,
    }
}

fn record_exchange(request: &PreparedRequest, exchange: HttpExchange) {
    if let Some(context) = &request.recording_context {
        context
            .recorder
            .record_exchange(&context.call_id, &exchange);
    }
}

fn record_transport_closed(request: &PreparedRequest) {
    if let Some(context) = &request.recording_context {
        context.recorder.record_transport_closed(&context.call_id);
    }
}

fn record_failed_exchange(
    request: &PreparedRequest,
    attempt: u32,
    exchange_index: u32,
    latency_ms: u64,
    error: &str,
) {
    record_exchange(
        request,
        HttpExchange {
            step: request
                .recording_context
                .as_ref()
                .and_then(RecordingContext::step),
            attempt,
            exchange_index,
            request: to_http_record(request),
            response: None,
            timing: TimingRecord {
                latency_ms,
                ttfb_ms: None,
            },
            error: Some(error.to_owned()),
            finalized: true,
        },
    );
}

fn redacted_response_headers(headers: &http::HeaderMap) -> Vec<(String, String)> {
    crate::extract_response_headers::extract_response_header_pairs(headers)
}

fn truncate_utf8(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn request_body_size(request: &PreparedRequest) -> u64 {
    match &request.body {
        HttpBody::Json(value) => serde_json::to_vec(value)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or_default(),
        HttpBody::Bytes(bytes, _) => bytes.len() as u64,
        HttpBody::Empty => 0,
    }
}

fn request_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

pub(crate) fn sanitized_request_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.to_string()
        }
        Err(_) => url.split('?').next().unwrap_or(url).to_owned(),
    }
}
