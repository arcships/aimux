//! WebSocket client for realtime provider APIs (RFC-0028).
//!
//! Gated behind the `ws` feature (tokio-tungstenite is optional). Provides a
//! minimal, abort/timeout-aware connection wrapper for providers whose
//! realtime APIs are WebSocket-based (OpenAI `gpt-realtime-whisper`
//! transcription today).
//!
//! Design notes (RFC-0028 §3.1, RFC-0034 §2):
//! - **Every await point is abort/timeout covered** — `connect`, `send`, and
//!   event receives all `select!` against the abort token and the timeout
//!   timers. This is the WS analogue of the HTTP API-call primitive and
//!   Core's semantic stream timeout (RFC-0016 R1–R4 precedent: a select on
//!   the loop alone does not cover the send path).
//! - **Backpressure is socket-level**: tungstenite's `send().await` drives
//!   flush and pends while the socket write buffer is full.
//! - **Proxy support (RFC-0034 §2)**: the global `ProxyConfig` that governs
//!   HTTP also governs `ws://`/`wss://` — `wss` uses `https_url` (or
//!   `all_url`), `ws` uses `http_url` (or `all_url`), `no_proxy` entries are
//!   honored, and tunneled connections are established with a manual HTTP
//!   CONNECT before the WebSocket handshake. tokio-tungstenite has no proxy
//!   support (and will not add it), so the tunnel is ours. SOCKS proxies are
//!   rejected loudly rather than silently bypassed.

use std::future::pending;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use aimux_core::AbortSignal;
use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::options::TimeoutConfiguration;

/// A request to open a WebSocket connection.
pub struct WebSocketRequest {
    /// `wss://` or `ws://` URL.
    pub url: String,
    /// Request headers (e.g. `Authorization`). Rust's WS client sets headers
    /// directly — no subprotocol auth workaround needed (unlike browsers).
    pub headers: Vec<(String, String)>,
    /// Subprotocols to offer (optional; sets `Sec-WebSocket-Protocol`).
    pub subprotocols: Vec<String>,
    /// Abort signal, checked at every await point.
    pub abort_signal: Option<AbortSignal>,
    /// Timeouts, interpreted for WS as: `first_chunk_ms` bounds connect +
    /// session establishment, `chunk_ms` bounds the gap between events,
    /// `total_ms` bounds the whole connection lifetime.
    pub timeout: Option<TimeoutConfiguration>,
}

/// An inbound WebSocket message.
#[derive(Debug, Clone)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

/// A connected WebSocket with abort/timeout enforcement built in.
pub struct WsConnection {
    stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    url: String,
    abort: Option<AbortSignal>,
    /// Deadline for the first event (connect + session ack). Cleared after
    /// the first event arrives.
    first_chunk_deadline: Option<tokio::time::Instant>,
    /// Duration allowed between consecutive events (None = no limit).
    chunk_timeout: Option<std::time::Duration>,
    /// Hard deadline for the whole connection (None = no limit).
    total_deadline: Option<tokio::time::Instant>,
}

/// A future that resolves when the abort signal fires, or never when there is
/// no signal. Lets `select!` treat "no abort" as a permanently pending arm.
async fn abort_future(abort: &Option<AbortSignal>) {
    match abort {
        Some(signal) => signal.cancelled().await,
        None => pending().await,
    }
}

fn abort_error(abort: &Option<AbortSignal>) -> AiMuxError {
    abort
        .as_ref()
        .map(AiMuxError::from_abort_signal)
        .unwrap_or_else(|| AiMuxError::Aborted("request aborted".into()))
}

fn ws_error(url: &str, msg: impl std::fmt::Display) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        is_retryable: true,
        ..ApiCallError::new(
            msg.to_string(),
            crate::http::sanitized_request_url(url),
            serde_json::json!({}),
        )
    }))
}

