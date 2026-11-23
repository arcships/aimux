//! Public L0 native HTTP passthrough (`NativeEndpoint`, `native_request`).
//!
//! Sends raw HTTP exchanges directly against a configured provider endpoint
//! while reusing the provider's authentication headers, transport decorator
//! (such as [`crate::SigV4Fetch`]), proxy settings, caller cancellation
//! ([`aimux_core::AbortSignal`]), deadlines, and exchange observer hooks.
//!
//! # Golden URL prefix & query examples (RFC-0040 §5.1)
//!
//! - Base `"https://api.openai.com/v1"` + `"responses?x=1&x=2"` ->
//!   `"https://api.openai.com/v1/responses?x=1&x=2"`
//! - Base `"https://api.openai.com/v1/"` + `"responses?x=1&x=2"` ->
//!   `"https://api.openai.com/v1/responses?x=1&x=2"`
//! - Base `"https://api.openai.com/v1"` + `"models?a=1&a=2&enc=%2F"` ->
//!   `"https://api.openai.com/v1/models?a=1&a=2&enc=%2F"`
//! - Base `"https://api.openai.com"` + `"v1/chat/completions"` ->
//!   `"https://api.openai.com/v1/chat/completions"`
//! - Base `"https://api.openai.com/v1"` + `"?dry_run=true"` ->
//!   `"https://api.openai.com/v1?dry_run=true"`

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use aimux_core::AbortSignal;
use bytes::Bytes;
use futures::Stream;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use tokio::time::Instant;
use url::Url;

use crate::fetch::{
    FetchError, FetchFunction, FetchRequest, FetchResponse, RequestCredentials, default_fetch,
};
use crate::headers::normalize_headers;
use crate::provider_headers::{AuthScheme, Credential, ProviderHeaders};

/// Default per-chunk backpressure buffer ceiling (`max_buffer_bytes`: 256 KiB).
pub const DEFAULT_MAX_BUFFER_BYTES: usize = 256 * 1024;

/// Maximum configurable `max_buffer_bytes` ceiling (64 MiB).
pub const MAX_ALLOWED_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum response body bytes forwarded to an exchange observer (6 MiB).
pub const DEFAULT_MAX_RECORDED_BYTES: usize = 6 * 1024 * 1024;

/// Headers owned and overwritten by [`crate::SigV4Fetch`] on AWS endpoints.
///
/// Endpoints created with [`NativeEndpoint::with_sigv4`], or bound via
/// [`NativeEndpoint::with_fetch`] to [`crate::SigV4Fetch`] / [`crate::SigV4AuthFetch`],
/// automatically protect these headers (`authorization`, `host`, `x-amz-date`,
/// `x-amz-security-token`, `x-amz-content-sha256`), the `x-amz-*` prefix, and any
/// transport-declared auth/dispatch header (such as `SigV4AuthFetch`'s `api_key_header`,
/// e.g. `x-api-key`). Custom `Fetch` wrappers around a SigV4 signer must use
/// [`NativeEndpoint::with_sigv4`] or explicitly protect `SIGV4_PROTECTED_HEADERS`,
/// `"x-amz-"`, and any custom dispatch headers via
/// [`NativeEndpoint::try_with_protected_headers`] /
/// [`NativeEndpoint::try_with_protected_header_prefixes`].
pub const SIGV4_PROTECTED_HEADERS: &[&str] = &[
    "authorization",
    "host",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
];

const SIGV4_PROTECTED_PREFIX: &str = "x-amz-";

static NEXT_EXCHANGE_ID: AtomicU64 = AtomicU64::new(1);

type AsyncHeaderResolver =
    Arc<dyn Fn() -> BoxFuture<'static, Result<HeaderMap, aimux_core::AiMuxError>> + Send + Sync>;

#[derive(Clone)]
enum EndpointAuth {
    None,
    ProviderHeaders(ProviderHeaders),
    CustomResolver(AsyncHeaderResolver),
}

