//! Pluggable HTTP transport: the Rust shape of the AI SDK's `FetchFunction`.
//!
//! Every provider exchange ends in one [`Fetch::fetch`] call. A provider (or
//! its settings) may carry a [`FetchFunction`] — a mock, a signing decorator
//! such as [`crate::sigv4_fetch::SigV4Fetch`], a proxy-aware client — and
//! when none is set the exchange uses [`default_fetch`], resolved *per
//! request* (never frozen when a provider is created), so a process-wide
//! default can change after a provider exists.
//!
//! This module owns the real network leaf: [`ReqwestFetch`] (one shared
//! connection pool per tokio runtime) and [`PinnedFetch`] (a single
//! non-redirecting hop connected only to pre-validated addresses, used by the
//! SSRF download guard). Recording, response handlers and retry stay above
//! this layer, in `http.rs` and `aimux-core`.

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::{HeaderMap, Method, StatusCode};
use reqwest::Client;
use url::Url;

use aimux_core::AbortSignal;

use crate::http::{ProxyConfig, global_proxy};

/// How a transport treats `3xx` responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedirectPolicy {
    /// Follow redirects inside the transport (the ordinary API-call path).
    #[default]
    Follow,
    /// Return the `3xx` response as-is. The caller follows redirects itself
    /// and re-validates every hop (the SSRF download guard).
    Manual,
}

/// One outbound HTTP request, fully materialised: body bytes are final
/// (multipart already encoded), so a decorator can sign or inspect exactly
/// what goes on the wire.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub redirect: RedirectPolicy,
    /// Caller cancellation. The helper layer already drops the in-flight
    /// future when this fires; transports may additionally use it to cancel
    /// work they spawned.
    pub signal: Option<AbortSignal>,
    /// Whole-exchange bound, when the caller wants the transport itself to
    /// enforce one. The helper layer enforces its own response deadline and
    /// leaves this `None`.
    pub timeout: Option<Duration>,
}

impl FetchRequest {
    /// A request with no headers, no body, redirects followed and no bounds.
    #[must_use]
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            body: Bytes::new(),
            redirect: RedirectPolicy::Follow,
            signal: None,
            timeout: None,
        }
    }
}

/// A response whose body has not been read yet.
pub struct FetchResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    /// The final URL (after any redirects the transport followed).
    pub url: Url,
    pub body: BoxStream<'static, Result<Bytes, FetchError>>,
}

impl FetchResponse {
    /// A response whose whole body is already in memory (mocks, tests).
    #[must_use]
    pub fn from_bytes(status: StatusCode, headers: HeaderMap, url: Url, body: Bytes) -> Self {
        Self {
            status,
            headers,
            url,
            body: futures::stream::once(async move { Ok(body) }).boxed(),
        }
    }

    /// The HTTP status.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The response headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The declared `Content-Length`, when present and well-formed.
    #[must_use]
    pub fn content_length(&self) -> Option<u64> {
        self.headers
            .get(http::header::CONTENT_LENGTH)?
            .to_str()
            .ok()?
            .parse()
            .ok()
    }

    /// Consume the response into its body stream.
    #[must_use]
    pub fn bytes_stream(self) -> BoxStream<'static, Result<Bytes, FetchError>> {
        self.body
    }
}

impl fmt::Debug for FetchResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

/// A transport-level failure: no HTTP response was produced, or the body
/// stream broke. HTTP error statuses are *not* errors here — they come back
/// as a [`FetchResponse`] and are interpreted by the response handlers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The connection could not be established.
    #[error("{0}")]
    Connect(String),
    /// The transport's own timeout fired.
    #[error("request timed out")]
    Timeout,
    /// The caller's [`AbortSignal`] fired.
    #[error("request aborted")]
    Aborted,
    /// The connection failed while sending the request or reading the body.
    #[error("{0}")]
    Io(String),
    /// Anything else, including requests the transport refuses to build.
    #[error("{0}")]
    Other(String),
}

impl FetchError {
    /// Whether an operation retry may help. Connection, timeout and I/O
    /// failures are transient; an abort is the caller's decision and `Other`
    /// (a malformed request, a broken transport) will fail the same way again.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Connect(_) | Self::Timeout | Self::Io(_))
    }
}

/// The transport contract: one request in, one response out.
#[async_trait]
pub trait Fetch: Send + Sync + 'static {
    /// Perform one HTTP exchange.
    ///
    /// # Errors
    ///
    /// Returns a [`FetchError`] when no response could be produced.
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError>;
}