enum ConnectError {
    Timeout,
    Tungstenite(tokio_tungstenite::tungstenite::Error),
    /// The proxy rejected the CONNECT tunnel with a non-2xx status.
    /// Carries the status and the proxy's response line so the surfaced
    /// error says what the proxy said (407 auth, 403 policy, …).
    ProxyRejected {
        status: u16,
        reason: String,
    },
}

// Rust 1.98 clippy: tungstenite::Error makes the Err variant ~136 bytes.
// Private single-use helper on a cold path (one connect per stream); boxing
// would only move bytes nobody pays for in a loop.
#[allow(clippy::result_large_err)]
async fn connect_with_timeout(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    proxy: Option<ProxyTunnel>,
    timeout: Option<std::time::Duration>,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    ConnectError,
> {
    let fut = async {
        let Some(proxy) = proxy else {
            return tokio_tungstenite::connect_async(request)
                .await
                .map_err(ConnectError::Tungstenite);
        };
        connect_through_proxy(request, &proxy).await
    };
    match timeout {
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(inner) => inner,
            Err(_) => Err(ConnectError::Timeout),
        },
        None => fut.await,
    }
}

/// A resolved proxy tunnel: where to TCP-connect and how to authenticate the
/// CONNECT request (RFC-0034 §2). Public fields are read by the proxy
/// integration tests only (`ws_proxy_test.rs`).
#[doc(hidden)]
#[derive(Debug)]
pub struct ProxyTunnel {
    pub host: String,
    pub port: u16,
    /// `Proxy-Authorization` value (`Basic …`) when the proxy URL carried
    /// userinfo.
    pub authorization: Option<String>,
    /// The TARGET is `wss://` — the tunnel stream needs a TLS upgrade before
    /// the WebSocket handshake.
    pub target_tls: bool,
    /// `host:port` of the WS target as it must appear in the CONNECT line.
    pub target_authority: String,
}

/// Test-only exposure of [`resolve_proxy`] (integration tests construct
/// arbitrary configs without the set-once global).
#[doc(hidden)]
#[allow(non_snake_case)]
pub fn ws__proxy_decision_for(
    target: &url::Url,
    config: &crate::http::ProxyConfig,
) -> Result<Option<ProxyTunnel>, AiMuxError> {
    resolve_proxy(target, config)
}

/// Test-only exposure of [`no_proxy_matches`].
#[doc(hidden)]
#[allow(non_snake_case)]
#[must_use]
pub fn ws__no_proxy_matches(no_proxy: &str, host: &str, port: u16) -> bool {
    no_proxy_matches(no_proxy, host, port)
}

/// Proxy URLs may carry credentials (`http://user:pass@proxy:8080`); error
/// strings must never echo them (they travel to FFI callers and logs). Mask
/// the userinfo portion the same way `sanitized_request_url` protects
/// request URLs.
fn sanitized_proxy_url(raw: &str) -> String {
    if let Some((scheme, rest)) = raw.split_once("://")
        && let Some((_userinfo, host_part)) = rest.split_once('@')
    {
        return format!("{scheme}://***@{host_part}");
    }
    raw.to_string()
}