/// Errors returned by L0 native passthrough setup, validation, or execution.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    /// Invalid endpoint or context configuration (`base_url`, buffer cap, timeout, header policy).
    #[error("invalid native endpoint configuration: {message}")]
    InvalidConfig { message: String },
    /// Rejected relative path or query (scheme/authority/traversal/encoding/prefix mismatch).
    #[error("invalid relative path: {message}")]
    InvalidPath { message: String },
    /// Caller attempted to supply or override an endpoint-owned credential header, or an invalid header.
    #[error("invalid native header `{name}`: {message}")]
    InvalidHeader { name: String, message: String },
    /// Credential or header resolution failed.
    #[error("native authentication error: {0}")]
    Auth(#[source] aimux_core::AiMuxError),
    /// Transport failure establishing the request or reading the response stream.
    #[error("native transport error: {0}")]
    Transport(#[source] FetchError),
    /// Caller [`AbortSignal`] cancelled the request.
    #[error("{0}")]
    Aborted(String),
    /// Operation [`NativeContext::timeout`] budget expired.
    #[error("{0}")]
    Timeout(String),
}

/// A configured endpoint for L0 native passthrough, pinned to one base URL,
/// auth scope, protected header policy, and transport.
#[derive(Clone)]
pub struct NativeEndpoint {
    base_url: Url,
    origin_prefix_str: String,
    base_path_prefix: String,
    auth: EndpointAuth,
    protected_headers: BTreeSet<String>,
    protected_header_prefixes: BTreeSet<String>,
    fetch: Option<FetchFunction>,
    idempotency_header: Option<HeaderName>,
    max_buffer_bytes: usize,
}

impl NativeEndpoint {
    /// Construct an unauthenticated endpoint pinned to `base_url`.
    ///
    /// Owns no credential headers and reads no environment credentials; callers
    /// may send normal headers unless restricted via [`Self::try_with_protected_headers`]
    /// or [`Self::try_with_protected_header_prefixes`]. For AWS SigV4 endpoints, prefer
    /// [`Self::with_sigv4`] (or bind [`crate::SigV4Fetch`] / [`crate::SigV4AuthFetch`]
    /// via [`Self::with_fetch`], which auto-protects `authorization` and `x-amz-*`).
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] when `base_url` is blank, unparseable,
    /// not `http`/`https`, lacks a host, contains userinfo, or carries a query or fragment.
    pub fn unauthenticated(base_url: &str) -> Result<Self, NativeError> {
        Self::new_inner(base_url, EndpointAuth::None, BTreeSet::new())
    }

    /// Construct a SigV4-authenticated endpoint pinned to `base_url` using `sigv4_fetch`.
    ///
    /// Automatically protects [`SIGV4_PROTECTED_HEADERS`] (`authorization`, `host`,
    /// `x-amz-date`, `x-amz-security-token`, `x-amz-content-sha256`), all `x-amz-*`
    /// headers, and any transport-declared headers from [`crate::Fetch::protected_header_names`]
    /// (such as [`crate::SigV4AuthFetch`]'s `api_key_header`) so callers cannot spoof
    /// AWS signing headers or bypass SigV4 signing.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] when `base_url` is blank, unparseable,
    /// not `http`/`https`, lacks a host, contains userinfo, or carries a query or fragment.
    pub fn with_sigv4(base_url: &str, sigv4_fetch: FetchFunction) -> Result<Self, NativeError> {
        let mut endpoint = Self::unauthenticated(base_url)?;
        endpoint.enable_sigv4_protection();
        endpoint.register_fetch_protected_headers(sigv4_fetch.as_ref());
        endpoint.fetch = Some(sigv4_fetch);
        Ok(endpoint)
    }

    /// Construct an endpoint authenticated by `headers`.
    ///
    /// When `headers.credential` is not [`Credential::None`], the credential
    /// carrier header (`authorization` or the configured custom header name) is
    /// registered as protected and cannot be set or overridden per request.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] when `base_url` is invalid or a
    /// protected header name is invalid.
    pub fn with_provider_headers(
        base_url: &str,
        headers: ProviderHeaders,
    ) -> Result<Self, NativeError> {
        let mut protected = BTreeSet::new();
        if !matches!(headers.credential, Credential::None) {
            let auth_header = match headers.scheme {
                AuthScheme::Bearer | AuthScheme::Scheme(_) => "authorization",
                AuthScheme::Header(name) => name,
            };
            validate_header_name(auth_header).map_err(|err| NativeError::InvalidConfig {
                message: format!("invalid auth scheme header `{auth_header}`: {err}"),
            })?;
            protected.insert(auth_header.to_ascii_lowercase());
        }
        Self::new_inner(base_url, EndpointAuth::ProviderHeaders(headers), protected)
    }

    /// Construct an endpoint using a custom async header resolver (for dynamic tokens).
    ///
    /// All names in `protected_headers` are marked endpoint-owned upfront (rejected before
    /// invoking `resolver`), and every header name returned by `resolver` is also
    /// automatically protected per request so caller-supplied headers cannot override
    /// dynamic auth headers and their values remain redacted in observer events.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] when `base_url` is invalid.
    pub fn with_header_resolver<F>(
        base_url: &str,
        protected_headers: impl IntoIterator<Item = HeaderName>,
        resolver: F,
    ) -> Result<Self, NativeError>
    where
        F: Fn() -> BoxFuture<'static, Result<HeaderMap, aimux_core::AiMuxError>>
            + Send
            + Sync
            + 'static,
    {
        let protected = protected_headers
            .into_iter()
            .map(|name| name.as_str().to_ascii_lowercase())
            .collect();
        Self::new_inner(
            base_url,
            EndpointAuth::CustomResolver(Arc::new(resolver)),
            protected,
        )
    }

    fn new_inner(
        base_url: &str,
        auth: EndpointAuth,
        protected_headers: BTreeSet<String>,
    ) -> Result<Self, NativeError> {
        let (parsed, origin_prefix_str, base_path_prefix) = parse_and_normalize_base_url(base_url)?;
        Ok(Self {
            base_url: parsed,
            origin_prefix_str,
            base_path_prefix,
            auth,
            protected_headers,
            protected_header_prefixes: BTreeSet::new(),
            fetch: None,
            idempotency_header: None,
            max_buffer_bytes: DEFAULT_MAX_BUFFER_BYTES,
        })
    }

    fn enable_sigv4_protection(&mut self) {
        for name in SIGV4_PROTECTED_HEADERS {
            self.protected_headers.insert((*name).to_string());
        }
        self.protected_header_prefixes
            .insert(SIGV4_PROTECTED_PREFIX.to_string());
    }

    fn register_fetch_protected_headers(&mut self, fetch: &dyn crate::fetch::Fetch) {
        for name in fetch.protected_header_names() {
            self.protected_headers
                .insert(name.as_str().to_ascii_lowercase());
        }
    }

    /// Bind an explicit [`FetchFunction`] (such as a proxy transport or [`crate::SigV4Fetch`]).
    ///
    /// If `fetch` reports SigV4 signing ([`crate::SigV4Fetch`] / [`crate::SigV4AuthFetch`]),
    /// [`SIGV4_PROTECTED_HEADERS`] (`authorization`, `host`, `x-amz-date`,
    /// `x-amz-security-token`, `x-amz-content-sha256`), the `x-amz-*` prefix, and any
    /// transport-declared headers from [`crate::Fetch::protected_header_names`] are
    /// protected automatically. If wrapping a SigV4 signer inside a custom `Fetch`, use
    /// [`Self::with_sigv4`] or declare protected headers/prefixes explicitly.
    #[must_use]
    pub fn with_fetch(mut self, fetch: FetchFunction) -> Self {
        if fetch.protects_sigv4_headers() {
            self.enable_sigv4_protection();
        }
        self.register_fetch_protected_headers(fetch.as_ref());
        self.fetch = Some(fetch);
        self
    }

    /// Declare additional endpoint-owned header names that callers may not set or override.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] if any supplied header name is invalid.
    pub fn try_with_protected_headers<'a>(
        mut self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, NativeError> {
        for raw in names {
            let valid = validate_header_name(raw).map_err(|err| NativeError::InvalidConfig {
                message: format!("invalid protected header name `{raw}`: {err}"),
            })?;
            self.protected_headers
                .insert(valid.as_str().to_ascii_lowercase());
        }
        Ok(self)
    }

    /// Declare endpoint-owned header name prefixes (such as `"x-amz-"`) that callers
    /// may not set or override.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] if any supplied prefix is empty or contains
    /// invalid HTTP header name characters.
    pub fn try_with_protected_header_prefixes<'a>(
        mut self,
        prefixes: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, NativeError> {
        for raw in prefixes {
            let valid = validate_header_name(raw).map_err(|err| NativeError::InvalidConfig {
                message: format!("invalid protected header prefix `{raw}`: {err}"),
            })?;
            self.protected_header_prefixes
                .insert(valid.as_str().to_ascii_lowercase());
        }
        Ok(self)
    }

    /// Declare the idempotency header name supported by this endpoint for safe-retry opt-in.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] if `name` is not a valid HTTP header name.
    pub fn try_with_idempotency_header(mut self, name: &str) -> Result<Self, NativeError> {
        let header_name = validate_header_name(name).map_err(|err| NativeError::InvalidConfig {
            message: format!("invalid idempotency header name `{name}`: {err}"),
        })?;
        self.idempotency_header = Some(header_name);
        Ok(self)
    }

    /// Configure the maximum yielded chunk size / per-poll buffer bound in bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] if `bytes == 0` or `bytes > MAX_ALLOWED_BUFFER_BYTES`.
    pub fn try_with_max_buffer_bytes(mut self, bytes: usize) -> Result<Self, NativeError> {
        if bytes == 0 || bytes > MAX_ALLOWED_BUFFER_BYTES {
            return Err(NativeError::InvalidConfig {
                message: format!(
                    "max_buffer_bytes must be between 1 and {MAX_ALLOWED_BUFFER_BYTES}, got {bytes}"
                ),
            });
        }
        self.max_buffer_bytes = bytes;
        Ok(self)
    }

    /// Return the sanitized base URL (with any embedded URL userinfo removed).
    #[must_use]
    pub fn sanitized_base_url(&self) -> String {
        sanitize_url(&self.base_url)
    }

    /// Return the set of lowercased protected header names owned by this endpoint.
    #[must_use]
    pub fn protected_headers(&self) -> &BTreeSet<String> {
        &self.protected_headers
    }

    /// Return the set of lowercased protected header name prefixes owned by this endpoint.
    #[must_use]
    pub fn protected_header_prefixes(&self) -> &BTreeSet<String> {
        &self.protected_header_prefixes
    }

    /// Return the configured idempotency header name, if any.
    #[must_use]
    pub fn idempotency_header(&self) -> Option<&HeaderName> {
        self.idempotency_header.as_ref()
    }

    /// Return the configured maximum chunk/buffer size in bytes.
    #[must_use]
    pub fn max_buffer_bytes(&self) -> usize {
        self.max_buffer_bytes
    }

    /// Resolve a validated relative path and query against this endpoint's base URL.
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidPath`] if `relative_path_and_query` tries
    /// to escape the endpoint's origin or base path prefix or contains illegal
    /// segments, control characters, fragments, backslashes, or invalid `%XX` escapes.
    pub fn resolve_url(&self, relative_path_and_query: &str) -> Result<Url, NativeError> {
        resolve_relative_url(
            &self.base_url,
            &self.origin_prefix_str,
            &self.base_path_prefix,
            relative_path_and_query,
        )
    }
}

impl fmt::Debug for NativeEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeEndpoint")
            .field("base_url", &self.sanitized_base_url())
            .field("protected_headers", &self.protected_headers)
            .field("protected_header_prefixes", &self.protected_header_prefixes)
            .field("has_fetch", &self.fetch.is_some())
            .field(
                "idempotency_header",
                &self.idempotency_header.as_ref().map(HeaderName::as_str),
            )
            .field("max_buffer_bytes", &self.max_buffer_bytes)
            .finish()
    }
}

/// Retry options when opting in to pre-head safe retries via [`NativeRetryPolicy::Safe`].
#[derive(Clone)]
pub struct SafeRetryOptions {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub retryable_statuses: Vec<StatusCode>,
    pub idempotency_key: Option<(HeaderName, HeaderValue)>,
}

impl fmt::Debug for SafeRetryOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafeRetryOptions")
            .field("max_retries", &self.max_retries)
            .field("initial_backoff", &self.initial_backoff)
            .field("retryable_statuses", &self.retryable_statuses)
            .field(
                "idempotency_key",
                &self
                    .idempotency_key
                    .as_ref()
                    .map(|(name, _)| (name.as_str(), "[REDACTED]")),
            )
            .finish()
    }
}

impl Default for SafeRetryOptions {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_millis(100),
            retryable_statuses: vec![
                StatusCode::REQUEST_TIMEOUT,
                StatusCode::TOO_MANY_REQUESTS,
                StatusCode::BAD_GATEWAY,
                StatusCode::SERVICE_UNAVAILABLE,
                StatusCode::GATEWAY_TIMEOUT,
            ],
            idempotency_key: None,
        }
    }
}

impl SafeRetryOptions {
    /// Set the maximum retry count (attempts after the initial attempt).
    #[must_use]
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the initial retry backoff duration (doubles each attempt).
    #[must_use]
    pub fn with_initial_backoff(mut self, initial_backoff: Duration) -> Self {
        self.initial_backoff = initial_backoff;
        self
    }

    /// Set the HTTP status codes eligible for retry before response head commitment.
    #[must_use]
    pub fn with_retryable_statuses(mut self, statuses: Vec<StatusCode>) -> Self {
        self.retryable_statuses = statuses;
        self
    }

    /// Attach an idempotency `(header_name, header_value)` pair for unsafe-method retry opt-in.
    #[must_use]
    pub fn with_idempotency_key(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.idempotency_key = Some((name, value));
        self
    }
}

/// Retry policy for [`native_request`] (RFC-0040 §5.2). Defaults to [`NativeRetryPolicy::Never`].
#[derive(Debug, Clone, Default)]
pub enum NativeRetryPolicy {
    /// Execute a single `Fetch` exchange (`retry=never`).
    #[default]
    Never,
    /// Allow pre-head retries on safe methods (`GET`, `HEAD`, `OPTIONS`), or
    /// on unsafe methods only when both the endpoint and options supply a matching
    /// non-empty idempotency header.
    Safe(SafeRetryOptions),
}

/// A raw HTTP request targeting a [`NativeEndpoint`].
#[derive(Clone)]
pub struct NativeRequest {
    pub method: Method,
    pub relative_path_and_query: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub retry: NativeRetryPolicy,
}

impl NativeRequest {
    /// Create a request with empty headers, empty body, and [`NativeRetryPolicy::Never`].
    #[must_use]
    pub fn new(method: Method, relative_path_and_query: impl Into<String>) -> Self {
        Self {
            method,
            relative_path_and_query: relative_path_and_query.into(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            retry: NativeRetryPolicy::Never,
        }
    }

    /// Replace request headers (preserving duplicate header values in [`HeaderMap`]).
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Set the raw request body bytes.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Configure the retry policy for this request.
    #[must_use]
    pub fn with_retry(mut self, retry: NativeRetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

impl fmt::Debug for NativeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        f.debug_struct("NativeRequest")
            .field("method", &self.method)
            .field(
                "relative_path_and_query",
                &sanitize_relative_path_and_query(&self.relative_path_and_query),
            )
            .field("header_names", &header_names)
            .field("body_bytes", &self.body.len())
            .field("retry", &self.retry)
            .finish()
    }
}

/// Metadata emitted when one passthrough HTTP attempt begins.
#[derive(Clone)]
pub struct NativeExchangeStarted {
    pub exchange_id: u64,
    pub attempt: u32,
    pub method: Method,
    pub sanitized_url: String,
    pub redacted_headers: Vec<(String, String)>,
    pub request_body_bytes: usize,
}

impl fmt::Debug for NativeExchangeStarted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeExchangeStarted")
            .field("exchange_id", &self.exchange_id)
            .field("attempt", &self.attempt)
            .field("method", &self.method)
            .field("sanitized_url", &self.sanitized_url)
            .field("redacted_headers", &self.redacted_headers)
            .field("request_body_bytes", &self.request_body_bytes)
            .finish()
    }
}

/// Terminal outcome of one observed passthrough exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeExchangeOutcome {
    Completed,
    Aborted,
    Failed(String),
}

/// Terminal metadata emitted once per started passthrough exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeExchangeFinished {
    pub exchange_id: u64,
    pub attempt: u32,
    pub outcome: NativeExchangeOutcome,
    pub recording_complete: bool,
    pub incomplete_reason: Option<String>,
}

/// Observer hook for recording / telemetry at the L0 exchange boundary.
pub trait NativeExchangeObserver: Send + Sync + 'static {
    /// Called immediately before sending an attempt. Returning `Err` marks
    /// the observation incomplete without aborting the live request.
    ///
    /// # Errors
    ///
    /// Returns a human-readable diagnostic explaining why starting observation failed.
    fn on_exchange_started(&self, event: &NativeExchangeStarted) -> Result<(), String>;

    /// Called once response head arrives. Returning `Err` stops further response
    /// chunk observation and marks the observation incomplete without failing the response.
    ///
    /// # Errors
    ///
    /// Returns a human-readable diagnostic explaining why recording response headers failed.
    fn on_response_head(
        &self,
        exchange_id: u64,
        status: StatusCode,
        headers: &HeaderMap,
        sanitized_effective_url: &str,
    ) -> Result<(), String>;

    /// Called as response chunks are pulled by the caller, up to `max_recorded_bytes`.
    /// Returning `Err` stops further chunk callbacks and marks the observation incomplete.
    ///
    /// # Errors
    ///
    /// Returns a human-readable diagnostic explaining why recording the chunk failed.
    fn on_response_chunk(&self, exchange_id: u64, chunk: &Bytes) -> Result<(), String>;

    /// Guaranteed to fire exactly once per started attempt (`Completed`, `Aborted`, or `Failed`).
    fn on_exchange_finished(&self, event: NativeExchangeFinished);
}

/// Per-operation execution context (cancellation, timeout, and optional L0 observer).
#[derive(Clone)]
pub struct NativeContext {
    pub abort_signal: Option<AbortSignal>,
    pub timeout: Option<Duration>,
    pub observer: Option<Arc<dyn NativeExchangeObserver>>,
    pub max_recorded_bytes: usize,
}

impl Default for NativeContext {
    fn default() -> Self {
        Self {
            abort_signal: None,
            timeout: None,
            observer: None,
            max_recorded_bytes: DEFAULT_MAX_RECORDED_BYTES,
        }
    }
}

impl NativeContext {
    /// Create a default context with no signal, no deadline, no observer, and [`DEFAULT_MAX_RECORDED_BYTES`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a caller cancellation signal.
    #[must_use]
    pub fn with_abort_signal(mut self, signal: AbortSignal) -> Self {
        self.abort_signal = Some(signal);
        self
    }

    /// Attach a whole-operation deadline spanning auth, exchange, and body reads.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Attach an L0 exchange observer (such as a recording sink).
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn NativeExchangeObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Override `max_recorded_bytes` (must be non-zero).
    ///
    /// # Errors
    ///
    /// Returns [`NativeError::InvalidConfig`] when `max_recorded_bytes == 0`.
    pub fn try_with_max_recorded_bytes(
        mut self,
        max_recorded_bytes: usize,
    ) -> Result<Self, NativeError> {
        if max_recorded_bytes == 0 {
            return Err(NativeError::InvalidConfig {
                message: "max_recorded_bytes must be greater than 0".to_string(),
            });
        }
        self.max_recorded_bytes = max_recorded_bytes;
        Ok(self)
    }
}

impl fmt::Debug for NativeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeContext")
            .field("has_abort_signal", &self.abort_signal.is_some())
            .field("timeout", &self.timeout)
            .field("has_observer", &self.observer.is_some())
            .field("max_recorded_bytes", &self.max_recorded_bytes)
            .finish()
    }
}

/// Raw HTTP response returned from [`native_request`].
pub struct NativeResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub effective_url: Url,
    pub body: NativeBodyStream,
}

impl fmt::Debug for NativeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        f.debug_struct("NativeResponse")
            .field("status", &self.status)
            .field("header_names", &header_names)
            .field("effective_url", &sanitize_url(&self.effective_url))
            .finish_non_exhaustive()
    }
}

struct ExchangeFinisher {
    observer: Option<Arc<dyn NativeExchangeObserver>>,
    exchange_id: u64,
    attempt: u32,
    recording_complete: bool,
    incomplete_reason: Option<String>,
    finished: bool,
}

impl ExchangeFinisher {
    fn new(
        observer: Option<Arc<dyn NativeExchangeObserver>>,
        exchange_id: u64,
        attempt: u32,
    ) -> Self {
        Self {
            observer,
            exchange_id,
            attempt,
            recording_complete: true,
            incomplete_reason: None,
            finished: false,
        }
    }

    fn mark_incomplete(&mut self, reason: String) {
        self.recording_complete = false;
        if self.incomplete_reason.is_none() {
            self.incomplete_reason = Some(reason);
        }
    }

    fn finish(&mut self, outcome: NativeExchangeOutcome, complete_on_eof: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        let recording_complete = complete_on_eof && self.recording_complete;
        let incomplete_reason = self.incomplete_reason.take().or_else(|| {
            (!complete_on_eof).then(|| match &outcome {
                NativeExchangeOutcome::Completed => "exchange ended before completion".to_string(),
                NativeExchangeOutcome::Aborted => "exchange aborted".to_string(),
                NativeExchangeOutcome::Failed(reason) => reason.clone(),
            })
        });
        if let Some(observer) = self.observer.take() {
            observer.on_exchange_finished(NativeExchangeFinished {
                exchange_id: self.exchange_id,
                attempt: self.attempt,
                outcome,
                recording_complete,
                incomplete_reason,
            });
        }
    }
}

impl Drop for ExchangeFinisher {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(NativeExchangeOutcome::Aborted, false);
        }
    }
}

struct StreamObserverState {
    observer: Option<Arc<dyn NativeExchangeObserver>>,
    exchange_id: u64,
    recorded_bytes: usize,
    max_recorded_bytes: usize,
    active: bool,
}

impl StreamObserverState {
    fn observe_chunk(&mut self, chunk: &Bytes, finisher: &mut ExchangeFinisher) {
        if !self.active {
            return;
        }
        let next_total = self.recorded_bytes.saturating_add(chunk.len());
        if next_total > self.max_recorded_bytes {
            self.active = false;
            self.observer = None;
            finisher.mark_incomplete(format!(
                "recorded response body exceeded limit of {} bytes",
                self.max_recorded_bytes
            ));
            return;
        }
        self.recorded_bytes = next_total;
        if let Some(observer) = &self.observer
            && let Err(reason) = observer.on_response_chunk(self.exchange_id, chunk)
        {
            self.active = false;
            self.observer = None;
            finisher.mark_incomplete(reason);
        }
    }
}

/// Consumer-driven response byte stream bounded by [`NativeEndpoint::max_buffer_bytes`],
/// caller [`AbortSignal`], and [`NativeContext::timeout`].
pub struct NativeBodyStream {
    inner: Option<BoxStream<'static, Result<Bytes, FetchError>>>,
    buffered: Bytes,
    max_buffer_bytes: usize,
    abort_signal: Option<AbortSignal>,
    abort_fut: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    deadline: Option<Instant>,
    timeout_label: Option<String>,
    deadline_sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    finisher: ExchangeFinisher,
    observer_state: StreamObserverState,
    terminated: bool,
}

impl NativeBodyStream {
    fn terminate_err(
        &mut self,
        error: NativeError,
        outcome: NativeExchangeOutcome,
    ) -> Poll<Option<Result<Bytes, NativeError>>> {
        self.terminated = true;
        self.buffered = Bytes::new();
        self.inner = None;
        self.abort_fut = None;
        self.deadline_sleep = None;
        self.finisher.finish(outcome, false);
        Poll::Ready(Some(Err(error)))
    }

    fn finish_eof(&mut self) -> Poll<Option<Result<Bytes, NativeError>>> {
        self.terminated = true;
        self.buffered = Bytes::new();
        self.inner = None;
        self.abort_fut = None;
        self.deadline_sleep = None;
        self.finisher.finish(NativeExchangeOutcome::Completed, true);
        Poll::Ready(None)
    }

    fn check_active_bounds(&mut self) -> Option<Poll<Option<Result<Bytes, NativeError>>>> {
        if let Some(signal) = &self.abort_signal
            && signal.is_aborted()
        {
            return Some(self.terminate_err(
                NativeError::Aborted("request aborted".to_string()),
                NativeExchangeOutcome::Aborted,
            ));
        }
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            let msg = self
                .timeout_label
                .clone()
                .unwrap_or_else(|| "request timed out".to_string());
            return Some(self.terminate_err(
                NativeError::Timeout(msg.clone()),
                NativeExchangeOutcome::Failed(msg),
            ));
        }
        None
    }

    fn poll_next_slice_from_buffer(&mut self) -> Option<Bytes> {
        if self.buffered.is_empty() {
            return None;
        }
        let take = self.buffered.len().min(self.max_buffer_bytes);
        Some(self.buffered.split_to(take))
    }
}

impl Stream for NativeBodyStream {
    type Item = Result<Bytes, NativeError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if this.terminated {
            return Poll::Ready(None);
        }

        if let Some(done) = this.check_active_bounds() {
            return done;
        }
        if let Some(abort_fut) = this.abort_fut.as_mut()
            && abort_fut.as_mut().poll(cx).is_ready()
        {
            return this.terminate_err(
                NativeError::Aborted("request aborted".to_string()),
                NativeExchangeOutcome::Aborted,
            );
        }
        if let Some(sleep) = this.deadline_sleep.as_mut()
            && sleep.as_mut().poll(cx).is_ready()
        {
            let msg = this
                .timeout_label
                .clone()
                .unwrap_or_else(|| "request timed out".to_string());
            return this.terminate_err(
                NativeError::Timeout(msg.clone()),
                NativeExchangeOutcome::Failed(msg),
            );
        }

        if let Some(slice) = this.poll_next_slice_from_buffer() {
            this.observer_state
                .observe_chunk(&slice, &mut this.finisher);
            return Poll::Ready(Some(Ok(slice)));
        }

        loop {
            let poll = match this.inner.as_mut() {
                Some(inner) => inner.as_mut().poll_next(cx),
                None => return this.finish_eof(),
            };
            match poll {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    if let Some(done) = this.check_active_bounds() {
                        return done;
                    }
                    return this.finish_eof();
                }
                Poll::Ready(Some(Err(fetch_err))) => {
                    if matches!(fetch_err, FetchError::Aborted) {
                        return this.terminate_err(
                            NativeError::Aborted("request aborted".to_string()),
                            NativeExchangeOutcome::Aborted,
                        );
                    }
                    if let Some(done) = this.check_active_bounds() {
                        return done;
                    }
                    let err = NativeError::Transport(fetch_err);
                    let msg = err.to_string();
                    return this.terminate_err(err, NativeExchangeOutcome::Failed(msg));
                }
                Poll::Ready(Some(Ok(bytes))) => {
                    if let Some(done) = this.check_active_bounds() {
                        return done;
                    }
                    if bytes.is_empty() {
                        continue;
                    }
                    let mut chunk = bytes;
                    let slice = if chunk.len() > this.max_buffer_bytes {
                        let head = chunk.split_to(this.max_buffer_bytes);
                        this.buffered = chunk;
                        head
                    } else {
                        chunk
                    };
                    this.observer_state
                        .observe_chunk(&slice, &mut this.finisher);
                    return Poll::Ready(Some(Ok(slice)));
                }
            }
        }
    }
}

impl Drop for NativeBodyStream {
    fn drop(&mut self) {
        // Release the underlying transport stream before finalizing observation so no
        // background consumption continues after consumer drop (RFC-0040 §5.3).
        self.inner = None;
        self.buffered = Bytes::new();
        self.abort_fut = None;
        self.deadline_sleep = None;
        if !self.terminated {
            self.terminated = true;
            self.finisher
                .mark_incomplete("response body dropped before EOF".to_string());
            self.finisher.finish(NativeExchangeOutcome::Aborted, false);
        }
    }
}

enum AttemptError {
    RetryableStatus,
    RetryableConnect(NativeError),
    Fatal(NativeError),
}

/// Send one raw HTTP request over `endpoint` with abort, deadline, and observer bounds.
///
/// # Errors
///
/// Returns:
/// - [`NativeError::InvalidConfig`] when `context.max_recorded_bytes == 0` or `context.timeout` overflows.
/// - [`NativeError::Aborted`] when `context.abort_signal` is already aborted or fires mid-operation.
/// - [`NativeError::Timeout`] when `context.timeout` expires before response head arrives.
/// - [`NativeError::InvalidPath`] when `request.relative_path_and_query` violates §5.1 relative path/origin rules.
/// - [`NativeError::InvalidHeader`] when `request.headers` (or `idempotency_key`) collides with `endpoint.protected_headers` or fails validation.
/// - [`NativeError::Auth`] when endpoint credential/header resolution fails.
/// - [`NativeError::Transport`] when `Fetch::fetch` fails before producing a response head.
pub async fn native_request(
    endpoint: &NativeEndpoint,
    request: NativeRequest,
    context: NativeContext,
) -> Result<NativeResponse, NativeError> {
    if context.max_recorded_bytes == 0 {
        return Err(NativeError::InvalidConfig {
            message: "max_recorded_bytes must be greater than 0".to_string(),
        });
    }
    let timeout_label = context
        .timeout
        .map(|duration| format!("Operation timeout of {}ms exceeded", duration.as_millis()));
    let deadline = match context.timeout {
        Some(duration) => {
            let at =
                Instant::now()
                    .checked_add(duration)
                    .ok_or_else(|| NativeError::InvalidConfig {
                        message: format!("timeout duration {duration:?} exceeds supported range"),
                    })?;
            Some(at)
        }
        None => None,
    };

    check_pre_bounds(
        context.abort_signal.as_ref(),
        deadline,
        timeout_label.as_deref(),
    )?;

    let target_url = endpoint.resolve_url(&request.relative_path_and_query)?;
    validate_caller_headers(
        &endpoint.protected_headers,
        &endpoint.protected_header_prefixes,
        &request.headers,
        &request.retry,
    )?;

    let resolved_headers = resolve_endpoint_headers(
        endpoint,
        context.abort_signal.as_ref(),
        deadline,
        timeout_label.as_deref(),
    )
    .await?;

    let effective_protected: std::borrow::Cow<'_, BTreeSet<String>> =
        if matches!(endpoint.auth, EndpointAuth::CustomResolver(_)) && !resolved_headers.is_empty()
        {
            let mut merged = endpoint.protected_headers.clone();
            for name in resolved_headers.keys() {
                merged.insert(name.as_str().to_ascii_lowercase());
            }
            validate_caller_headers(
                &merged,
                &endpoint.protected_header_prefixes,
                &request.headers,
                &request.retry,
            )?;
            std::borrow::Cow::Owned(merged)
        } else {
            std::borrow::Cow::Borrowed(&endpoint.protected_headers)
        };

    let wire_headers = assemble_wire_headers(
        endpoint,
        effective_protected.as_ref(),
        resolved_headers,
        &request.headers,
        &request.retry,
    );

    let fetch = endpoint.fetch.clone().unwrap_or_else(default_fetch);
    let sanitized_target = sanitize_url(&target_url);
    let redacted_req_headers = redact_header_pairs(
        &wire_headers,
        effective_protected.as_ref(),
        &endpoint.protected_header_prefixes,
    );
    let (max_retries, initial_backoff, retryable_statuses) =
        effective_retry_plan(endpoint, &request.method, &request.retry);

    let mut attempt: u64 = 1;
    loop {
        check_pre_bounds(
            context.abort_signal.as_ref(),
            deadline,
            timeout_label.as_deref(),
        )?;
        let can_retry = can_retry_attempt(attempt, max_retries);
        let reported_attempt = u32::try_from(attempt).unwrap_or(u32::MAX);
        let attempt_result = execute_attempt(
            fetch.as_ref(),
            &request.method,
            &target_url,
            &sanitized_target,
            &wire_headers,
            &redacted_req_headers,
            effective_protected.as_ref(),
            &endpoint.protected_header_prefixes,
            &request.body,
            endpoint.max_buffer_bytes,
            context.max_recorded_bytes,
            reported_attempt,
            can_retry,
            retryable_statuses,
            context.abort_signal.as_ref(),
            deadline,
            timeout_label.as_deref(),
            context.observer.clone(),
        )
        .await;

        match attempt_result {
            Ok(response) => return Ok(response),
            Err(AttemptError::RetryableStatus | AttemptError::RetryableConnect(_)) if can_retry => {
                wait_retry_backoff(
                    initial_backoff,
                    attempt,
                    context.abort_signal.as_ref(),
                    deadline,
                    timeout_label.as_deref(),
                )
                .await?;
                attempt += 1;
            }
            Err(AttemptError::RetryableConnect(err) | AttemptError::Fatal(err)) => return Err(err),
            Err(AttemptError::RetryableStatus) => {
                return Err(NativeError::InvalidConfig {
                    message: "internal retry state mismatch".to_string(),
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_attempt(
    fetch: &dyn crate::fetch::Fetch,
    method: &Method,
    target_url: &Url,
    sanitized_target: &str,
    wire_headers: &HeaderMap,
    redacted_req_headers: &[(String, String)],
    protected_headers: &BTreeSet<String>,
    protected_prefixes: &BTreeSet<String>,
    body: &Bytes,
    max_buffer_bytes: usize,
    max_recorded_bytes: usize,
    attempt: u32,
    can_retry: bool,
    retryable_statuses: &[StatusCode],
    abort_signal: Option<&AbortSignal>,
    deadline: Option<Instant>,
    timeout_label: Option<&str>,
    observer: Option<Arc<dyn NativeExchangeObserver>>,
) -> Result<NativeResponse, AttemptError> {
    let exchange_id = NEXT_EXCHANGE_ID.fetch_add(1, Ordering::Relaxed);
    let mut finisher = ExchangeFinisher::new(observer.clone(), exchange_id, attempt);
    let mut observer_active = observer.is_some();

    if let Some(obs) = &observer {
        let started = NativeExchangeStarted {
            exchange_id,
            attempt,
            method: method.clone(),
            sanitized_url: sanitized_target.to_string(),
            redacted_headers: redacted_req_headers.to_vec(),
            request_body_bytes: body.len(),
        };
        if let Err(reason) = obs.on_exchange_started(&started) {
            observer_active = false;
            finisher.mark_incomplete(reason);
        }
    }

    let mut fetch_req = FetchRequest::new(method.clone(), target_url.clone());
    fetch_req.headers = wire_headers.clone();
    fetch_req.body = body.clone();
    fetch_req.signal = abort_signal.cloned();
    fetch_req.credentials = RequestCredentials::Include;

    let sent = race_with_bounds(
        async move {
            fetch.fetch(fetch_req).await.map_err(|err| match err {
                FetchError::Aborted => (NativeError::Aborted("request aborted".to_string()), false),
                other => {
                    let retryable = matches!(other, FetchError::Connect(_));
                    (NativeError::Transport(other), retryable)
                }
            })
        },
        abort_signal,
        deadline,
        timeout_label,
    )
    .await;

    let response = match sent {
        Ok(resp) => resp,
        Err((err, retryable)) => {
            let outcome = if matches!(err, NativeError::Aborted(_)) {
                NativeExchangeOutcome::Aborted
            } else {
                NativeExchangeOutcome::Failed(err.to_string())
            };
            finisher.finish(outcome, false);
            return Err(if retryable && can_retry {
                AttemptError::RetryableConnect(err)
            } else {
                AttemptError::Fatal(err)
            });
        }
    };

    if !is_same_origin(&response.url, target_url) {
        let sanitized_resp_url = sanitize_url(&response.url);
        drop(response);
        let err = NativeError::Transport(FetchError::Other(aimux_core::SourceError::from(
            format!("transport returned cross-origin response URL: {sanitized_resp_url}"),
        )));
        finisher.finish(NativeExchangeOutcome::Failed(err.to_string()), false);
        return Err(AttemptError::Fatal(err));
    }

    if observer_active && let Some(obs) = &observer {
        let sanitized_effective = sanitize_url(&response.url);
        let redacted_head =
            redact_response_headers(&response.headers, protected_headers, protected_prefixes);
        if let Err(reason) = obs.on_response_head(
            exchange_id,
            response.status,
            &redacted_head,
            &sanitized_effective,
        ) {
            observer_active = false;
            finisher.mark_incomplete(reason);
        }
    }

    if can_retry && retryable_statuses.contains(&response.status) {
        let status = response.status;
        drop(response);
        finisher.mark_incomplete(format!(
            "response status {} discarded before retry",
            status.as_u16()
        ));
        finisher.finish(NativeExchangeOutcome::Completed, false);
        return Err(AttemptError::RetryableStatus);
    }

    let FetchResponse {
        status,
        headers,
        url: effective_url,
        body: inner_body,
    } = response;

    let abort_fut = abort_signal
        .map(|sig| Box::pin(sig.cancelled()) as Pin<Box<dyn Future<Output = ()> + Send>>);
    let deadline_sleep = deadline.map(|at| Box::pin(tokio::time::sleep_until(at)));

    let stream = NativeBodyStream {
        inner: Some(inner_body),
        buffered: Bytes::new(),
        max_buffer_bytes,
        abort_signal: abort_signal.cloned(),
        abort_fut,
        deadline,
        timeout_label: timeout_label.map(str::to_owned),
        deadline_sleep,
        finisher,
        observer_state: StreamObserverState {
            observer: if observer_active { observer } else { None },
            exchange_id,
            recorded_bytes: 0,
            max_recorded_bytes,
            active: observer_active,
        },
        terminated: false,
    };

    Ok(NativeResponse {
        status,
        headers,
        effective_url,
        body: stream,
    })
}

fn check_pre_bounds(
    abort_signal: Option<&AbortSignal>,
    deadline: Option<Instant>,
    timeout_label: Option<&str>,
) -> Result<(), NativeError> {
    if abort_signal.is_some_and(AbortSignal::is_aborted) {
        return Err(NativeError::Aborted("request aborted".to_string()));
    }
    if let Some(at) = deadline
        && Instant::now() >= at
    {
        return Err(NativeError::Timeout(
            timeout_label.unwrap_or("request timed out").to_string(),
        ));
    }
    Ok(())
}

async fn wait_for_abort_opt(signal: Option<&AbortSignal>) {
    match signal {
        Some(sig) => sig.cancelled().await,
        None => std::future::pending().await,
    }
}

async fn wait_for_deadline_opt(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn race_with_bounds<T>(
    fut: impl Future<Output = Result<T, (NativeError, bool)>>,
    abort_signal: Option<&AbortSignal>,
    deadline: Option<Instant>,
    timeout_label: Option<&str>,
) -> Result<T, (NativeError, bool)> {
    tokio::select! {
        biased;
        () = wait_for_abort_opt(abort_signal) => {
            Err((NativeError::Aborted("request aborted".to_string()), false))
        }
        () = wait_for_deadline_opt(deadline) => {
            let msg = timeout_label.unwrap_or("request timed out").to_string();
            Err((NativeError::Timeout(msg), false))
        }
        result = fut => result,
    }
}

fn parse_and_normalize_base_url(base_url: &str) -> Result<(Url, String, String), NativeError> {
    if base_url.trim().is_empty() {
        return Err(NativeError::InvalidConfig {
            message: "base_url must be a non-empty URL".to_string(),
        });
    }
    let parsed = Url::parse(base_url).map_err(|err| NativeError::InvalidConfig {
        message: format!("invalid base_url: {err}"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(NativeError::InvalidConfig {
            message: format!(
                "base_url scheme must be http or https, got {}",
                parsed.scheme()
            ),
        });
    }
    if parsed.host_str().map(str::is_empty).unwrap_or(true) || parsed.cannot_be_a_base() {
        return Err(NativeError::InvalidConfig {
            message: "base_url must have a valid host".to_string(),
        });
    }
    let authority_has_at = base_url
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .is_some_and(|authority| authority.contains('@'));
    if !parsed.username().is_empty() || parsed.password().is_some() || authority_has_at {
        return Err(NativeError::InvalidConfig {
            message: "base_url must not contain userinfo".to_string(),
        });
    }
    if parsed.query().is_some() || base_url.contains('?') {
        return Err(NativeError::InvalidConfig {
            message: "base_url must not contain a query string".to_string(),
        });
    }
    if parsed.fragment().is_some() || base_url.contains('#') {
        return Err(NativeError::InvalidConfig {
            message: "base_url must not contain a fragment".to_string(),
        });
    }

    let base_path_prefix = parsed.path().trim_end_matches('/').to_string();
    let mut origin_only = parsed.clone();
    origin_only.set_path("");
    origin_only.set_query(None);
    origin_only.set_fragment(None);
    let origin_prefix_str = origin_only.as_str().trim_end_matches('/').to_string();

    Ok((parsed, origin_prefix_str, base_path_prefix))
}

fn resolve_relative_url(
    base_url: &Url,
    origin_prefix_str: &str,
    base_path_prefix: &str,
    relative_path_and_query: &str,
) -> Result<Url, NativeError> {
    if relative_path_and_query
        .chars()
        .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain control or whitespace characters".to_string(),
        });
    }
    if relative_path_and_query.contains('#') {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain a URL fragment".to_string(),
        });
    }

    let (raw_path, raw_query) = match relative_path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (relative_path_and_query, None),
    };

    if raw_path.starts_with('/') {
        return Err(NativeError::InvalidPath {
            message: "relative path must not start with `/`".to_string(),
        });
    }
    if raw_path.contains('\\') {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain backslashes".to_string(),
        });
    }
    if raw_path.contains("://") {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain a URL scheme".to_string(),
        });
    }
    if raw_path.contains("//") {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain empty `//` segments".to_string(),
        });
    }

    if !raw_path.is_empty() {
        let mut segments = raw_path.split('/').peekable();
        if let Some(first) = segments.peek()
            && first.contains(':')
        {
            return Err(NativeError::InvalidPath {
                message: "first path segment must not contain `:`".to_string(),
            });
        }
        for segment in raw_path.split('/') {
            validate_segment(segment)?;
        }
    }

    let full_path = if raw_path.is_empty() {
        if base_path_prefix.is_empty() {
            "/".to_string()
        } else {
            base_path_prefix.to_string()
        }
    } else if base_path_prefix.is_empty() {
        format!("/{raw_path}")
    } else {
        format!("{base_path_prefix}/{raw_path}")
    };

    let full_str = match raw_query {
        Some(q) => format!("{origin_prefix_str}{full_path}?{q}"),
        None => format!("{origin_prefix_str}{full_path}"),
    };

    let target = Url::parse(&full_str).map_err(|err| NativeError::InvalidPath {
        message: format!("invalid resolved URL: {err}"),
    })?;

    let prefix_ok = if base_path_prefix.is_empty() {
        target.path().starts_with('/')
    } else {
        target.path() == base_path_prefix
            || target
                .path()
                .strip_prefix(base_path_prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    };

    if target.scheme() != base_url.scheme()
        || target.host_str() != base_url.host_str()
        || target.port_or_known_default() != base_url.port_or_known_default()
        || target.username() != base_url.username()
        || target.password() != base_url.password()
        || target.fragment().is_some()
        || !prefix_ok
    {
        return Err(NativeError::InvalidPath {
            message: "resolved URL escaped configured endpoint origin or base path".to_string(),
        });
    }

    if let Some(q) = raw_query {
        validate_query_escapes(q)?;
    }

    Ok(target)
}

