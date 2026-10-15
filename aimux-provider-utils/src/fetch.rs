//! Pluggable HTTP transport: the Rust shape of the AI SDK's `FetchFunction`.
//!
//! Every provider exchange ends in one [`Fetch::fetch`] call. A provider (or
//! its settings) may carry a [`FetchFunction`], as the AI SDK's `fetch`
//! setting: a mock, a signing decorator such as
//! [`crate::sigv4_fetch::SigV4Fetch`], or a [`ReqwestFetch`] with its own
//! [`ProxyConfig`]. When none is set the exchange uses [`default_fetch`],
//! resolved *per request* (never frozen when a provider is created), so the
//! process default can change after a provider exists, as replacing
//! `globalThis.fetch` does in JavaScript.
//!
//! This module owns the real network leaf: [`ReqwestFetch`] (its own
//! connection pool per tokio runtime, its own proxy routing) and
//! [`PinnedFetch`] (a single non-redirecting hop connected only to
//! pre-validated addresses, used by the SSRF download guard). Recording,
//! response handlers and retry stay above this layer, in `http.rs` and
//! `aimux-core`.

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::{HeaderMap, Method, StatusCode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use aimux_core::AbortSignal;

/// Proxy routing of one transport: a [`ReqwestFetch`], a [`PinnedFetch`]
/// hop, or the WebSocket connector. With every field empty, HTTP follows the
/// `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY` environment, as
/// reqwest does, and the WebSocket connector connects directly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub http_url: Option<String>,
    pub https_url: Option<String>,
    pub all_url: Option<String>,
    pub no_proxy: Option<String>,
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
    /// Caller cancellation. The helper layer already drops the in-flight
    /// future when this fires; transports may additionally use it to cancel
    /// work they spawned.
    pub signal: Option<AbortSignal>,
    /// Whole-exchange bound, when the caller wants the transport itself to
    /// enforce one. The helper layer enforces its own response deadline and
    /// leaves this `None`.
    pub timeout: Option<Duration>,
    /// Whether credential decorators may authenticate this request.
    pub credentials: RequestCredentials,
}

/// Whether a credential decorator may authenticate a request, after the
/// `credentials` member of the WHATWG fetch `RequestInit`.
///
/// The helper layer sets [`RequestCredentials::Omit`] once a redirect leaves
/// the origin the credentials belong to. Caller headers are already reduced
/// to `User-Agent` by then; a decorator that adds credentials itself, such as
/// [`crate::sigv4_fetch::SigV4Fetch`], must forward the request untouched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RequestCredentials {
    /// Decorators authenticate the request.
    #[default]
    Include,
    /// Decorators add no credentials.
    Omit,
}

impl FetchRequest {
    /// A request with no headers, no body and no bounds.
    #[must_use]
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            body: Bytes::new(),
            signal: None,
            timeout: None,
            credentials: RequestCredentials::Include,
        }
    }
}

/// A response whose body has not been read yet.
pub struct FetchResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    /// The URL this response came from.
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

/// The transport contract: one request in, one response out. A transport does
/// not follow redirects: a `3xx` is returned as it is and the helper layer
/// (`http.rs`) decides, hop by hop, whether and with which headers to follow.
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

struct DefaultTransport {
    transport: Arc<ReqwestFetch>,
    frozen: bool,
    configured: bool,
}

static DEFAULT_TRANSPORT: OnceLock<RwLock<DefaultTransport>> = OnceLock::new();

fn default_transport_slot() -> &'static RwLock<DefaultTransport> {
    DEFAULT_TRANSPORT.get_or_init(|| {
        RwLock::new(DefaultTransport {
            transport: Arc::new(ReqwestFetch::default()),
            frozen: false,
            configured: false,
        })
    })
}

/// The process default transport, acquired at request time. Acquisition freezes
/// startup proxy configuration for HTTP, WebSocket and guarded downloads alike.
#[must_use]
pub fn default_fetch() -> FetchFunction {
    default_transport()
}

pub(crate) fn default_transport() -> Arc<ReqwestFetch> {
    let mut slot = default_transport_slot()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.frozen = true;
    slot.transport.clone()
}

/// Explicit native replacement of the process default transport. In-flight
/// requests retain their transport; startup configuration stays frozen once used.
pub fn set_default_transport(transport: ReqwestFetch) {
    let mut slot = default_transport_slot()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    slot.transport = Arc::new(transport);
    slot.configured = true;
}

/// Configure the startup proxy; repeating the active configuration is idempotent.
/// # Errors
/// Returns an error if a different configuration is requested after acquisition.
pub fn configure_startup_proxy(config: ProxyConfig) -> Result<(), StartupProxyFrozen> {
    let mut slot = default_transport_slot()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.transport.proxy() != &config {
        if slot.frozen {
            return Err(StartupProxyFrozen);
        }
        slot.transport = Arc::new(ReqwestFetch::new(config));
    }
    slot.configured = true;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("startup proxy is frozen")]
pub struct StartupProxyFrozen;

/// Inspect startup state without acquiring or freezing the transport.
#[must_use]
pub fn startup_proxy_state() -> (bool, bool) {
    let slot = default_transport_slot()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (slot.configured, slot.frozen)
}

/// The real network leaf: reqwest over rustls, routed by its own
/// [`ProxyConfig`], with its own connection pool per tokio runtime (clones
/// share the pool).
///
/// The pool has a connect timeout but no client-wide response timeout: it
/// also serves streaming exchanges, so the non-streaming response bound lives
/// per exchange in the helper layer and `aimux-core` owns the operation
/// deadline.
#[derive(Debug, Clone, Default)]
pub struct ReqwestFetch {
    proxy: ProxyConfig,
    clients: Arc<Mutex<HashMap<Option<tokio::runtime::Id>, Client>>>,
}