/// Pick direct vs. tunneled for a WS target under the global proxy config.
fn resolve_proxy(
    target: &url::Url,
    config: &crate::http::ProxyConfig,
) -> Result<Option<ProxyTunnel>, AiMuxError> {
    let raw = match target.scheme() {
        "wss" => config.https_url.clone().or_else(|| config.all_url.clone()),
        "ws" => config.http_url.clone().or_else(|| config.all_url.clone()),
        _ => None,
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let proxy = url::Url::parse(&raw).map_err(|e| {
        AiMuxError::InvalidArgument(format!(
            "invalid proxy URL {}: {e}",
            sanitized_proxy_url(&raw)
        ))
    })?;
    let scheme = proxy.scheme().to_ascii_lowercase();
    if scheme != "http" {
        // SOCKS is untunnelable via CONNECT; `https` proxies need TLS to the
        // proxy itself (TLS-in-TLS) which has no demonstrated need. Both fail
        // loudly — never silently bypass a configured proxy (RFC-0034 D2).
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "WebSocket proxy tunneling supports http proxies only, got {scheme:?} ({}); \
             refusing to bypass the configured proxy with a direct connection",
            sanitized_proxy_url(&raw)
        )));
    }
    let host = proxy
        .host_str()
        .ok_or_else(|| {
            AiMuxError::InvalidArgument(format!(
                "proxy URL has no host: {}",
                sanitized_proxy_url(&raw)
            ))
        })?
        // `host_str` returns IPv6 literals WITH brackets; the socket resolver
        // wants the bare address.
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = proxy.port().unwrap_or(80);
    let authorization = if proxy.username().is_empty() && proxy.password().is_none() {
        None
    } else {
        let credentials = format!("{}:{}", proxy.username(), proxy.password().unwrap_or(""));
        use base64::Engine as _;
        Some(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        ))
    };
    let target_host = target.host_str().unwrap_or_default();
    let target_port = target
        .port_or_known_default()
        .unwrap_or(if target.scheme() == "wss" { 443 } else { 80 });
    Ok(Some(ProxyTunnel {
        host,
        port,
        authorization,
        target_tls: target.scheme() == "wss",
        target_authority: format!("{target_host}:{target_port}"),
    }))
}

/// `no_proxy` matching aligned with reqwest's `NoProxy::from_string`
/// semantics (RFC-0034 §2.1): comma-separated entries; `*` matches
/// everything; an entry matches by exact host or dot-suffix; an entry with
/// an explicit `:port` additionally requires the port to match.
///
/// Known divergence: reqwest additionally supports IP/CIDR entries
/// (`10.0.0.0/8`); those are matched here as literal strings only (they will
/// not match a CIDR-style entry). Provider WS targets are DNS names, and a
/// non-matching entry means "tunnel through the proxy" — the safe direction.
fn no_proxy_matches(no_proxy: &str, host: &str, port: u16) -> bool {
    for entry in no_proxy.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if entry == "*" {
            return true;
        }
        let (name, port_entry) = match entry.rsplit_once(':') {
            Some((name, digits))
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
            {
                (name, Some(digits))
            }
            _ => (entry, None),
        };
        let name = name.trim_start_matches('.').to_ascii_lowercase();
        let host = host.to_ascii_lowercase();
        if (host == name || host.ends_with(&format!(".{name}")))
            && port_entry.is_none_or(|p| p.parse::<u16>() == Ok(port))
        {
            return true;
        }
    }
    false
}

/// TCP-connect to the proxy, issue CONNECT, validate the 2xx response, then
/// run the WebSocket handshake (with TLS for `wss://` targets) over the
/// tunnel stream. Every step is bounded by the caller's timeout; abort is
/// enforced by `ws_connect`'s select dropping this future.
// Same cold-path rationale as `connect_with_timeout` above.
#[allow(clippy::result_large_err)]
async fn connect_through_proxy(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    proxy: &ProxyTunnel,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    ConnectError,
> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|e| ConnectError::Tungstenite(tokio_tungstenite::tungstenite::Error::Io(e)))?;

    let mut connect_request = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\n",
        proxy.target_authority, proxy.target_authority
    );
    if let Some(authorization) = &proxy.authorization {
        connect_request.push_str(&format!("Proxy-Authorization: {authorization}\r\n"));
    }
    connect_request.push_str("\r\n");
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(|e| ConnectError::Tungstenite(tokio_tungstenite::tungstenite::Error::Io(e)))?;

    // Read until end of headers, bounded (a hostile proxy must not be able to
    // feed us an unbounded "response").
    let mut response = Vec::with_capacity(256);
    let mut chunk = [0u8; 512];
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| ConnectError::Tungstenite(tokio_tungstenite::tungstenite::Error::Io(e)))?;
        if n == 0 {
            return Err(ConnectError::ProxyRejected {
                status: 0,
                reason: "proxy closed the connection before responding to CONNECT".into(),
            });
        }
        response.extend_from_slice(&chunk[..n]);
        if response.len() > 8 * 1024 {
            return Err(ConnectError::ProxyRejected {
                status: 0,
                reason: "proxy CONNECT response headers exceed 8 KiB".into(),
            });
        }
    }
    let response_text = String::from_utf8_lossy(&response);
    let status_line = response_text.lines().next().unwrap_or_default();
    // "HTTP/1.1 200 Connection established" → parse the middle token.
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(ConnectError::ProxyRejected {
            status,
            reason: status_line.to_string(),
        });
    }

    // TLS upgrade for wss targets: same roots as the reqwest client
    // (webpki-roots), explicit ring provider so a build where multiple
    // rustls CryptoProvider features unify cannot panic on the implicit
    // default.
    let connector = if proxy.target_tls {
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| {
                ConnectError::Tungstenite(tokio_tungstenite::tungstenite::Error::Io(
                    std::io::Error::other(format!("building rustls client config: {e}")),
                ))
            })?
            .with_root_certificates(roots)
            .with_no_client_auth();
        tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(config))
    } else {
        tokio_tungstenite::Connector::Plain
    };

    let (websocket, response) =
        tokio_tungstenite::client_async_tls_with_config(request, stream, None, Some(connector))
            .await
            .map_err(ConnectError::Tungstenite)?;
    Ok((websocket, response))
}