fn validate_query_escapes(query: &str) -> Result<(), NativeError> {
    let bytes = query.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            let _ = parse_percent_hex(bytes, idx)?;
            idx += 3;
        } else {
            idx += 1;
        }
    }
    Ok(())
}

fn validate_segment(segment: &str) -> Result<(), NativeError> {
    if segment == "." || segment == ".." {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain `.` or `..` segments".to_string(),
        });
    }
    let decoded = percent_decode_strict(segment)?;
    if decoded.as_slice() == b"." || decoded.as_slice() == b".." {
        return Err(NativeError::InvalidPath {
            message: "relative path must not contain percent-encoded `.` or `..` segments"
                .to_string(),
        });
    }
    if decoded
        .iter()
        .any(|&b| b == b'/' || b == b'\\' || b == 0 || b < 0x20 || b == 0x7f)
    {
        return Err(NativeError::InvalidPath {
            message: "relative path segment contains forbidden decoded characters".to_string(),
        });
    }
    Ok(())
}

fn parse_percent_hex(bytes: &[u8], idx: usize) -> Result<u8, NativeError> {
    let Some(hex_bytes) = bytes.get(idx + 1..idx + 3) else {
        return Err(NativeError::InvalidPath {
            message: "truncated percent-encoding in relative path".to_string(),
        });
    };
    if !hex_bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(NativeError::InvalidPath {
            message: "invalid percent-encoding in relative path".to_string(),
        });
    }
    let hex_slice = std::str::from_utf8(hex_bytes).map_err(|_| NativeError::InvalidPath {
        message: "invalid percent-encoding in relative path".to_string(),
    })?;
    u8::from_str_radix(hex_slice, 16).map_err(|_| NativeError::InvalidPath {
        message: "invalid percent-encoding in relative path".to_string(),
    })
}

