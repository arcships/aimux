//! HTTP 层 — 共享 client + 请求/响应抽象。
//!
//! 本模块是 aimux 的 HTTP 边界：provider 给出纯数据的 [`HttpRequest`]，本
//! 模块负责执行（含连接池、超时、retry），返回 [`HttpResponse`]（非流式）或
//! [`HttpStreamResponse`]（流式）。**reqwest 类型完全不外泄到 provider**——
//! provider 不持有 `Client`、不构造 `RequestBuilder`、不碰 `reqwest::Response`。
//!
//! 三个职责：
//! - **连接池**：`shared_client()` / `shared_streaming_client()` 返回 `&'static
//!   Client` 全局单例，TLS 会话与连接池全仓复用（RFC-0009 §4.1）。替代散落
//!   各处的 `Client::new()`。
//! - **超时**：非流式带 30s 整体超时；流式禁用整体超时（LLM 流式时长取决于
//!   生成长度，固定超时会误杀长生成，RFC-0009 §4.3）。
//! - **retry**：429/5xx 重试 + Full Jitter 退避（RFC-0009 §4.2）。retry 是本
//!   模块内部逻辑——provider 不感知重试发生。

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use futures::stream::{BoxStream, Stream, StreamExt};
use reqwest::Client;

use aimux_core::AiMuxError;
use aimux_core::options::TimeoutConfiguration;
use aimux_core::shared::AbortSignal;

use crate::logging::{auto_init_from_env, body_logging_enabled, redact_body};
use crate::response::{ErrorStructure, parse_provider_error};
use crate::retry::{RetryConfig, get_retry_delay_ms_with_jitter, parse_retry_after};

// ════════════════════════════════════════════════════════════════════════════
// 连接池 & 超时配置
// ════════════════════════════════════════════════════════════════════════════

/// 连接池配置（参考 catcher `PoolConfig` 字段设计）。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 每个主机的最大空闲连接数。catcher 默认 10。
    pub max_idle_per_host: usize,
    /// 空闲连接超时（秒）。catcher 默认 30 — 防 retry 复用死连接。
    pub idle_timeout_secs: u64,
    /// 是否启用 TCP keepalive。
    pub keep_alive: bool,
    /// TCP keepalive 间隔（秒）。catcher 默认 20 — 更快发现死连接。
    pub keep_alive_interval_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 10,
            idle_timeout_secs: 30,
            keep_alive: true,
            keep_alive_interval_secs: 20,
        }
    }
}

/// 超时配置（参考 catcher `HttpClientConfig` 的两个超时字段）。
#[derive(Debug, Clone, Copy)]
pub struct TimeoutConfig {
    /// 建连超时（毫秒）。catcher 默认 10_000。
    pub connect_timeout_ms: u64,
    /// 整体响应超时（毫秒）。传 0 禁用整体超时（流式用）。
    /// catcher 默认 30_000。
    pub response_timeout_ms: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 10_000,
            response_timeout_ms: 30_000,
        }
    }
}

impl TimeoutConfig {
    /// 流式超时配置：仅 connect timeout，禁用整体超时。
    ///
    /// LLM 流式时长取决于生成长度 / max_tokens，固定整体超时会误杀长
    /// 生成请求（RFC-0009 §4.3）。仅保留 connect timeout 守护建连阶段。
    pub fn streaming() -> Self {
        Self {
            connect_timeout_ms: 10_000,
            response_timeout_ms: 0,
        }
    }
}

/// 全局共享的 reqwest::Client（非流式，带 30s 整体超时）。
static SHARED: OnceLock<Client> = OnceLock::new();

/// 全局共享的流式 reqwest::Client（无整体超时）。
static SHARED_STREAMING: OnceLock<Client> = OnceLock::new();

/// 获取（或惰性初始化）共享 reqwest Client（带 30s 整体超时，非流式用）。
///
/// 返回 `&'static Client`——provider **拿引用即用**，不持有、不 clone、不传参。
pub fn shared_client() -> &'static Client {
    SHARED.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::default()))
}

/// 获取（或惰性初始化）流式共享 reqwest Client（无整体超时，流式用）。
pub fn shared_streaming_client() -> &'static Client {
    SHARED_STREAMING.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::streaming()))
}

/// 用给定配置构建一个 reqwest Client。
fn build_client(pool: PoolConfig, timeout: TimeoutConfig) -> Client {
    let mut b = Client::builder()
        .connect_timeout(Duration::from_millis(timeout.connect_timeout_ms))
        .pool_max_idle_per_host(pool.max_idle_per_host)
        .pool_idle_timeout(Some(Duration::from_secs(pool.idle_timeout_secs)));
    if pool.keep_alive {
        b = b.tcp_keepalive(Some(Duration::from_secs(pool.keep_alive_interval_secs)));
    }
    if timeout.response_timeout_ms > 0 {
        b = b.timeout(Duration::from_millis(timeout.response_timeout_ms));
    }
    b.build().expect("shared reqwest Client build failed")
}

// ════════════════════════════════════════════════════════════════════════════
// 请求 / 响应抽象（纯数据，不依赖 reqwest）
// ════════════════════════════════════════════════════════════════════════════

/// HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    /// 方法名（小写），用于日志。
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Delete => "delete",
            HttpMethod::Patch => "patch",
        }
    }
}

