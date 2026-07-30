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
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Client;

use aimux_core::AiMuxError;

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
    SHARED_STREAMING
        .get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::streaming()))
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
    let client = shared_client();
    let resp = send_with_retry_raw(client, &request, retry_config, error_structure).await?;

    let status = resp.status().as_u16();
    let headers = collect_headers(resp.headers());
    let body = resp.bytes().await.map_err(|e| AiMuxError::Http(e.to_string()))?;

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
    let client = shared_streaming_client();
    let resp = send_with_retry_raw(client, &request, retry_config, error_structure).await?;

    let status = resp.status().as_u16();
    let headers = collect_headers(resp.headers());
    let body = resp
        .bytes_stream()
        .map(|item| item.map_err(|e| AiMuxError::Http(e.to_string())))
        .boxed();

    Ok(HttpStreamResponse {
        status,
        headers,
        body,
    })
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
    let mut last_error = AiMuxError::Other("no attempts made".to_string());
    let mut exponential_delay_ms = retry_config.initial_delay.as_millis() as i64;

    for attempt in 0..=retry_config.max_retries {
        let resp = build_request_builder(client, request)
            .send()
            .await
            .map_err(|e| AiMuxError::Http(e.to_string()));

        match resp {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status_code = resp.status().as_u16();
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
                    let _ = resp.text().await; // 消费 body
                    last_error = AiMuxError::RateLimited {
                        retry_after_ms: hint.unwrap_or(1000).max(0) as u64,
                    };
                } else if resp.status().is_server_error() {
                    // 5xx: 可重试。
                    let body = resp.text().await.unwrap_or_default();
                    last_error =
                        AiMuxError::ApiCall(format!("HTTP {}: {}", status_code, body));
                } else {
                    // 非 4xx 非 5xx：不可重试，立即返回。
                    let body = resp.text().await.unwrap_or_default();
                    return Err(parse_provider_error(status_code, &body, error_structure));
                }
            }
            Err(e) => {
                last_error = e;
            }
        }

        if !last_error.is_retryable() || attempt == retry_config.max_retries {
            return Err(last_error);
        }

        let hint = last_error.retry_after_hint();
        let delay_ms = {
            let mut rng = rand::thread_rng();
            get_retry_delay_ms_with_jitter(hint, exponential_delay_ms, &mut rng)
        };
        tokio::time::sleep(Duration::from_millis(delay_ms.max(0) as u64)).await;
        exponential_delay_ms =
            exponential_delay_ms.saturating_mul(retry_config.backoff_factor as i64);
    }

    Err(last_error)
}

/// 把纯数据的 [`HttpRequest`] 转成 `reqwest::RequestBuilder`。
///
/// 每次重试调用一次。`request` 以引用传入，不消费——支持重试重建。
fn build_request_builder(client: &Client, request: &HttpRequest) -> reqwest::RequestBuilder {
    let mut builder = match request.method {
        HttpMethod::Get => client.get(&request.url),
        HttpMethod::Post => client.post(&request.url),
        HttpMethod::Put => client.put(&request.url),
        HttpMethod::Delete => client.delete(&request.url),
        HttpMethod::Patch => client.patch(&request.url),
    };
    for (name, value) in &request.headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::try_from(name),
            reqwest::header::HeaderValue::try_from(value),
        ) {
            builder = builder.header(name, value);
        }
    }
    match &request.body {
        HttpBody::Json(value) => builder.json(value),
        HttpBody::Bytes(bytes, content_type) => builder
            .header("Content-Type", content_type)
            .body(bytes.clone()),
        HttpBody::Empty => builder,
    }
}

/// 从 `reqwest::header::HeaderMap` 提取 `HashMap<String, String>`。
fn collect_headers(headers: &reqwest::header::HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect()
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
        assert!(!std::ptr::eq(
            shared_client(),
            shared_streaming_client()
        ));
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
        };
        let _clone = req.clone();
        assert_eq!(req.method, HttpMethod::Post);
    }
}