fn percent_decode_strict(segment: &str) -> Result<Vec<u8>, NativeError> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            out.push(parse_percent_hex(bytes, idx)?);
            idx += 3;
        } else {
            out.push(bytes[idx]);
            idx += 1;
        }
    }
    Ok(out)
}

fn is_same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().is_some()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
        && a.username() == b.username()
        && a.password() == b.password()
}

fn validate_header_name(name: &str) -> Result<HeaderName, http::header::InvalidHeaderName> {
    HeaderName::try_from(name)
}

fn is_protected_header_name(
    name: &str,
    protected: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
) -> bool {
    let lower = name.to_ascii_lowercase();
    protected.contains(&lower) || prefixes.iter().any(|prefix| lower.starts_with(prefix))
}

fn validate_caller_headers(
    protected: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
    headers: &HeaderMap,
    retry: &NativeRetryPolicy,
) -> Result<(), NativeError> {
    for name in headers.keys() {
        if is_protected_header_name(name.as_str(), protected, prefixes) {
            return Err(NativeError::InvalidHeader {
                name: name.as_str().to_string(),
                message: "header is owned by endpoint authentication and cannot be overridden"
                    .to_string(),
            });
        }
    }
    if let NativeRetryPolicy::Safe(opts) = retry
        && let Some((name, _)) = &opts.idempotency_key
        && is_protected_header_name(name.as_str(), protected, prefixes)
    {
        return Err(NativeError::InvalidHeader {
            name: name.as_str().to_string(),
            message: "idempotency header cannot name an endpoint-protected header".to_string(),
        });
    }
    Ok(())
}