/// HTTP 请求体（纯数据）。
#[derive(Debug, Clone)]
pub enum HttpBody {
    /// JSON 请求体（自动设 `Content-Type: application/json`）。
    Json(serde_json::Value),
    /// 原始字节，带 content-type（multipart / 二进制上传）。
    /// `MultipartForm::finish()` 返回的 `(Vec<u8>, String)` 直接归入此变体。
    Bytes(Vec<u8>, String),
    /// 无请求体。
    Empty,
}

/// Per-request timeout limits (milliseconds). `None` disables a limit.
///
/// This is the HTTP-layer view of [`TimeoutConfiguration`] (from
/// `CallOptions.timeout`). The HTTP layer enforces all three:
/// - `total_ms` — covers the whole call: connect + response (+ retries and,
///   for streaming, the entire stream).
/// - `first_chunk_ms` — streaming only: time waiting for the first chunk.
/// - `chunk_ms` — streaming only: max idle time between chunks.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestTimeout {
    pub total_ms: Option<u64>,
    pub first_chunk_ms: Option<u64>,
    pub chunk_ms: Option<u64>,
}

impl From<TimeoutConfiguration> for RequestTimeout {
    fn from(t: TimeoutConfiguration) -> Self {
        Self {
            total_ms: t.total_ms,
            first_chunk_ms: t.first_chunk_ms,
            chunk_ms: t.chunk_ms,
        }
    }
}

/// HTTP 请求描述（纯数据，不依赖 reqwest）。
///
/// provider 构造此结构后交给 [`send`] / [`send_stream`] 执行。retry 时本
/// 模块按需从此结构重建 `RequestBuilder`——provider 不参与重建。
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: HttpBody,
    /// Optional signal used to cancel the request while it is connecting.
    pub abort_signal: Option<AbortSignal>,
}

/// HTTP 响应（非流式，纯数据）。
///
/// `body` 为完整响应体字节；provider 用 `serde_json::from_slice` 等解析。
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}

/// HTTP 流式响应。
///
/// `body` 为字节流（`BoxStream<Result<Bytes, AiMuxError>>`，不依赖 reqwest）。
/// 传给 `SseStream::new` 即可解析 SSE——`SseStream` 是泛型的，能直接接收。
pub struct HttpStreamResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: BoxStream<'static, Result<Bytes, AiMuxError>>,
}

// ════════════════════════════════════════════════════════════════════════════
// 执行（含 retry，内部逻辑）
// ════════════════════════════════════════════════════════════════════════════

/// 发送非流式请求，带 retry。内部用 `shared_client()`（30s 整体超时）。
///
/// retry 策略（RFC-0009 §4.2）：
/// - 网络错误 → `AiMuxError::Http`（可重试）
/// - 429 → 读 `retry-after` / `retry-after-ms` header，`AiMuxError::RateLimited`（可重试）
/// - 5xx → `AiMuxError::ApiCall`（可重试）
/// - 其他 4xx → `parse_provider_error`，**立即返回不重试**
///
/// 退避用 Full Jitter（`delay ∈ [0, base)`），防并发 429 惊群。
pub async fn send(
    request: HttpRequest,
    retry_config: RetryConfig,
    error_structure: &ErrorStructure,
) -> Result<HttpResponse, AiMuxError> {
    auto_init_from_env();
    let client = shared_client();
    let resp = send_with_retry_raw(client, &request, retry_config, error_structure).await?;

    let status = resp.status().as_u16();
    let headers = collect_headers(resp.headers());
    // The response body can be large/slow; keep honoring abort while reading.
    let body = match &request.abort_signal {
        Some(signal) => {
            let bytes = resp.bytes();
            tokio::select! {
                biased;
                _ = signal.cancelled() => return Err(AiMuxError::Aborted),
                b = bytes => b.map_err(|e| AiMuxError::Http(e.to_string()))?,
            }
        }
        None => resp
            .bytes()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))?,
    };

    if body_logging_enabled() {
        tracing::trace!(
            target: "aimux_provider_utils::http",
            body = %redact_body(&String::from_utf8_lossy(&body)),
            "response_body"
        );
    }

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// 发送流式请求，返回字节流。内部用 `shared_streaming_client()`（无整体超时）。
///
/// retry 仅覆盖**建连阶段**——`.send()` 返回 200 后立即返回字节流，流中途
/// 出错不重试（已吐 token 后重试会重复内容，RFC-0009 §5）。
pub async fn send_stream(
    request: HttpRequest,
    retry_config: RetryConfig,
    error_structure: &ErrorStructure,
) -> Result<HttpStreamResponse, AiMuxError> {
    auto_init_from_env();
    let client = shared_streaming_client();
    let resp = send_with_retry_raw(client, &request, retry_config, error_structure).await?;

    let status = resp.status().as_u16();
    let headers = collect_headers(resp.headers());
    tracing::debug!(
        target: "aimux_provider_utils::http",
        status = status,
        "stream_connected"
    );
    let body = ObservedByteStream {
        inner: resp
            .bytes_stream()
            .map(|item| item.map_err(|e| AiMuxError::Http(e.to_string())))
            .boxed(),
        started: false,
        start: Instant::now(),
        chunks: 0u64,
        done: false,
    }
    .boxed();

    Ok(HttpStreamResponse {
        status,
        headers,
        body,
    })
}

