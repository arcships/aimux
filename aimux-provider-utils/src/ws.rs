//! WebSocket client for realtime provider APIs (RFC-0028).
//!
//! Gated behind the `ws` feature (tokio-tungstenite is optional). Provides a
//! minimal, abort/timeout-aware connection wrapper for providers whose
//! realtime APIs are WebSocket-based (OpenAI `gpt-realtime-whisper`
//! transcription today).
//!
//! Design notes (RFC-0028 §3.1):
//! - **Every await point is abort/timeout covered** — `connect`, `send`, and
//!   event receives all `select!` against the abort token and the timeout
//!   timers. This is the WS analogue of the HTTP layer's `send_timed` /
//!   `TimeoutBodyStream` pattern (RFC-0016 R1–R4 precedent: a select on the
//!   loop alone does not cover the send path).
//! - **Backpressure is socket-level**: tungstenite's `send().await` drives
//!   flush and pends while the socket write buffer is full.
//! - **No proxy support**: tokio-tungstenite has no proxy parameter; WS
//!   connections are direct (see RFC-0028 §3.1 / Open Questions).

use std::future::pending;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::options::TimeoutConfiguration;
use aimux_core::shared::AbortSignal;

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

fn ws_error(msg: impl std::fmt::Display) -> AiMuxError {
    AiMuxError::ApiCall(ApiCallError {
        message: msg.to_string(),
        ..Default::default()
    })
}

enum ConnectError {
    Timeout,
    Tungstenite(tokio_tungstenite::tungstenite::Error),
}

async fn connect_with_timeout(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    timeout: Option<std::time::Duration>,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    ),
    ConnectError,
> {
    let fut = tokio_tungstenite::connect_async(request);
    match timeout {
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(inner) => inner.map_err(ConnectError::Tungstenite),
            Err(_) => Err(ConnectError::Timeout),
        },
        None => fut.await.map_err(ConnectError::Tungstenite),
    }
}

/// Open a WebSocket connection. The connect phase races abort and the
/// `first_chunk_ms` timeout (which doubles as the connect timeout).
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

    let connect_timeout = req
        .timeout
        .as_ref()
        .and_then(|t| t.first_chunk_ms)
        .map(tokio::time::Duration::from_millis);

    let (stream, _response) = tokio::select! {
        biased;
        _ = abort_future(&req.abort_signal) => {
            return Err(AiMuxError::Aborted);
        }
        res = connect_with_timeout(http_req, connect_timeout) => match res {
            Ok(v) => v,
            Err(ConnectError::Timeout) => {
                return Err(AiMuxError::Timeout("websocket connect timed out".into()));
            }
            Err(ConnectError::Tungstenite(e)) => {
                return Err(ws_error(format!("websocket connect failed: {e}")));
            }
        },
    };

    let now = tokio::time::Instant::now();
    Ok(WsConnection {
        stream,
        abort: req.abort_signal.clone(),
        first_chunk_deadline: req
            .timeout
            .as_ref()
            .and_then(|t| t.first_chunk_ms)
            .map(|ms| now + tokio::time::Duration::from_millis(ms)),
        chunk_timeout: req
            .timeout
            .as_ref()
            .and_then(|t| t.chunk_ms)
            .map(tokio::time::Duration::from_millis),
        total_deadline: req
            .timeout
            .as_ref()
            .and_then(|t| t.total_ms)
            .map(|ms| now + tokio::time::Duration::from_millis(ms)),
    })
}

impl WsConnection {
    /// Send a text message. Aborted / timed out sends surface as errors; the
    /// pending-while-buffer-full behavior is the socket-level backpressure.
    pub async fn send_text(&mut self, text: &str) -> Result<(), AiMuxError> {
        tokio::select! {
            biased;
            _ = abort_future(&self.abort) => Err(AiMuxError::Aborted),
            _ = total_deadline_future(&self.total_deadline), if self.total_deadline.is_some() => {
                Err(AiMuxError::Timeout("websocket send exceeded total timeout".into()))
            }
            res = self.stream.send(Message::Text(text.to_string())) => {
                res.map_err(|e| ws_error(format!("websocket send failed: {e}")))
            }
        }
    }

    /// Send a binary message (same abort/timeout semantics as `send_text`).
    pub async fn send_binary(&mut self, bytes: &[u8]) -> Result<(), AiMuxError> {
        tokio::select! {
            biased;
            _ = abort_future(&self.abort) => Err(AiMuxError::Aborted),
            _ = total_deadline_future(&self.total_deadline), if self.total_deadline.is_some() => {
                Err(AiMuxError::Timeout("websocket send exceeded total timeout".into()))
            }
            res = self.stream.send(Message::Binary(bytes.to_vec())) => {
                res.map_err(|e| ws_error(format!("websocket send failed: {e}")))
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
                _ = abort_future(&self.abort) => return Some(Err(AiMuxError::Aborted)),
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
                        return Some(Err(ws_error(format!(
                            "websocket closed by peer{detail}"
                        ))));
                    }
                    Some(Err(e)) => {
                        return Some(Err(ws_error(format!("websocket error: {e}"))));
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

async fn total_deadline_future(deadline: &Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(*d).await,
        None => pending().await,
    }
}