impl ReqwestFetch {
    /// A transport routed by `proxy`.
    #[must_use]
    pub fn new(proxy: ProxyConfig) -> Self {
        Self {
            proxy,
            clients: Arc::default(),
        }
    }

    /// The proxy routing of this transport.
    #[must_use]
    pub fn proxy(&self) -> &ProxyConfig {
        &self.proxy
    }

    // One client (and so one connection pool) PER RUNTIME, not per
    // transport: pooled connections are driven by tasks spawned onto the
    // runtime that made the request, so when that runtime shuts down its
    // pooled connections become unusable while staying checked in — and the
    // OS can recycle their ports to fresh servers, handing later requests a
    // dead connection. Production processes run a single runtime and still
    // get exactly one client. Runtimes leave no drop signal, so dead entries
    // are undetectable; the map is instead bounded by `CLIENTS_PER_TRANSPORT`.
    fn client(&self) -> Result<Client, FetchError> {
        let key = tokio::runtime::Handle::try_current().ok().map(|h| h.id());
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let client = build_client(&self.proxy).map_err(|error| {
            FetchError::Other(format!("HTTP client initialization failed: {error}"))
        })?;
        if clients.len() >= CLIENTS_PER_TRANSPORT {
            // Which entries are dead is unknowable, so evict them all;
            // in-flight requests hold their own `Client` clone and are
            // unaffected.
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }
}

#[async_trait]
impl Fetch for ReqwestFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let client = self.client()?;
        send(&client, request).await
    }
}

/// One non-redirecting hop of a validated download. The connection is pinned
/// to exactly the DNS answers that passed the SSRF guard (resolve overrides),
/// defeating TTL-0 rebinding; an empty pin list is the hop on the trusted
/// origin, which connects normally. The hop's proxy configuration is applied so reqwest makes the per-URL
/// routing decision itself: when a proxy carries the request the proxy
/// resolves the target (a trusted transport, the override is unused), and any
/// request the proxy rules send DIRECT still connects only through the
/// validated, pinned addresses.
///
/// A client is built per hop: downloads are rare and each hop is one
/// request, so there is no pool to share — and no stale-runtime pool to
/// reuse.
#[derive(Debug, Clone, Default)]
pub struct PinnedFetch {
    addresses: Vec<IpAddr>,
    proxy: ProxyConfig,
}

impl PinnedFetch {
    /// Pin the connection to these validated addresses, routed by `proxy`.
    #[must_use]
    pub fn new(addresses: Vec<IpAddr>, proxy: ProxyConfig) -> Self {
        Self { addresses, proxy }
    }

    fn client(&self, url: &Url) -> Result<Client, FetchError> {
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(10_000))
            .redirect(reqwest::redirect::Policy::none());
        let builder = apply_proxy(builder, &self.proxy);
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

// Hosts that churn short-lived runtimes (a test binary, an FFI embedder
// creating a runtime per call) would otherwise grow a transport's client map
// without bound. Eviction is safe: correctness only requires never *reusing*
// a dead runtime's pool, and an evicted live runtime simply rebuilds a fresh
// client on its next request.
const CLIENTS_PER_TRANSPORT: usize = 8;

fn build_client(proxy: &ProxyConfig) -> Result<Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(10_000))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Some(Duration::from_secs(30)))
        .tcp_keepalive(Some(Duration::from_secs(20)));
    apply_proxy(builder, proxy)
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

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A fake HTTP proxy answering every request with `ok`; returns its URL
    /// and the request lines it saw.
    async fn fake_proxy() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                    log.lock()
                        .unwrap()
                        .push(text.lines().next().unwrap_or_default().to_string());
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        (format!("http://{address}"), seen)
    }

    fn get(url: &str) -> FetchRequest {
        FetchRequest::new(Method::GET, Url::parse(url).unwrap())
    }

    /// The proxy belongs to the transport, as the AI SDK leaves routing to
    /// the `fetch` a provider is given: a `ReqwestFetch` with a proxy sends
    /// through it, whatever the process default is.
    #[tokio::test]
    async fn a_transport_routes_through_its_own_proxy() {
        let (proxy, seen) = fake_proxy().await;
        let transport = ReqwestFetch::new(ProxyConfig {
            http_url: Some(proxy),
            ..ProxyConfig::default()
        });
        let response = transport
            .fetch(get("http://upstream.invalid/v1/models"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["GET http://upstream.invalid/v1/models HTTP/1.1"]
        );
    }

    /// `default_fetch` is read per request: replacing the default transport
    /// reroutes a transport handle taken later, and the WebSocket connector
    /// and download hops read the same settings.
    #[tokio::test]
    #[serial_test::serial]
    async fn the_default_transport_can_be_replaced() {
        let (proxy, seen) = fake_proxy().await;
        let previous = default_transport();
        let config = ProxyConfig {
            http_url: Some(proxy),
            no_proxy: Some("localhost".to_string()),
            ..ProxyConfig::default()
        };
        set_default_transport(ReqwestFetch::new(config.clone()));
        assert_eq!(default_transport().proxy(), &config);
        let response = default_fetch()
            .fetch(get("http://upstream.invalid/v1"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().len(), 1);
        set_default_transport((*previous).clone());
        assert_eq!(default_transport().proxy(), previous.proxy());
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