/// Open a WebSocket connection. The connect phase races abort and the
/// `first_chunk_ms` timeout (which doubles as the connect timeout).
///
/// # Errors
///
/// Returns `InvalidArgument` for a bad URL or header value, `Aborted` when
/// cancellation wins the connect race, and `Timeout` when `first_chunk_ms`
/// expires while connecting.
pub async fn ws_connect(req: &WebSocketRequest) -> Result<WsConnection, AiMuxError> {
    let mut http_req = req
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| AiMuxError::InvalidArgument(format!("invalid WebSocket URL: {e}")))?;
    for (k, v) in &req.headers {
        http_req.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::HeaderName::try_from(k.as_str()).map_err(
                |e| AiMuxError::InvalidArgument(format!("invalid WS header name {k}: {e}")),
            )?,
            HeaderValue::from_str(v).map_err(|e| {
                AiMuxError::InvalidArgument(format!("invalid WS header value for {k}: {e}"))
            })?,
        );
    }
    if !req.subprotocols.is_empty() {
        http_req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_str(&req.subprotocols.join(", ")).map_err(|e| {
                AiMuxError::InvalidArgument(format!("invalid WS subprotocols: {e}"))
            })?,
        );
    }

    // `first_chunk_ms` is ONE combined budget for connect + first event
    // (RFC §3.1): anchor the deadline here, spend the remainder on the
    // post-connect first-event wait.
    let first_chunk_deadline = req
        .timeout
        .as_ref()
        .and_then(|t| t.first_chunk_ms)
        .map(|ms| tokio::time::Instant::now() + tokio::time::Duration::from_millis(ms));
    let connect_deadline = first_chunk_deadline;

    // Proxy resolution (RFC-0034 §2): the global ProxyConfig governs WS too.
    // no_proxy hits and a proxy-less config keep the direct path unchanged.
    let target = url::Url::parse(&req.url)
        .map_err(|e| AiMuxError::InvalidArgument(format!("invalid WebSocket URL: {e}")))?;
    let proxy = {
        let config = crate::http::global_proxy();
        let target_host = target.host_str().unwrap_or_default();
        let target_port = target.port_or_known_default().unwrap_or(80);
        if config
            .no_proxy
            .as_deref()
            .is_some_and(|list| no_proxy_matches(list, target_host, target_port))
        {
            None
        } else {
            resolve_proxy(&target, &config)?
        }
    };

    let (stream, _response) = tokio::select! {
        biased;
        _ = abort_future(&req.abort_signal) => {
            return Err(abort_error(&req.abort_signal));
        }
        res = connect_with_timeout(http_req, proxy, connect_deadline.map(|d| d - tokio::time::Instant::now())) => match res {
            Ok(v) => v,
            Err(ConnectError::Timeout) => {
                return Err(AiMuxError::Timeout("websocket connect timed out".into()));
            }
            Err(ConnectError::ProxyRejected { status, reason }) => {
                // Classify by the shared rule like the handshake rejection
                // below: 407/403 are auth/policy verdicts (never retried),
                // but a proxy 502/503/504 is transient — same as HTTP.
                // `status == 0` (EOF before responding / oversized headers)
                // behaves like a dropped connection: transient.
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: (status != 0).then_some(status),
                    is_retryable: if status == 0 {
                        true
                    } else {
                        aimux_core::error::is_retryable_status(status)
                    },
                    ..ApiCallError::new(
                        format!("proxy rejected websocket CONNECT: {reason}"),
                        crate::http::sanitized_request_url(&req.url),
                        serde_json::json!({}),
                    )
                })));
            }
            // An HTTP handshake rejection carries a real status: keep it and
            // classify retryability by the shared rule instead of the blanket
            // transport `is_retryable = true` (401/403 must not be retried).
            Err(ConnectError::Tungstenite(tokio_tungstenite::tungstenite::Error::Http(
                response,
            ))) => {
                let status = response.status().as_u16();
                return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                    status_code: Some(status),
                    is_retryable: aimux_core::error::is_retryable_status(status),
                    response_body: response
                        .body()
                        .as_ref()
                        .map(|body| String::from_utf8_lossy(body).into_owned()),
                    ..ApiCallError::new(
                        format!("websocket connect rejected: HTTP {status}"),
                        crate::http::sanitized_request_url(&req.url),
                        serde_json::json!({}),
                    )
                })));
            }
            Err(ConnectError::Tungstenite(e)) => {
                return Err(ws_error(&req.url, format!("websocket connect failed: {e}")));
            }
        },
    };

    Ok(WsConnection {
        stream,
        url: req.url.clone(),
        abort: req.abort_signal.clone(),
        // The REMAINING first-chunk budget (anchored before connect) — not a
        // fresh full window, so connect+ack can never exceed first_chunk_ms.
        first_chunk_deadline,
        chunk_timeout: req
            .timeout
            .as_ref()
            .and_then(|t| t.chunk_ms)
            .map(tokio::time::Duration::from_millis),
        total_deadline: req
            .timeout
            .as_ref()
            .and_then(|t| t.total_ms)
            .map(|ms| tokio::time::Instant::now() + tokio::time::Duration::from_millis(ms)),
    })
}