async fn resolve_endpoint_headers(
    endpoint: &NativeEndpoint,
    abort_signal: Option<&AbortSignal>,
    deadline: Option<Instant>,
    timeout_label: Option<&str>,
) -> Result<HeaderMap, NativeError> {
    match &endpoint.auth {
        EndpointAuth::None => Ok(HeaderMap::new()),
        EndpointAuth::ProviderHeaders(provider_headers) => {
            let fut = async {
                let resolved_opt = provider_headers
                    .resolve()
                    .await
                    .map_err(|err| (NativeError::Auth(err), false))?;
                let mut map = HeaderMap::new();
                for (name, value) in normalize_headers(resolved_opt) {
                    let header_name = HeaderName::try_from(name.as_str()).map_err(|_| {
                        (
                            NativeError::InvalidHeader {
                                name: name.clone(),
                                message: "invalid header name produced by endpoint".to_string(),
                            },
                            false,
                        )
                    })?;
                    let header_value = HeaderValue::try_from(value.as_str()).map_err(|_| {
                        (
                            NativeError::InvalidHeader {
                                name: name.clone(),
                                message: "invalid header value produced by endpoint".to_string(),
                            },
                            false,
                        )
                    })?;
                    map.append(header_name, header_value);
                }
                Ok(map)
            };
            race_with_bounds(fut, abort_signal, deadline, timeout_label)
                .await
                .map_err(|(err, _)| err)
        }
        EndpointAuth::CustomResolver(resolver) => {
            let fut = async {
                resolver()
                    .await
                    .map_err(|err| (NativeError::Auth(err), false))
            };
            race_with_bounds(fut, abort_signal, deadline, timeout_label)
                .await
                .map_err(|(err, _)| err)
        }
    }
}