/// 流式响应体观测包装：发出「首字节」与「流结束」两个 debug 事件
/// （RFC-0014 §4.2 stream 行）。零成本——事件只在开启时格式化。
struct ObservedByteStream {
    inner: BoxStream<'static, Result<Bytes, AiMuxError>>,
    started: bool,
    start: Instant,
    chunks: u64,
    done: bool,
}

impl Stream for ObservedByteStream {
    type Item = Result<Bytes, AiMuxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        let result = this.inner.as_mut().poll_next(cx);
        match &result {
            Poll::Ready(Some(Ok(_))) => {
                if !this.started {
                    this.started = true;
                    tracing::debug!(
                        target: "aimux_provider_utils::http",
                        ttfb_ms = this.start.elapsed().as_millis() as u64,
                        "stream_first_byte"
                    );
                }
                this.chunks += 1;
            }
            Poll::Ready(Some(Err(_))) | Poll::Ready(None) => {
                this.done = true;
                tracing::debug!(
                    target: "aimux_provider_utils::http",
                    chunks = this.chunks,
                    duration_ms = this.start.elapsed().as_millis() as u64,
                    "stream_end"
                );
            }
            Poll::Pending => {}
        }
        result
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Timed variants — per-call timeout support (RFC-0016 H3)
// ════════════════════════════════════════════════════════════════════════════

/// Send a non-streaming request with an optional per-call total timeout.
///
/// The timeout covers the **entire** call: connect, response body, and any
/// retry backoff. On expiry the call fails with `AiMuxError::Timeout` (no
/// retry is attempted after the deadline).
pub async fn send_timed(
    request: HttpRequest,
    retry_config: RetryConfig,
    error_structure: &ErrorStructure,
    timeout: Option<RequestTimeout>,
) -> Result<HttpResponse, AiMuxError> {
    let timeout = timeout.unwrap_or_default();
    validate_timeout(&timeout)?;

    let fut = send(request, retry_config, error_structure);
    match timeout.total_ms {
        Some(0) => Err(AiMuxError::Timeout("total timeout".to_string())),
        Some(ms) => Ok(tokio::time::timeout(Duration::from_millis(ms), fut)
            .await
            .map_err(|_| AiMuxError::Timeout(format!("total timeout after {ms}ms")))??),
        None => fut.await,
    }
}
/// Send a streaming request with optional per-call timeouts.
///
/// `total_ms` covers connect (+ retries); the returned byte stream is then
/// wrapped so that `first_chunk_ms` / `chunk_ms` / the remaining `total_ms`
/// are enforced on the chunks themselves. Expired limits surface as
/// `AiMuxError::Timeout` items in the stream (after which the stream ends).
pub async fn send_stream_timed(
    request: HttpRequest,
    retry_config: RetryConfig,
    error_structure: &ErrorStructure,
    timeout: Option<RequestTimeout>,
) -> Result<HttpStreamResponse, AiMuxError> {
    let timeout = timeout.unwrap_or_default();
    validate_timeout(&timeout)?;
    let start = Instant::now();

    let resp = match timeout.total_ms {
        Some(0) => return Err(AiMuxError::Timeout("total timeout".to_string())),
        Some(ms) => tokio::time::timeout(
            Duration::from_millis(ms),
            send_stream(request.clone(), retry_config, error_structure),
        )
        .await
        .map_err(|_| AiMuxError::Timeout(format!("total timeout after {ms}ms")))??,
        None => send_stream(request.clone(), retry_config, error_structure).await?,
    };
    // Wrap the body whenever timeouts OR an abort signal are in play: the
    // wrapper is what makes the body phase honor abort, so bypassing it when
    // only an abort signal is set would drop body cancellation (RFC-0016
    // review S1).
    if timeout.total_ms.is_none()
        && timeout.first_chunk_ms.is_none()
        && timeout.chunk_ms.is_none()
        && request.abort_signal.is_none()
    {
        return Ok(resp);
    }

    Ok(HttpStreamResponse {
        status: resp.status,
        headers: resp.headers,
        body: TimeoutBodyStream {
            inner: resp.body,
            start,
            total_ms: timeout.total_ms,
            first_chunk_ms: timeout.first_chunk_ms,
            chunk_ms: timeout.chunk_ms,
            abort_signal: request.abort_signal,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        }
        .boxed(),
    })
}

/// Reject timeout values whose deadline cannot be represented (e.g.
/// `u64::MAX` ms on platforms whose `Instant` range is narrower) — adding
/// such a duration to an `Instant` would panic.
fn validate_timeout(timeout: &RequestTimeout) -> Result<(), AiMuxError> {
    let now = Instant::now();
    for ms in [timeout.total_ms, timeout.first_chunk_ms, timeout.chunk_ms]
        .into_iter()
        .flatten()
    {
        if now.checked_add(Duration::from_millis(ms)).is_none() {
            return Err(AiMuxError::InvalidArgument(format!(
                "timeout value {ms}ms is too large"
            )));
        }
    }
    Ok(())
}

/// Which deadline fired (for error messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeoutKind {
    FirstChunk,
    ChunkIdle,
    Total,
}

impl TimeoutKind {
    fn message(self) -> &'static str {
        match self {
            TimeoutKind::FirstChunk => "first chunk timeout",
            TimeoutKind::ChunkIdle => "chunk idle timeout",
            TimeoutKind::Total => "total timeout",
        }
    }
}