impl fmt::Debug for dyn Fetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dyn Fetch")
    }
}

/// A shareable transport, as carried by provider settings and requests.
pub type FetchFunction = Arc<dyn Fetch>;

/// The process-wide default transport, resolved at request time.
#[must_use]
pub fn default_fetch() -> FetchFunction {
    static DEFAULT: OnceLock<FetchFunction> = OnceLock::new();
    DEFAULT.get_or_init(|| Arc::new(ReqwestFetch)).clone()
}

/// The default transport: reqwest over rustls, with a connection pool shared
/// by every `ReqwestFetch` in the process (one pool per tokio runtime).
///
/// The pool has a connect timeout but no client-wide response timeout: it
/// also serves streaming exchanges, so the non-streaming response bound lives
/// per exchange in the helper layer and `aimux-core` owns the operation
/// deadline. The process-wide [`ProxyConfig`] is snapshotted when a runtime's
/// client is first built.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReqwestFetch;

#[async_trait]
impl Fetch for ReqwestFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        if request.redirect == RedirectPolicy::Manual {
            // The pooled client follows redirects; a manual hop needs a
            // non-redirecting client.
            return PinnedFetch::unpinned().fetch(request).await;
        }
        let client = shared_client()?;
        send(&client, request).await
    }
}

/// One non-redirecting hop of a validated download. The connection is pinned
/// to exactly the DNS answers that passed the SSRF guard (resolve overrides),
/// defeating TTL-0 rebinding; [`PinnedFetch::unpinned`] is the hop on the
/// trusted origin, which connects normally but still never follows a
/// redirect. The proxy configuration is applied so reqwest makes the per-URL
/// routing decision itself: when a proxy carries the request the proxy
/// resolves the target (a trusted transport, the override is unused), and any
/// request the proxy rules send DIRECT still connects only through the
/// validated, pinned addresses.
///
/// A client is built per hop: downloads are rare and each hop is one
/// request, so there is no pool to share — and no stale-runtime pool to
/// reuse. Always behaves as [`RedirectPolicy::Manual`].
#[derive(Debug, Clone, Default)]
pub struct PinnedFetch {
    addresses: Vec<IpAddr>,
}

impl PinnedFetch {
    /// Pin the connection to these validated addresses.
    #[must_use]
    pub fn new(addresses: Vec<IpAddr>) -> Self {
        Self { addresses }
    }

    /// No pinning: the hop targets the trusted origin.
    #[must_use]
    pub fn unpinned() -> Self {
        Self::default()
    }

    fn client(&self, url: &Url) -> Result<Client, FetchError> {
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(10_000))
            .redirect(reqwest::redirect::Policy::none());
        let builder = apply_proxy(builder, &global_proxy());
        if self.addresses.is_empty() {
            return builder.build().map_err(|error| {
                FetchError::Other(format!("download client initialization failed: {error}"))
            });
        }
        let host = url
            .host_str()
            .ok_or_else(|| FetchError::Other("download url has no host".to_string()))?;
        let port = url.port_or_known_default().unwrap_or(80);
        let socket_addresses: Vec<_> = self
            .addresses
            .iter()
            .map(|address| std::net::SocketAddr::new(*address, port))
            .collect();
        builder
            .resolve_to_addrs(host, &socket_addresses)
            .build()
            .map_err(|error| {
                FetchError::Other(format!(
                    "pinned download client initialization failed: {error}"
                ))
            })
    }
}

#[async_trait]
impl Fetch for PinnedFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let client = self.client(&request.url)?;
        send(&client, request).await
    }
}

async fn send(client: &Client, request: FetchRequest) -> Result<FetchResponse, FetchError> {
    let mut builder = client
        .request(request.method, request.url)
        .headers(request.headers);
    if !request.body.is_empty() {
        builder = builder.body(request.body);
    }
    if let Some(timeout) = request.timeout {
        builder = builder.timeout(timeout);
    }
    let response = builder.send().await.map_err(map_reqwest_error)?;
    let status = response.status();
    let headers = response.headers().clone();
    let url = response.url().clone();
    Ok(FetchResponse {
        status,
        headers,
        url,
        body: response
            .bytes_stream()
            .map(|chunk| chunk.map_err(map_reqwest_error))
            .boxed(),
    })
}