fn assemble_wire_headers(
    endpoint: &NativeEndpoint,
    protected_headers: &BTreeSet<String>,
    resolved_endpoint_headers: HeaderMap,
    caller_headers: &HeaderMap,
    retry: &NativeRetryPolicy,
) -> HeaderMap {
    let mut wire = HeaderMap::new();

    for (name, value) in &resolved_endpoint_headers {
        if is_protected_header_name(
            name.as_str(),
            protected_headers,
            &endpoint.protected_header_prefixes,
        ) {
            wire.append(name.clone(), value.clone());
        } else {
            // Non-protected endpoint defaults apply only when the caller did not specify the name.
            if !caller_headers.contains_key(name) {
                wire.append(name.clone(), value.clone());
            }
        }
    }

    for name in caller_headers.keys() {
        for value in caller_headers.get_all(name) {
            wire.append(name.clone(), value.clone());
        }
    }

    if let NativeRetryPolicy::Safe(opts) = retry
        && let Some((name, value)) = &opts.idempotency_key
        && !value.is_empty()
        && Some(name) == endpoint.idempotency_header.as_ref()
    {
        wire.remove(name);
        wire.append(name.clone(), value.clone());
    }

    wire
}

fn is_redacted_header(
    name: &HeaderName,
    protected: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
) -> bool {
    is_protected_header_name(name.as_str(), protected, prefixes)
        || aimux_core::recording::is_sensitive_key(name.as_str())
}