/// Stream wrapper enforcing first-chunk / chunk-idle / total deadlines plus
/// abort-signal cancellation.
///
/// Polling is bounded by the earliest applicable deadline: before the first
/// item, `first_chunk_ms` (from request start) applies; afterwards,
/// `chunk_ms` is a **sliding window** reset on every chunk. `total_ms` caps
/// everything from request start. An aborted signal ends the stream with
/// `AiMuxError::Aborted` (abort wins over deadlines). After an error item
/// the stream yields `None` (fused).
///
/// The pending `tokio::time::Sleep` and the abort waiter are stored in the
/// stream (not locals): dropping a `Sleep` unregisters its timer entry, and
/// the abort waiter is a `Notify` future whose waker must stay registered
/// while the stream is Pending.
struct TimeoutBodyStream {
    inner: BoxStream<'static, Result<Bytes, AiMuxError>>,
    start: Instant,
    total_ms: Option<u64>,
    first_chunk_ms: Option<u64>,
    chunk_ms: Option<u64>,
    abort_signal: Option<AbortSignal>,
    first: bool,
    last_chunk_at: Option<Instant>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    abort_wait: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    done: bool,
}

impl TimeoutBodyStream {
    /// The earliest applicable deadline and what it is, or `None` when no
    /// limit is set.
    ///
    /// `checked_add` guards the chunk-idle case: `last_chunk_at` moves
    /// forward with every chunk, so a value that passed entry validation
    /// (relative to that moment) could still overflow later. On overflow the
    /// limit is treated as total-immediate (the earliest possible deadline).
    fn next_deadline(&self) -> Option<(Instant, TimeoutKind)> {
        let phase = if self.first {
            self.first_chunk_ms.and_then(|ms| {
                self.start
                    .checked_add(Duration::from_millis(ms))
                    .map(|d| (d, TimeoutKind::FirstChunk))
            })
        } else {
            self.chunk_ms.and_then(|ms| {
                self.last_chunk_at
                    .and_then(|t| t.checked_add(Duration::from_millis(ms)))
                    .map(|d| (d, TimeoutKind::ChunkIdle))
            })
        };
        let total = self.total_ms.and_then(|ms| {
            self.start
                .checked_add(Duration::from_millis(ms))
                .map(|d| (d, TimeoutKind::Total))
        });
        match (phase, total) {
            (Some((pd, pk)), Some((td, _))) => {
                if pd <= td {
                    Some((pd, pk))
                } else {
                    Some((td, TimeoutKind::Total))
                }
            }
            (Some(x), None) | (None, Some(x)) => Some(x),
            // (None, None): either nothing applies in the current phase
            // (→ no deadline) or an applicable limit overflowed its
            // representable range (→ treat as already expired).
            (None, None) => {
                let phase_configured = if self.first {
                    self.first_chunk_ms.is_some()
                } else {
                    self.chunk_ms.is_some()
                };
                (phase_configured || self.total_ms.is_some())
                    .then_some((self.start, TimeoutKind::Total))
            }
        }
    }
}

impl Stream for TimeoutBodyStream {
    type Item = Result<Bytes, AiMuxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        if this.done {
            return Poll::Ready(None);
        }

        // Abort fast path — also catches signals aborted before the call.
        if let Some(sig) = &this.abort_signal
            && sig.is_aborted()
        {
            this.done = true;
            this.sleep = None;
            return Poll::Ready(Some(Err(AiMuxError::Aborted)));
        }

        // Lazy-create the event-driven abort waiter so an abort wakes us
        // while Pending (the signal's `Notify` stores the waker).
        if this.abort_wait.is_none()
            && let Some(signal) = this.abort_signal.as_ref()
        {
            this.abort_wait = Some(Box::pin(signal.cancelled()));
        }

        let Some((deadline, kind)) = this.next_deadline() else {
            // No timeout configured: pass through (abort still honored).
            if let Some(wait) = this.abort_wait.as_mut()
                && wait.as_mut().poll(cx).is_ready()
            {
                this.done = true;
                return Poll::Ready(Some(Err(AiMuxError::Aborted)));
            }
            let result = this.inner.as_mut().poll_next(cx);
            match &result {
                Poll::Ready(Some(Ok(_))) => {
                    this.first = false;
                    this.last_chunk_at = Some(Instant::now());
                }
                Poll::Ready(Some(Err(_))) | Poll::Ready(None) => {
                    this.done = true;
                }
                Poll::Pending => {}
            }
            return result;
        };

        if Instant::now() >= deadline {
            this.done = true;
            this.sleep = None;
            return Poll::Ready(Some(Err(AiMuxError::Timeout(kind.message().to_string()))));
        }

        // (Re)create the timer if missing or pointed at a different deadline.
        let needs_reset = match &this.sleep {
            Some(s) => s.deadline() != deadline.into(),
            None => true,
        };
        if needs_reset {
            this.sleep = Some(Box::pin(tokio::time::sleep_until(deadline.into())));
        }