impl WsConnection {
    /// Send a text message. Aborted / timed out sends surface as errors; the
    /// pending-while-buffer-full behavior is the socket-level backpressure.
    ///
    /// # Errors
    ///
    /// Returns `Aborted` on cancellation, `Timeout` when the send exceeds the
    /// total timeout, and a WebSocket error mapped to `ApiCall` when the socket
    /// write fails.
    pub async fn send_text(&mut self, text: &str) -> Result<(), AiMuxError> {
        tokio::select! {
            biased;
            _ = abort_future(&self.abort) => Err(abort_error(&self.abort)),
            _ = deadline_future(self.total_deadline), if self.total_deadline.is_some() => {
                Err(AiMuxError::Timeout("websocket send exceeded total timeout".into()))
            }
            res = self.stream.send(Message::Text(text.to_string())) => {
                res.map_err(|e| ws_error(&self.url, format!("websocket send failed: {e}")))
            }
        }
    }

    /// Send a binary message (same abort/timeout semantics as `send_text`).
    ///
    /// # Errors
    ///
    /// Returns `Aborted` on cancellation, `Timeout` when the send exceeds the
    /// total timeout, and a WebSocket error mapped to `ApiCall` when the socket
    /// write fails.
    pub async fn send_binary(&mut self, bytes: &[u8]) -> Result<(), AiMuxError> {
        tokio::select! {
            biased;
            _ = abort_future(&self.abort) => Err(abort_error(&self.abort)),
            _ = deadline_future(self.total_deadline), if self.total_deadline.is_some() => {
                Err(AiMuxError::Timeout("websocket send exceeded total timeout".into()))
            }
            res = self.stream.send(Message::Binary(bytes.to_vec())) => {
                res.map_err(|e| ws_error(&self.url, format!("websocket send failed: {e}")))
            }
        }
    }