fn redact_response_headers<'a>(
    headers: &'a HeaderMap,
    protected: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
) -> std::borrow::Cow<'a, HeaderMap> {
    if !headers
        .keys()
        .any(|name| is_redacted_header(name, protected, prefixes))
    {
        return std::borrow::Cow::Borrowed(headers);
    }
    let mut redacted = HeaderMap::try_with_capacity(headers.keys_len()).unwrap_or_default();
    for (name, value) in headers {
        if is_redacted_header(name, protected, prefixes) {
            redacted.append(name.clone(), HeaderValue::from_static("[REDACTED]"));
        } else {
            redacted.append(name.clone(), value.clone());
        }
    }
    std::borrow::Cow::Owned(redacted)
}

fn redact_header_pairs(
    headers: &HeaderMap,
    protected: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let lower = name.as_str().to_ascii_lowercase();
            let redacted = if is_redacted_header(name, protected, prefixes) {
                "[REDACTED]".to_string()
            } else {
                value.to_str().unwrap_or("<binary>").to_string()
            };
            (lower, redacted)
        })
        .collect()
}

fn redact_query_values(raw_query: &str) -> String {
    if raw_query.is_empty() {
        return String::new();
    }
    raw_query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) if key.to_ascii_lowercase().contains("%3d") => {
                "REDACTED=REDACTED".to_string()
            }
            Some((key, _)) => format!("{key}=REDACTED"),
            None if pair.is_empty() => String::new(),
            None => "REDACTED".to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn sanitize_url(url: &Url) -> String {
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.set_fragment(None);
    if let Some(q) = url.query() {
        let redacted = redact_query_values(q);
        sanitized.set_query(Some(&redacted));
    }
    sanitized.to_string()
}