        {
            // Abort wins over any deadline.
            if let Some(wait) = this.abort_wait.as_mut()
                && wait.as_mut().poll(cx).is_ready()
            {
                this.done = true;
                this.sleep = None;
                return Poll::Ready(Some(Err(AiMuxError::Aborted)));
            }
            if let Some(sleep) = this.sleep.as_mut()
                && sleep.as_mut().poll(cx).is_ready()
            {
                this.done = true;
                this.sleep = None;
                return Poll::Ready(Some(Err(AiMuxError::Timeout(kind.message().to_string()))));
            }
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    this.first = false;
                    this.last_chunk_at = Some(Instant::now());
                    // Reset the idle window: the next poll re-creates the
                    // timer with a fresh `now + chunk_ms` deadline.
                    this.sleep = None;
                    Poll::Ready(Some(Ok(bytes)))
                }
                Poll::Ready(Some(Err(e))) => {
                    this.done = true;
                    this.sleep = None;
                    Poll::Ready(Some(Err(e)))
                }
                Poll::Ready(None) => {
                    this.done = true;
                    this.sleep = None;
                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// retry 核心
// ════════════════════════════════════════════════════════════════════════════

/// Read an error-response body honoring abort (RFC-0016 review S2).
///
/// Body reads for 429/5xx/other failures are awaited before the next retry
/// decision; without this, an abort during the read would be ignored until
/// the next attempt.
///
/// The body is **streamed** (chunk-by-chunk) and only the first
/// [`MAX_ERROR_BODY_BYTES`] are retained — the total byte count is still
/// counted for the truncation marker — so a misbehaving provider cannot force
/// unbounded memory growth or flood logs/FFI envelopes with a huge error
/// payload (audit finding, rounds 1–2).
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

async fn read_error_body(
    resp: reqwest::Response,
    request: &HttpRequest,
) -> Result<String, AiMuxError> {
    let read = async {
        let mut stream = resp.bytes_stream();
        let mut collected: Vec<u8> = Vec::new();
        let mut total: usize = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                // The connection died mid-read: surface what we have.
                Err(_) => break,
            };
            total += chunk.len();
            if collected.len() < MAX_ERROR_BODY_BYTES {
                let take = (MAX_ERROR_BODY_BYTES - collected.len()).min(chunk.len());
                collected.extend_from_slice(&chunk[..take]);
            }
        }
        let mut s = String::from_utf8_lossy(&collected).into_owned();
        // Lossy decoding can expand invalid bytes (each became U+FFFD, 3
        // bytes), so gate the cap on the *decoded* length, not just `total`.
        if s.len() > MAX_ERROR_BODY_BYTES {
            // Truncate on a UTF-8 char boundary so the marker never splits a
            // multi-byte character. `0` is always a char boundary, so the
            // loop terminates.
            let mut cut = s.len().min(MAX_ERROR_BODY_BYTES);
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
            s.push_str(&format!("…(truncated, full error body {total} bytes)"));
        }
        s
    };
    match &request.abort_signal {
        Some(signal) => {
            let text = read;
            tokio::select! {
                biased;
                _ = signal.cancelled() => Err(AiMuxError::Aborted),
                t = text => Ok(t),
            }
        }
        None => Ok(read.await),
    }
}