fn map_reqwest_error(error: reqwest::Error) -> FetchError {
    if error.is_builder() {
        FetchError::Other(error.to_string())
    } else if error.is_connect() {
        FetchError::Connect(error.to_string())
    } else if error.is_timeout() {
        FetchError::Timeout
    } else {
        FetchError::Io(error.to_string())
    }
}

// One client (and so one connection pool) PER RUNTIME, not per process:
// pooled connections are driven by tasks spawned onto the runtime that
// made the request, so when that runtime shuts down its pooled
// connections become unusable while staying checked in — and the OS can
// recycle their ports to fresh servers, handing later requests a dead
// connection. Production processes run a single runtime and still get
// exactly one client. Runtimes leave no drop signal, so dead entries are
// undetectable; the map is instead bounded by `SHARED_CLIENT_CAP` — see
// `shared_client`.
static SHARED: OnceLock<Mutex<HashMap<Option<tokio::runtime::Id>, Client>>> = OnceLock::new();

// Hosts that churn short-lived runtimes (a test binary, an FFI embedder
// creating a runtime per call) would otherwise grow the map without bound.
// Eviction is safe: correctness only requires never *reusing* a dead
// runtime's pool, and an evicted live runtime simply rebuilds a fresh
// client on its next request.
const SHARED_CLIENT_CAP: usize = 8;

/// Return the shared client for the current runtime, building it on first use.
fn shared_client() -> Result<Client, FetchError> {
    let key = tokio::runtime::Handle::try_current().ok().map(|h| h.id());
    let mut clients = SHARED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("shared HTTP client mutex poisoned");
    if let Some(client) = clients.get(&key) {
        return Ok(client.clone());
    }
    let client = build_client(global_proxy()).map_err(|error| {
        FetchError::Other(format!("shared HTTP client initialization failed: {error}"))
    })?;
    if clients.len() >= SHARED_CLIENT_CAP {
        // Which entries are dead is unknowable, so evict them all; in-flight
        // requests hold their own `Client` clone and are unaffected.
        clients.clear();
    }
    clients.insert(key, client.clone());
    Ok(client)
}

fn build_client(proxy: ProxyConfig) -> Result<Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(10_000))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Some(Duration::from_secs(30)))
        .tcp_keepalive(Some(Duration::from_secs(20)));
    apply_proxy(builder, &proxy)
        .build()
        .map_err(|error| error.to_string())
}

fn apply_proxy(mut builder: reqwest::ClientBuilder, proxy: &ProxyConfig) -> reqwest::ClientBuilder {
    let http = proxy.http_url.as_deref().or(proxy.all_url.as_deref());
    let https = proxy.https_url.as_deref().or(proxy.all_url.as_deref());
    if let Some(url) = http
        && let Ok(reqwest_proxy) = reqwest::Proxy::http(url)
    {
        builder = apply_no_proxy(builder, reqwest_proxy, &proxy.no_proxy);
    }
    if let Some(url) = https
        && let Ok(reqwest_proxy) = reqwest::Proxy::https(url)
    {
        builder = apply_no_proxy(builder, reqwest_proxy, &proxy.no_proxy);
    }
    builder
}

// Kept separate so both proxy schemes use precisely the same no-proxy rule.
fn apply_no_proxy(
    builder: reqwest::ClientBuilder,
    proxy: reqwest::Proxy,
    no_proxy: &Option<String>,
) -> reqwest::ClientBuilder {
    match no_proxy.as_deref().map(reqwest::NoProxy::from_string) {
        Some(Some(no_proxy)) => builder.proxy(proxy.no_proxy(Some(no_proxy))),
        _ => builder.proxy(proxy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryability_follows_the_failure_kind() {
        assert!(FetchError::Connect("refused".into()).is_retryable());
        assert!(FetchError::Timeout.is_retryable());
        assert!(FetchError::Io("reset".into()).is_retryable());
        assert!(!FetchError::Aborted.is_retryable());
        assert!(!FetchError::Other("bad request".into()).is_retryable());
    }

    #[tokio::test]
    async fn from_bytes_serves_the_body_once() {
        let url = Url::parse("https://example.test/").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_LENGTH, "5".parse().unwrap());
        let response =
            FetchResponse::from_bytes(StatusCode::OK, headers, url, Bytes::from("hello"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.content_length(), Some(5));
        let chunks: Vec<_> = response.bytes_stream().collect().await;
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap(), &Bytes::from("hello"));
    }

    #[test]
    fn default_fetch_is_a_single_process_wide_leaf() {
        assert!(Arc::ptr_eq(&default_fetch(), &default_fetch()));
    }
}