    /// Receive the next message. `None` = the peer closed cleanly with no
    /// close frame (or the stream simply ended). A peer close frame surfaces
    /// as `Err` carrying the close code (RFC §3.1 error mapping). Enforces
    /// first-chunk / chunk-idle / total timeouts and abort. Control frames
    /// (ping/pong) are handled by tungstenite internally and skipped — only
    /// data frames surface (they do NOT reset the chunk-idle deadline: a
    /// half-alive peer pinging forever must still trip the idle timer).
    pub async fn next(&mut self) -> Option<Result<WsMessage, AiMuxError>> {
        // Compute the idle deadline ONCE per call: control-frame `continue`s
        // must not re-extend the window.
        let chunk_idle = self.current_chunk_deadline();
        loop {
            tokio::select! {
                biased;
                _ = abort_future(&self.abort) => return Some(Err(abort_error(&self.abort))),
                _ = deadline_future(self.first_chunk_deadline), if self.first_chunk_deadline.is_some() => {
                    return Some(Err(AiMuxError::Timeout("timed out waiting for first websocket event".into())));
                }
                _ = deadline_future(chunk_idle), if chunk_idle.is_some() => {
                    return Some(Err(AiMuxError::Timeout("websocket chunk idle timeout".into())));
                }
                _ = deadline_future(self.total_deadline), if self.total_deadline.is_some() => {
                    return Some(Err(AiMuxError::Timeout("websocket total timeout".into())));
                }
                msg = self.stream.next() => match msg {
                    None => return None,
                    Some(Ok(Message::Text(t))) => {
                        self.on_event();
                        return Some(Ok(WsMessage::Text(t)));
                    }
                    Some(Ok(Message::Binary(b))) => {
                        self.on_event();
                        return Some(Ok(WsMessage::Binary(b.to_vec())));
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {
                        // Handled inside tungstenite; wait for the next frame.
                        continue;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        // Surface the peer's close code/reason (auth/quota
                        // failures often arrive this way) instead of a bare
                        // EOF.
                        let detail = frame
                            .map(|f| format!(" (code {}: {})", u16::from(f.code), f.reason))
                            .unwrap_or_default();
                        return Some(Err(ws_error(&self.url, format!(
                            "websocket closed by peer{detail}"
                        ))));
                    }
                    Some(Err(e)) => {
                        return Some(Err(ws_error(&self.url, format!("websocket error: {e}"))));
                    }
                },
            }
        }
    }

    /// Close the connection with a normal-completion code (1000). Bounded:
    /// a close handshake against a dead peer must not hang the caller (the
    /// send pends while the socket buffer can't drain).
    pub async fn close(&mut self) {
        let close_fut = self.stream.close(Some(
            tokio_tungstenite::tungstenite::protocol::frame::CloseFrame {
                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: std::borrow::Cow::Borrowed("finished"),
            },
        ));
        tokio::select! {
            res = close_fut => {
                let _ = res;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                tracing::warn!(
                    "aimux: websocket close handshake timed out; abandoning socket"
                );
            }
        }
    }

    /// After an inbound event: the first-chunk deadline is spent. The
    /// chunk-idle window naturally restarts on the next `next()` call.
    fn on_event(&mut self) {
        self.first_chunk_deadline = None;
    }

    fn current_chunk_deadline(&self) -> Option<tokio::time::Instant> {
        self.chunk_timeout.map(|d| tokio::time::Instant::now() + d)
    }
}

/// A sleep that never fires when there is no deadline; `select!` guards with
/// `if deadline.is_some()` anyway, but keep the future total.
async fn deadline_future(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => pending().await,
    }
}