fn sanitize_relative_path_and_query(path_and_query: &str) -> String {
    match path_and_query.split_once('?') {
        Some((path, query)) => format!("{path}?{}", redact_query_values(query)),
        None => path_and_query.to_string(),
    }
}

fn effective_retry_plan<'a>(
    endpoint: &NativeEndpoint,
    method: &Method,
    retry: &'a NativeRetryPolicy,
) -> (u32, Duration, &'a [StatusCode]) {
    let NativeRetryPolicy::Safe(opts) = retry else {
        return (0, Duration::ZERO, &[]);
    };
    let is_safe_method = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    let has_valid_idempotency = match (&endpoint.idempotency_header, &opts.idempotency_key) {
        (Some(supported), Some((supplied, value))) => supplied == supported && !value.is_empty(),
        _ => false,
    };
    if is_safe_method || has_valid_idempotency {
        (
            opts.max_retries,
            opts.initial_backoff,
            &opts.retryable_statuses,
        )
    } else {
        (0, Duration::ZERO, &[])
    }
}

fn can_retry_attempt(attempt: u64, max_retries: u32) -> bool {
    attempt <= u64::from(max_retries)
}

async fn wait_retry_backoff(
    initial_backoff: Duration,
    attempt: u64,
    abort_signal: Option<&AbortSignal>,
    deadline: Option<Instant>,
    timeout_label: Option<&str>,
) -> Result<(), NativeError> {
    let shift = u32::try_from(attempt.saturating_sub(1).min(31)).unwrap_or(31);
    let multiplier = 1u32 << shift;
    let delay = initial_backoff.saturating_mul(multiplier);
    if delay.is_zero() {
        return check_pre_bounds(abort_signal, deadline, timeout_label);
    }
    race_with_bounds(
        async move {
            tokio::time::sleep(delay).await;
            Ok(())
        },
        abort_signal,
        deadline,
        timeout_label,
    )
    .await
    .map_err(|(err, _)| err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_max_retry_attempt_bound_never_overflows() {
        assert!(!can_retry_attempt(1, 0));
        assert!(can_retry_attempt(1, 2));
        assert!(can_retry_attempt(2, 2));
        assert!(!can_retry_attempt(3, 2));

        let penultimate = u64::from(u32::MAX);
        assert!(can_retry_attempt(penultimate, u32::MAX));
        let final_attempt = penultimate + 1;
        assert!(!can_retry_attempt(final_attempt, u32::MAX));
    }
}