/// retry 核心：反复 `.send()` 直到拿到 2xx 响应或耗尽重试。
///
/// 这是 http 层内部函数——`reqwest::Response` 不外泄。每次重试从 `&request`
/// 重建 `RequestBuilder`（HttpRequest 是纯数据，可重复读）。
async fn send_with_retry_raw(
    client: &Client,
    request: &HttpRequest,
    retry_config: RetryConfig,
    error_structure: &ErrorStructure,
) -> Result<reqwest::Response, AiMuxError> {
    let span = tracing::debug_span!(
        target: "aimux_provider_utils::http",
        "http_request",
        method = %request.method.as_str(),
        host = %request_host(&request.url),
        attempt = 0u64,
    );
    let _enter = span.enter();

    let mut last_error = AiMuxError::Other("no attempts made".to_string());
    let mut last_status: Option<u16> = None;
    let mut exponential_delay_ms = retry_config.initial_delay.as_millis() as i64;

    for attempt in 0..=retry_config.max_retries {
        let attempt_start = Instant::now();
        let resp = send_request(client, request).await;
        let latency_ms = attempt_start.elapsed().as_millis() as u64;

        match resp {
            Ok(resp) => {
                let status_code = resp.status().as_u16();
                last_status = Some(status_code);
                tracing::debug!(
                    target: "aimux_provider_utils::http",
                    status = status_code,
                    latency_ms = latency_ms,
                    "response"
                );
                if resp.status().is_success() {
                    return Ok(resp);
                }
                // 429: 读 retry-after headers（在消费 body 之前）。
                if status_code == 429 {
                    let hint = parse_retry_after(
                        resp.headers()
                            .get("retry-after-ms")
                            .and_then(|v| v.to_str().ok()),
                        resp.headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok()),
                        SystemTime::now(),
                    );
                    // Keep the provider's rate-limit body (e.g. "quota
                    // exceeded" vs "too many requests") so consumers can tell
                    // the two apart (issue M6).
                    let body = read_error_body(resp, request).await?;
                    last_error = AiMuxError::RateLimited {
                        retry_after_ms: hint.unwrap_or(1000).max(0) as u64,
                        message: format!("HTTP {status_code}: {}", body),
                    };
                } else if resp.status().is_server_error() {
                    // 5xx: 可重试。
                    let body = read_error_body(resp, request).await?;
                    last_error = AiMuxError::ApiCall(format!("HTTP {}: {}", status_code, body));
                } else {
                    // 非 4xx 非 5xx：不可重试，立即返回。
                    let body = read_error_body(resp, request).await?;
                    return Err(parse_provider_error(status_code, &body, error_structure));
                }
            }
            Err(e) => {
                last_error = e;
            }
        }

        if !last_error.is_retryable() || attempt == retry_config.max_retries {
            // 用户主动取消不算故障，不落 error 日志（RFC-0014 §4.2 failed 行）。
            if !matches!(last_error, AiMuxError::Aborted) {
                tracing::error!(
                    target: "aimux_provider_utils::http",
                    status = last_status.unwrap_or(0),
                    attempts = attempt + 1,
                    reason = %error_variant(&last_error),
                    "failed"
                );
            }
            return Err(last_error);
        }

        let hint = last_error.retry_after_hint();
        let delay_ms = {
            let mut rng = rand::thread_rng();
            get_retry_delay_ms_with_jitter(hint, exponential_delay_ms, &mut rng)
        };
        tracing::warn!(
            target: "aimux_provider_utils::http",
            attempt = attempt + 1,
            max_retries = retry_config.max_retries,
            status = last_status.unwrap_or(0),
            delay_ms = delay_ms.max(0) as u64,
            reason = %error_variant(&last_error),
            "retry"
        );
        span.record("attempt", attempt + 1);
        // Backoff must also be abortable: aborting during a long backoff
        // window (e.g. a large Retry-After) must not be delayed until the
        // next attempt.
        let delay = Duration::from_millis(delay_ms.max(0) as u64);
        match &request.abort_signal {
            Some(signal) => {
                tokio::select! {
                    biased;
                    _ = signal.cancelled() => return Err(AiMuxError::Aborted),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            None => tokio::time::sleep(delay).await,
        }
        exponential_delay_ms =
            exponential_delay_ms.saturating_mul(retry_config.backoff_factor as i64);
    }

    Err(last_error)
}

/// Send one HTTP attempt, cancelling the in-flight connection if requested.
async fn send_request(
    client: &Client,
    request: &HttpRequest,
) -> Result<reqwest::Response, AiMuxError> {
    tracing::debug!(
        target: "aimux_provider_utils::http",
        method = %request.method.as_str(),
        url = %request_url_no_query(&request.url),
        body_size = request_body_size(request),
        header_count = request.headers.len(),
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

    if let Some(signal) = &request.abort_signal {
        if signal.is_aborted() {
            return Err(AiMuxError::Aborted);
        }

        let response = build_request_builder(client, request)?.send();
        tokio::select! {
            biased;
            _ = signal.cancelled() => Err(AiMuxError::Aborted),
            result = response => result.map_err(|e| AiMuxError::Http(e.to_string())),
        }
    } else {
        build_request_builder(client, request)?
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()))
    }
}

/// 把纯数据的 [`HttpRequest`] 转成 `reqwest::RequestBuilder`。
///
/// 每次重试调用一次。`request` 以引用传入，不消费——支持重试重建。
///
/// 若任一 header 的 name/value 无法转成合法的 reqwest header（含非法字节），
/// 返回 [`AiMuxError::InvalidArgument`]——这通常意味着鉴权 header 等关键
/// header 被丢弃，调用者必须感知，而不是静默跳过。
fn build_request_builder(
    client: &Client,
    request: &HttpRequest,
) -> Result<reqwest::RequestBuilder, AiMuxError> {
    let mut builder = match request.method {
        HttpMethod::Get => client.get(&request.url),
        HttpMethod::Post => client.post(&request.url),
        HttpMethod::Put => client.put(&request.url),
        HttpMethod::Delete => client.delete(&request.url),
        HttpMethod::Patch => client.patch(&request.url),
    };
    for (name, value) in &request.headers {
        let header_name = reqwest::header::HeaderName::try_from(name)
            .map_err(|_| AiMuxError::InvalidArgument(format!("invalid header name: {name}")))?;
        let header_value = reqwest::header::HeaderValue::try_from(value).map_err(|_| {
            AiMuxError::InvalidArgument(format!("invalid header value for {name}: {value}"))
        })?;
        builder = builder.header(header_name, header_value);
    }
    let builder = match &request.body {
        HttpBody::Json(value) => builder.json(value),
        HttpBody::Bytes(bytes, content_type) => builder
            .header("Content-Type", content_type)
            .body(bytes.clone()),
        HttpBody::Empty => builder,
    };
    Ok(builder)
}

/// 从 `reqwest::header::HeaderMap` 提取 `HashMap<String, String>`。
fn collect_headers(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// 日志辅助（RFC-0014 §4.2/§4.3：URL 去 query、header 值永不落日志）
// ════════════════════════════════════════════════════════════════════════════

/// 提取 host（不含 query/path）。解析失败回退 "unknown"。
fn request_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// URL 去掉 query string（query 可能含 key，RFC-0014 §4.3）。
fn request_url_no_query(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.to_string()
        }
        Err(_) => url.split('?').next().unwrap_or(url).to_owned(),
    }
}

/// 请求体大小（字节）。JSON 体序列化一次求长度——只在 debug 事件启用时求值。
fn request_body_size(request: &HttpRequest) -> u64 {
    match &request.body {
        HttpBody::Json(value) => serde_json::to_vec(value)
            .map(|b| b.len() as u64)
            .unwrap_or(0),
        HttpBody::Bytes(bytes, _) => bytes.len() as u64,
        HttpBody::Empty => 0,
    }
}

/// `AiMuxError` 变体的短名（日志 reason 字段用）。
fn error_variant(e: &AiMuxError) -> &'static str {
    match e {
        AiMuxError::Provider(_) => "provider",
        AiMuxError::Http(_) => "http",
        AiMuxError::Json(_) => "json",
        AiMuxError::Stream(_) => "stream",
        AiMuxError::Tool(_) => "tool",
        AiMuxError::InvalidArgument(_) => "invalid_argument",
        AiMuxError::InvalidPrompt(_) => "invalid_prompt",
        AiMuxError::RateLimited { .. } => "rate_limited",
        AiMuxError::Auth(_) => "auth",
        AiMuxError::TokenExpired(_) => "token_expired",
        AiMuxError::ModelNotFound(_) => "model_not_found",
        AiMuxError::Unsupported(_) => "unsupported",
        AiMuxError::NoSuchModel(_) => "no_such_model",
        AiMuxError::UnknownProvider(_) => "unknown_provider",
        AiMuxError::ApiCall(_) => "api_call",
        AiMuxError::Timeout(_) => "timeout",
        AiMuxError::Aborted => "aborted",
        AiMuxError::Other(_) => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_client_is_stable_handle() {
        let a = shared_client();
        let b = shared_client();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn shared_streaming_client_is_stable_handle() {
        let a = shared_streaming_client();
        let b = shared_streaming_client();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn shared_and_streaming_are_distinct() {
        assert!(!std::ptr::eq(shared_client(), shared_streaming_client()));
    }

    #[test]
    fn streaming_config_has_no_response_timeout() {
        let t = TimeoutConfig::streaming();
        assert_eq!(t.response_timeout_ms, 0);
        assert_ne!(t.connect_timeout_ms, 0);
    }

    #[test]
    fn default_config_has_response_timeout() {
        let t = TimeoutConfig::default();
        assert_ne!(t.response_timeout_ms, 0);
    }

    #[test]
    fn http_request_is_clone_for_retry() {
        // HttpRequest 必须可 Clone —— retry 时从 &request 重建 RequestBuilder。
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: "https://example.com".to_string(),
            headers: vec![("Authorization".to_string(), "Bearer x".to_string())],
            body: HttpBody::Json(serde_json::json!({"q": "hi"})),

            abort_signal: None,
        };
        let _clone = req.clone();
        assert_eq!(req.method, HttpMethod::Post);
    }

    #[test]
    fn build_request_builder_rejects_invalid_header_name() {
        // A header name containing a space/CR is not a valid token and must be
        // rejected rather than silently dropped (the header could be an auth
        // header, which the caller must know about).
        let client = Client::new();
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: "https://example.com".to_string(),
            headers: vec![("Invalid Header\r\n".to_string(), "secret".to_string())],
            body: HttpBody::Empty,

            abort_signal: None,
        };
        let err = build_request_builder(&client, &req).unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "got {err:?}");
        assert!(err.to_string().contains("invalid header name"));
    }

    #[test]
    fn build_request_builder_rejects_invalid_header_value() {
        // A header value containing a raw CR is invalid and must be rejected.
        let client = Client::new();
        let req = HttpRequest {
            method: HttpMethod::Get,
            url: "https://example.com".to_string(),
            headers: vec![(
                "Authorization".to_string(),
                "Bearer evil\r\nX-Injected: yes".to_string(),
            )],
            body: HttpBody::Empty,

            abort_signal: None,
        };
        let err = build_request_builder(&client, &req).unwrap_err();
        assert!(matches!(err, AiMuxError::InvalidArgument(_)), "got {err:?}");
        assert!(err.to_string().contains("invalid header value"));
    }

    #[test]
    fn build_request_builder_accepts_valid_headers() {
        let client = Client::new();
        let req = HttpRequest {
            method: HttpMethod::Post,
            url: "https://example.com".to_string(),
            headers: vec![
                ("Authorization".to_string(), "Bearer token".to_string()),
                ("X-Custom".to_string(), "value".to_string()),
            ],
            body: HttpBody::Json(serde_json::json!({"q": "hi"})),

            abort_signal: None,
        };
        assert!(build_request_builder(&client, &req).is_ok());
    }

    #[tokio::test]
    async fn timeout_stream_yields_after_first_chunk_deadline() {
        // Inner stream never produces anything → first-chunk deadline fires.
        let inner = futures::stream::pending::<Result<Bytes, AiMuxError>>().boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: Some(200),
            chunk_ms: None,
            abort_signal: None,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };
        let item = tokio::time::timeout(Duration::from_secs(5), timed.next())
            .await
            .expect("first-chunk deadline must fire within 5s")
            .expect("stream must yield an error");
        assert!(matches!(item, Err(AiMuxError::Timeout(_))));
        assert!(
            item.unwrap_err()
                .to_string()
                .contains("first chunk timeout")
        );
    }

    #[tokio::test]
    async fn timeout_stream_enforces_chunk_idle_deadline() {
        // First chunk arrives immediately; the second never comes → the
        // chunk-idle deadline fires on the second poll.
        let inner = futures::stream::iter(vec![Ok(Bytes::from("first"))])
            .chain(futures::stream::pending::<Result<Bytes, AiMuxError>>())
            .boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: None,
            chunk_ms: Some(200),
            abort_signal: None,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };

        let first = timed.next().await.expect("first chunk").unwrap();
        assert_eq!(&first[..], b"first");

        let second = tokio::time::timeout(Duration::from_secs(5), timed.next())
            .await
            .expect("chunk idle timeout must fire within 5s")
            .expect("stream must yield an error");
        assert!(matches!(second, Err(AiMuxError::Timeout(_))));
        assert!(
            second
                .unwrap_err()
                .to_string()
                .contains("chunk idle timeout")
        );
    }

    #[tokio::test]
    async fn timeout_stream_early_chunks_pass_within_budget() {
        let inner = futures::stream::iter(vec![Ok(Bytes::from("a")), Ok(Bytes::from("b"))]).boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: Some(1000),
            chunk_ms: Some(1000),
            abort_signal: None,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };
        let a = timed.next().await.unwrap().unwrap();
        let b = timed.next().await.unwrap().unwrap();
        assert_eq!(&a[..], b"a");
        assert_eq!(&b[..], b"b");
        assert!(timed.next().await.is_none());
    }

    #[tokio::test]
    async fn timeout_stream_is_fused_after_timeout() {
        // After a timeout error item the stream must yield None, not an
        // endless stream of errors (RFC-0016 review P0).
        let inner = futures::stream::pending::<Result<Bytes, AiMuxError>>().boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: Some(100),
            chunk_ms: None,
            abort_signal: None,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };
        let first = tokio::time::timeout(Duration::from_secs(5), timed.next())
            .await
            .expect("must fire")
            .expect("must be an error");
        assert!(matches!(first, Err(AiMuxError::Timeout(_))));
        let second = tokio::time::timeout(Duration::from_millis(500), timed.next())
            .await
            .expect("stream must end right after the timeout error");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn abort_wakes_pending_timeout_stream() {
        // Abort while the inner stream is Pending must end the stream
        // promptly (event-driven, no 50ms polling). No deadline configured —
        // this is the "abort without timeout" streaming path (RFC-0016
        // review S1): the wrapper must still honor the signal.
        let signal = AbortSignal::new();
        let inner = futures::stream::pending::<Result<Bytes, AiMuxError>>().boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: None,
            chunk_ms: None,
            abort_signal: Some(signal.clone()),
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };

        let handle = tokio::spawn(async move {
            let item = timed.next().await;
            (item, timed)
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal.abort();

        let (item, mut timed) = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("abort must wake the pending stream within 2s")
            .expect("task must finish cleanly");
        let item = item.expect("stream must yield an item");
        assert!(matches!(item, Err(AiMuxError::Aborted)), "got {item:?}");
        // Fused after abort.
        let next = timed.next().await;
        assert!(next.is_none(), "stream must be fused after abort");
    }

    #[tokio::test]
    async fn abort_before_first_poll_fails_fast() {
        let signal = AbortSignal::new();
        signal.abort();
        let inner = futures::stream::pending::<Result<Bytes, AiMuxError>>().boxed();
        let mut timed = TimeoutBodyStream {
            inner,
            start: Instant::now(),
            total_ms: None,
            first_chunk_ms: None,
            chunk_ms: None,
            abort_signal: Some(signal),
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };
        let item = timed.next().await.expect("must yield an error");
        assert!(matches!(item, Err(AiMuxError::Aborted)));
        assert!(
            timed.next().await.is_none(),
            "stream must be fused after abort"
        );
    }

    #[test]
    fn validate_timeout_rejects_overflowing_values() {
        // Platform-dependent: on platforms whose `Instant` range is narrower
        // (e.g. Unix), huge values must be rejected with InvalidArgument and
        // must never panic; on wider ranges (Windows) they are accepted.
        let now = Instant::now();
        let huge = RequestTimeout {
            total_ms: Some(u64::MAX),
            ..Default::default()
        };
        match now.checked_add(Duration::from_millis(u64::MAX)) {
            None => {
                let err = validate_timeout(&huge).unwrap_err();
                assert!(matches!(err, AiMuxError::InvalidArgument(_)));
            }
            Some(_) => {
                assert!(validate_timeout(&huge).is_ok());
            }
        }
        assert!(validate_timeout(&RequestTimeout::default()).is_ok());
    }

    #[test]
    fn next_deadline_kind_selection() {
        let now = Instant::now();
        let base = TimeoutBodyStream {
            inner: futures::stream::pending().boxed(),
            start: now,
            total_ms: Some(10_000),
            first_chunk_ms: Some(1_000),
            chunk_ms: None,
            abort_signal: None,
            first: true,
            last_chunk_at: None,
            sleep: None,
            abort_wait: None,
            done: false,
        };
        let (d, k) = base.next_deadline().unwrap();
        assert_eq!(k, TimeoutKind::FirstChunk);
        assert_eq!(d, now + Duration::from_millis(1_000));

        // Total wins when it is earlier.
        let s2 = TimeoutBodyStream {
            total_ms: Some(500),
            first_chunk_ms: Some(1_000),
            ..base
        };
        let (d2, k2) = s2.next_deadline().unwrap();
        assert_eq!(k2, TimeoutKind::Total);
        assert_eq!(d2, now + Duration::from_millis(500));
    }
}
