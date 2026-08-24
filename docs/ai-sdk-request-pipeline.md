# RFC-0031: 对齐 AI SDK 的请求管线

> **Status**: IMPLEMENTED（2026-08-20；jitter 与首条 SSE error 两处有意差异见 §3）
>
> **Date**: 2026-08-19
>
> **Reference baseline**: Vercel AI SDK `63db193`
>
> **Scope**: operation retry、timeout/abort、POST/GET API helpers、response handlers、`ApiCallError` 与 `RetryError`

---

## 1. 摘要

aimux 当前把 HTTP、retry、timeout、body 读取和错误解析集中在：

```text
send
send_timed
send_stream_timed
send_with_retry_raw
ErrorStructure
parse_provider_error
```

结果是 retry 只包围 HTTP exchange，无法包围完整的 Provider operation；成功响应解析、
Provider 业务错误和 semantic stream timeout 也处在错误的层级。

本 RFC 不重新发明请求框架，直接采用 AI SDK 的现有分层和函数名称：

```text
Core user operation
  └─ prepare_retries
       └─ retry_with_exponential_backoff_respecting_retry_headers
            └─ model.do_generate / do_stream / do_embed / ...
                 └─ Provider
                      └─ post_json_to_api / post_form_data_to_api /
                         post_to_api / get_from_api
                           ├─ failed_response_handler
                           └─ successful_response_handler
```

核心不变量：

> 一个用户级 model operation 只有一个通用 retry owner；一次 Provider Utils API helper
> 调用只执行一次 fetch attempt，不做 retry。自动 redirect 属于这次 fetch attempt，
> recording 仍把整条 redirect chain 记为一个 logical exchange。

---

## 2. 规范来源

以下 AI SDK 文件是本 RFC 对应部分的规范来源：

| 能力 | AI SDK 文件 |
|---|---|
| Core retry 包围 `model.doGenerate` | `packages/ai/src/generate-text/generate-text.ts` |
| stream 建立阶段 retry 与 semantic timeout | `packages/ai/src/generate-text/stream-text.ts` |
| `prepareRetries` | `packages/ai/src/util/prepare-retries.ts` |
| APICall-aware retry | `packages/ai/src/util/retry-with-exponential-backoff.ts` |
| 通用 retry primitive | `packages/provider-utils/src/retry-with-exponential-backoff.ts` |
| `RetryError` | `packages/ai/src/util/retry-error.ts` |
| POST helpers | `packages/provider-utils/src/post-to-api.ts` |
| GET helper | `packages/provider-utils/src/get-from-api.ts` |
| response handlers | `packages/provider-utils/src/response-handler.ts` |
| fetch error normalization | `packages/provider-utils/src/handle-fetch-error.ts` |
| `APICallError` | `packages/provider/src/errors/api-call-error.ts` |
| timeout configuration | `packages/ai/src/prompt/request-options.ts` |
| abort/timeout merge | `packages/ai/src/util/merge-abort-signals.ts`、`set-abort-timeout.ts` |

除 §3 明列的 Rust/Aimux 差异外，实现应遵循这些文件的函数边界和行为，不得用新的
`send_*`、`execute_api`、`call_to_api` 等公开抽象替代。

---

## 3. Aimux 仅有的适配差异

| 差异 | 决策 |
|---|---|
| crate 依赖方向 | retry primitive 和 Core wrapper 都放进 `aimux-core::retry`，避免 `aimux-core` 反向依赖 `aimux-provider-utils`；函数边界和算法不变 |
| Rust 错误类型 | AI SDK `RetryError.errors: unknown[]` 对应 `Vec<AiMuxError>`；这是同一错误历史的强类型表达 |
| enum 尺寸 | `AiMuxError::ApiCall(Box<ApiCallError>)`；`Box` 只解决 Rust enum 尺寸，不改变序列化或 binding 语义 |
| `cause` | 本轮不加入；AI SDK 用 `cause` 保存的底层错误追加到 message（`{upstream message}: {source}`），或保存在 data/response body，不得静默丢失 |
| Gateway | 不增加 `GatewayError`；aimux 没有独立 AI Gateway 错误体系 |
| 既有错误字段 | 保留 Aimux 的 `provider_code`；删除旧路径派生的 `request_id` / `retry_after_ms`，retry hint 的唯一事实源是 `response_headers` |
| error context 安全 | AI SDK 传原始 request context；Aimux 写入 public error 前使用统一白名单/脱敏/大小限制，binary 与大型 data URL 只保留摘要 |
| 错误 body 上限 | AI SDK 全量读取错误 body（仅受 2 GiB 防 OOM 上限，超限直接抛 `DownloadError` 替换原错误）；Aimux 的 `ApiCallError` 会跨 FFI 序列化并写入 recording，因此错误 body 采用 best-effort 截断读：public `response_body` 上限 64 KiB（lossy 解码后按字符边界执行，带 `…(truncated)` 标记），解析上限 1 MiB 保证超大但合法的错误 JSON 仍能进 provider mapper，读取中途连接死亡保留已收到的部分 |
| provider 默认 retry | 保留既有 `RetryConfig` 源码与行为兼容；Core 读取 model config，per-call 只覆盖 `max_retries` |
| jitter | AI SDK 默认不抖动但留 `getDelayInMs` 注入点；Aimux 用同一注入点默认注入 RFC-0009 Full Jitter，只作用于 exponential delay，server hint 精确遵守（§6.5） |
| 第一条 SSE error | AI SDK OpenAI provider 会扫描到首个 semantic output；Aimux 保留 RFC-0016 的更窄 first-event peek，使立即到达的 error 成为 retry 边界内的 attempt 失败（§8.3） |
| 默认超时 | 与 AI SDK 一致无默认 `total_ms`；Aimux 有意保留非流式 exchange 的 30s whole-response 上限，但从 shared-client 全局配置下移到单次 exchange，流式 exchange 豁免（§5.4） |
| timeout 输入形状 | 保留跨语言已有的 object/struct 形式，不增加 AI SDK 的裸 number 简写；字段语义一致 |
| `AbortSignal` | 保留既有的 `CancellationToken` 薄包装，只表示调用方取消；Rust 可直接 drop future，因此 timeout 由 Core deadline/select 表达，不照搬 JS 的 signal merge（§8.0） |
| SSE framing | AI SDK 的 event-source handler 直接产出解析后的事件；Aimux 复用 `aimux_stream::SseStream` 做同一件事，framing 不留在 Provider |
| tool timeout | aimux 目前不执行用户 tool，本 RFC 不增加 `tool_ms`/per-tool timeout |
| observability | 每次 exchange 继续走 Aimux recording/tracing；这不改变 helper 的单次请求语义 |
| FFI 运输 | 不由本 RFC 重新设计；C 错误 owner 与 nested error getter 由 RFC-0030 单独规定 |

这些是适配，不是另一套架构。

---

## 4. 分层

### 4.1 `aimux-core`

负责：

- 用户级 `generate_text` / `stream_text` / multimodal operations；
- `prepare_retries` 和两层 exponential-backoff 函数；
- `RetryError`；
- 用户 abort、total/step/stream timeout；
- semantic stream timeout。

固定模块：

```text
aimux-core/src/abort_signal.rs   AbortSignal（实现从 shared.rs 迁出；旧 `shared::AbortSignal` 路径 re-export 保留）
aimux-core/src/retry.rs
aimux-core/src/timeout.rs
```

### 4.2 `aimux-provider-utils`

负责：

- shared reqwest client、pool、proxy；
- 单次 POST/GET exchange；
- `ResponseHandler` 与标准 handler factories；
- body size limit、header extraction、fetch error normalization；
- HTTP exchange recording。

固定模块：

```text
aimux-provider-utils/src/post_to_api.rs
aimux-provider-utils/src/get_from_api.rs
aimux-provider-utils/src/response_handler.rs
aimux-provider-utils/src/handle_fetch_error.rs
aimux-provider-utils/src/extract_response_headers.rs
aimux-provider-utils/src/read_response_with_size_limit.rs
```

`http.rs` 只保留 client/pool/proxy、HTTP value types 和私有的单次 reqwest primitive。

### 4.3 Provider

负责：

- URL、headers、request body；
- 为每个 HTTP operation 选择 successful/failed response handler；
- Provider-specific schema、业务错误和结果转换；
- SSE 或 custom protocol 到 `StreamPart` 的转换；
- submit/poll/download 等有状态工作流；
- 在现有 monolithic `do_generate` SPI 下，对已获得 job id 后的幂等 poll/download exchange 单独 retry。

Provider 不对完整 model operation 执行通用 exponential retry；上述安全 exchange 是
为避免重新 submit 的有状态例外。

---

## 5. Provider Utils API

### 5.1 固定函数面

| AI SDK | Aimux Rust |
|---|---|
| `postJsonToApi` | `post_json_to_api` |
| `postFormDataToApi` | `post_form_data_to_api` |
| `postToApi` | `post_to_api` |
| `getFromApi` | `get_from_api` |
| `ResponseHandler<T>` | `ResponseHandler<T>` |
| `createJsonErrorResponseHandler` | `create_json_error_response_handler` |
| `createJsonResponseHandler` | `create_json_response_handler` |
| `createEventSourceResponseHandler` | `create_event_source_response_handler` |
| `createBinaryResponseHandler` | `create_binary_response_handler` |
| `createStatusCodeErrorResponseHandler` | `create_status_code_error_response_handler` |
| `handleFetchError` | `handle_fetch_error` |
| `extractResponseHeaders` | `extract_response_headers` |
| `readResponseWithSizeLimit` | `read_response_with_size_limit` |

### 5.2 `ResponseHandler`

语义直接对应 AI SDK：

```rust
pub struct ResponseHandlerInput {
    pub url: String,
    pub request_body_values: serde_json::Value,
    pub response: reqwest::Response,
    /// Rust 适配：fetch 的 body 天然绑定 AbortSignal，reqwest 的不绑定。handler 里
    /// 所有 body 读取（size-limited read、SSE 分帧）都必须 `select!` 这个信号。
    pub abort_signal: Option<AbortSignal>,
}

pub struct ResponseHandlerOutput<T> {
    pub value: T,
    pub raw_value: Option<serde_json::Value>,
    pub response_headers: Option<std::collections::HashMap<String, String>>,
}
```

具体实现可以使用泛型 async closure，不要求 `Arc<dyn ResponseHandler<_>>`。

每个 API call 同时传入：

```text
successful_response_handler
failed_response_handler
```

分发固定为：

```text
transport failure → handle_fetch_error
2xx               → successful_response_handler
non-2xx           → failed_response_handler
```

2xx 解析失败由 successful handler 产生 `ApiCallError`，不得再调用 failed handler。
handler 返回的 ApiCall/Timeout/Aborted 原样透传；其他 handler failure 按 AI SDK 的
`Failed to process successful/error response` 规则包装。由于本轮没有 `cause`，原始信息
必须保存在 message、data 或受限长度的 response body 中。

标准 handler factory 直接移植 AI SDK `response-handler.ts` 的同名行为。Provider-specific
协议使用 custom `ResponseHandler`；例如 Bedrock binary event-stream 不是 SSE，不得套
`create_event_source_response_handler`。

各 factory 的固定签名（AI SDK 的 `schema` 参数在 Rust 里就是类型参数）：

```rust
pub fn create_json_response_handler<T: DeserializeOwned + Send + 'static>()
    -> ResponseHandler<T>;                       // value: T, raw_value: Some(json)
pub fn create_event_source_response_handler<T: DeserializeOwned + Send + 'static>()
    -> ResponseHandler<BoxStream<'static, Result<T, AiMuxError>>>;
    // 用 aimux_stream::SseStream 分帧；每个 event 的 `data` 解析成 T；`[DONE]` 跳过；
    // 单条解析失败 yield 一个 Err 项（见 §7.2 stream 行），流不终止，由 provider 决定怎么办
pub fn create_binary_response_handler() -> ResponseHandler<Bytes>;
pub fn create_json_error_response_handler<F>(error_to_message: F) -> ResponseHandler<AiMuxError>
where F: Fn(&serde_json::Value) -> ProviderErrorParts;  // { message, provider_code }
pub fn create_status_code_error_response_handler() -> ResponseHandler<AiMuxError>;
```

aimux 所有 SSE provider 都按 `data` 里的 `type` 字段分派，没有人读 `event:` 名，所以
T-typed handler 覆盖全部现有用法；需要同时接受正常 chunk 和 error chunk 的 provider 用
`#[serde(untagged)] enum Chunk { Ok(..), Error(..) }` 作 T——这正是 AI SDK 里
`z.union([chunkSchema, errorSchema])` 的 Rust 写法。

`create_json_response_handler` 返回 `Bytes` 再让 Provider 自己 `serde_json::from_slice(..)?`
是错误实现：schema 失败会变成没有 URL/status/headers 的 `JsonParse`，违反 §7.2。
Provider 调用 JSON helper 之后不得再出现 `serde_json::from_slice` / `from_str` 解析响应
主体；需要原始 `Value`（例如 `Usage.raw`）时读 `raw_value`。

失败 handler 的归属与 AI SDK 相同：`@ai-sdk/provider-utils` 只提供 factory，
`openaiFailedResponseHandler`、`anthropicFailedResponseHandler` 等常量定义在各 provider
包里。Aimux 同样在 `aimux-providers` 的各 provider 模块内定义
`<provider>_failed_response_handler()`（或同义 `static`），由该 endpoint 的 wire schema
决定。`aimux-provider-utils` 不得提供 `create_openai_error_response_handler`、
`create_json_path_error_response_handler` 这类“换个名字的 `ErrorStructure`”，也不得把
OpenAI 形状的失败 handler 当作所有 Provider 的默认值——fal、ElevenLabs、Cohere 等的错误
体不是 `{error:{message}}`，套 OpenAI handler 会丢 message。

`ErrorStructure`、`DEFAULT_ERROR_STRUCTURE` 和 `parse_provider_error` 全部删除，不增加另一套
全局默认错误 schema。handler 是否复用由具体 endpoint 的 wire schema 决定，不能由
Provider 名称或调用点数量推断。

### 5.3 `post_json_to_api` / `post_form_data_to_api`

两者只负责准备 body/context 后调用 `post_to_api`：

```text
post_json_to_api:
  content = JSON bytes
  values  = structured request body

post_form_data_to_api:
  content = multipart body
  values  = field summary; binary 仅保留类型与长度
```

body 类型由函数签名决定，不在运行时检查：`post_json_to_api` 接 `serde_json::Value`，
`post_form_data_to_api` 接 `MultipartForm`，`post_to_api` 接 `HttpBody`，`get_from_api`
没有 body。"`post_json_to_api` 收到非 JSON body 返回 `InvalidArgument`" 这类守卫是签名
没设计好的补丁，不要出现。其余参数（url、headers、abort_signal、call_id、recording_context）
沿用既有 `HttpRequest` 去掉 method/body 后的那部分。

handler 接口仍接收当前 request context；当它被存入 `ApiCallError` 时必须通过统一
redaction helper。不得依赖每个 Provider 自己记得脱敏。

### 5.4 `post_to_api` / `get_from_api`

它们遵循 AI SDK 的单次 fetch attempt 契约：

- 不接受 `RetryConfig`；
- 不接受 `TimeoutConfiguration`；
- 不执行 backoff；
- 不调用自身或另一个 API helper 进行 retry；
- body read/parse 由选中的 response handler 完成；
- abort signal 只负责中止本次 exchange。

四个 helper 共用一个私有的单次 exchange primitive（`pub(crate)`，名字不进入公开 API）。
该 primitive 是既有 `send_with_retry_raw` 中 **去掉 for 循环、backoff 与 `ErrorStructure`
之后剩下的那部分**，而不是另起一份裸 `client.execute()`：

- 共享 client/pool/proxy 选择；
- abort-aware 的发送（`tokio::select!` on `abort_signal.cancelled()`）；
- transport error → `handle_fetch_error`；
- **recording / tracing**：`record_exchange` / `record_failed_exchange` /
  `record_transport_closed` 和 `tracing::info!` 的 exchange 行原样保留。`HttpRequest.call_id`、
  `recording_context` 不能变成未读字段。每次 helper 调用恰好记录一个 logical exchange；
  redirect chain 仍属于该 exchange，retry 不再出现在这里。`attempt` 与 `exchange_index`
  都由 §10.2 的 `RecordingContext` 提供。

`get_from_api` 的 `request_body_values` 为 `{}`。provider 返回下载 URL 时所需的 SSRF
防护（私网/loopback 拒绝、DNS pinning、redirect 逐跳校验和跨 origin credential 清理）
由独立的 #163 交付；该 PR rebase 到本管线后负责接入对应 Provider call site。

**默认超时的取舍**：Core 与 AI SDK 一样没有默认 `total_ms`，binding 也不得补 operation
deadline；但 Aimux 恢复旧行为，给每个**非流式** helper exchange 保留固定 30s 的
whole-response 上限（connect、headers、完整 body/handler parse）。它不放回 shared client，
因为 shared client 同时服务长生命周期 stream；而是在单次 exchange primitive 周围执行。
超时产生 no-status、retryable `ApiCallError`，因此仍由 Core 决定是否重试完整 operation，
不会伪装成 non-retryable 的 Core `Timeout`。streaming handler 豁免这个 30s guard，只受
connect timeout、caller abort、显式 Core total/step/first/chunk deadline 约束。这是相对 AI SDK
fetch 无默认 exchange timeout 的有意差异，CHANGELOG 必须明确。

### 5.5 `handle_fetch_error`

与 AI SDK 一致：

| 输入 | 输出 |
|---|---|
| Timeout/Aborted | 原样返回 |
| 已是 `ApiCallError` | 原样返回 |
| reqwest DNS/connect/TLS/socket/request transport failure | no-status、retryable `ApiCallError` |
| 其他本地错误 | 原样返回 |

它只标准化错误，不 retry。

---

## 6. Core retry

### 6.1 固定函数面

| AI SDK | Aimux Rust |
|---|---|
| `prepareRetries` | `prepare_retries` |
| `retryWithExponentialBackoffRespectingRetryHeaders` | `retry_with_exponential_backoff_respecting_retry_headers` |
| `retryWithExponentialBackoff` | `retry_with_exponential_backoff` |
| `delay` | `delay` |
| `mergeAbortSignals` | 不逐字移植；Core 的 `tokio::select!` 同时观察 caller signal、deadline 和 operation future（§8.0） |
| `setAbortTimeout` | `timeout::OperationTimeout` 保存 deadline，驱动 future 直接 `sleep_until`（§8.0） |

Core operation 把 per-call override、既有 model `RetryConfig` 和 caller abort 交给同名函数：

```text
prepare_retries(max_retries, retry_config, abort_signal)
```

`max_retries=None` 使用 `retry_config.max_retries`（默认 2）；per-call `Some(n)` 只覆盖
该计数，`initial_delay` 和 `backoff_factor` 保持既有配置。`RetryConfig` 的 canonical
定义移到 `aimux-core::retry`，`aimux-provider-utils::RetryConfig` 和
`aimux_provider_utils::retry::RetryConfig` 都 re-export 同一类型。旧
`retry_config` / `with_retry_config` 保留，不增加第二套 provider config 命名。

两层函数的边界与 AI SDK 相同，**不得合并成一个 APICall-aware 的 primitive**：

```rust
// 通用 primitive：不认识 ApiCallError
pub(crate) async fn retry_with_exponential_backoff<F, Fut, T>(
    op: F,
    max_retries: u32,
    initial_delay_ms: u64,
    backoff_factor: u64,
    abort_signal: Option<&AbortSignal>,
    should_retry: impl FnMut(&AiMuxError) -> bool,
    get_delay_ms: impl FnMut(&AiMuxError, u64 /* exponential */) -> u64,
) -> Result<T, AiMuxError>;

// APICall-aware 包装：只填两个 hook
async fn retry_with_exponential_backoff_respecting_retry_headers<..>(
    op, max_retries, initial_delay_ms, backoff_factor, abort_signal,
) {
    retry_with_exponential_backoff(
        op, max_retries, initial_delay_ms, backoff_factor, abort_signal,
        |e| matches!(e, AiMuxError::ApiCall(d) if d.is_retryable),
        get_retry_delay_ms, // §6.5
    )
}
```

`prepare_retries(max_retries, retry_config, abort_signal)` 返回 `PreparedRetries { max_retries, retry }`
的 Rust 形式是 `PreparedRetries::retry(&self, op)`，与 AI SDK 的 `{ maxRetries, retry }` 一致；
不另起额外的 retry 类型。

### 6.2 Retry 边界

非流式：

```text
retry
  └─ model.do_generate / do_embed / do_rerank / ...
       ├─ HTTP exchange
       ├─ body read
       ├─ successful response parse
       └─ Provider result conversion
```

流式：

```text
retry
  └─ model.do_stream
       ├─ HTTP exchange
       ├─ peek 第一条 SSE 事件（RFC-0016 M3；是 error 就 Err 返回 → 本次 attempt 失败）
       └─ 返回 parsed stream（被 peek 的事件重新接回流首，不丢）
```

stream 返回之后的 SSE error、parse error 或 transport failure 不自动 retry/reconnect。
peek 在返回之前，因此它产生的错误按 §8.3 走正常 attempt 语义。

### 6.3 `RetryError`

```rust
#[serde(rename_all = "camelCase")]
pub enum RetryErrorReason {
    MaxRetriesExceeded,
    ErrorNotRetryable,
}

pub struct RetryError {
    pub reason: RetryErrorReason,
    pub errors: Vec<AiMuxError>,
}

impl RetryError {
    pub fn last_error(&self) -> &AiMuxError {
        self.errors.last().expect("RetryError always contains an error")
    }
}
```

AI SDK 使用 `errors: unknown[]`，因为 enclosing operation 的不同 attempt 可以失败为不同
错误；Aimux 的等价强类型是 `Vec<AiMuxError>`，不是 `Vec<ApiCallError>`。

`last_error` 从 `errors.last()` 派生，不重复序列化。AI SDK public reason union 中的 `abort`
没有 retry primitive 生产路径；Aimux 同样让 Timeout/Aborted 原样返回，不创建 Abort reason。
公开 reason 值固定为 `maxRetriesExceeded` / `errorNotRetryable`；各语言只做惯用命名映射，
不得另造数值 code。

### 6.4 精确行为

默认值与 AI SDK 一致：

```text
max_retries = 2
initial_delay = 2000ms
backoff_factor = 2
```

| 情况 | 返回 |
|---|---|
| `max_retries=0` | 原错误 |
| 第一次 non-retryable | 原错误 |
| Timeout/Aborted | 原错误 |
| retry 后成功 | 成功 |
| retryable errors 耗尽 | `RetryError::MaxRetriesExceeded` |
| retry 后遇到 non-retryable | `RetryError::ErrorNotRetryable`，保存全部 errors |

`max_retries=2` 最多执行三次。`RetryError` 自身不可重试，也不得嵌套。
判定顺序与 AI SDK 相同：先 `try_number > max_retries` → `MaxRetriesExceeded`（**不看最后一个
错误是否 retryable**：`[retryable, retryable, non-retryable]` 在 `max_retries=2` 下是
`MaxRetriesExceeded`），再 `should_retry` → delay 后重试，再 `try_number == 1` → 原错误，
否则 `ErrorNotRetryable`。
`should_retry` 固定为 `AiMuxError::ApiCall(e) && e.is_retryable`；其他 variant 不得自行
加入通用 retry 白名单。

固定 message 与 AI SDK 一致：

```text
Failed after {N} attempts. Last error: {last_error}
Failed after {N} attempts with non-retryable error: '{last_error}'
```

### 6.5 Retry headers

delay 依次读取：

1. `retry-after-ms`；
2. `retry-after` 数字秒；
3. `retry-after` HTTP-date；
4. exponential delay。

server hint 只在 AI SDK 的 reasonable-delay 条件成立时使用：

```text
ms >= 0 && (ms < 60_000 || ms < exponential_delay)
```

事实源是 `ApiCallError.response_headers`，适用于任意 retryable status，不限于 429。
不再复制成 `retry_after_ms`；`response_headers` 是唯一事实源。

**Jitter**：AI SDK 的 primitive 通过 `getDelayInMs` 把 delay 策略交给调用方，默认
`({exponentialBackoffDelay}) => exponentialBackoffDelay`，即默认不抖动但刻意留了注入点。
Aimux 沿用这个注入点，默认策略保留 RFC-0009 的 Full Jitter——但只作用在 exponential delay
上：`get_retry_delay_ms` 选中 server hint 时**原样返回 hint**，不再对 hint 取随机（这是
旧 `get_retry_delay_ms_with_jitter` 把 `Retry-After` 提前的 bug）。

```rust
// 只回答“有没有可用的 server hint”，不回退到 exponential——
// 用 `base != exponential` 反推 hint 是否胜出会在 hint 恰好等于 exponential 时误判。
fn retry_after_hint_ms(error: &AiMuxError, exponential: u64) -> Option<u64>;

match retry_after_hint_ms(error, exponential) {
    Some(hint) => hint,              // 精确遵守，不抖
    None       => rng.gen_range(0..=exponential),   // Full Jitter（RFC-0009）
}
```

RFC-0009 在 jitter 上**不被** supersede；被 supersede 的只是 "retry 在 HTTP 层"。

delay 必须 abort-aware；delay 期间 timeout/abort 直接返回对应错误，不进入 `RetryError`。

---

## 7. 错误模型

### 7.1 `ApiCallError`

字段直接对应 AI SDK，外加现有兼容字段：

```rust
pub struct ApiCallError {
    pub message: String,
    pub url: String,
    pub request_body_values: serde_json::Value,

    pub status_code: Option<u16>,
    pub response_headers: Option<std::collections::HashMap<String, String>>,
    pub response_body: Option<String>,
    pub data: Option<serde_json::Value>,
    pub is_retryable: bool,

    // Aimux extension:
    pub provider_code: Option<String>,
}

pub enum AiMuxError {
    ApiCall(Box<ApiCallError>),
    Retry(RetryError),
    Timeout(String),
    Aborted(String),
    // existing variants...
}
```

`url` 和 `request_body_values` 是 required，与 AI SDK 一致。所有 `ApiCallError` producer
必须从当前 API operation 取得这两个值，不得以 `None` 掩盖缺失的 context。
`ApiCallError` 不再实现 `Default`；invalid endpoint、client construction 等 pre-HTTP
失败必须改成其真实的参数/配置/内部错误类型，不能伪造 API context。

`Box<ApiCallError>` 是必要的 Rust 布局适配：新增 context 后不得删除现有
`AiMuxError <= 128 bytes` guard；serde、TS 和 binding 的外部结构保持不变。
`Retry(RetryError)` 同理：`Vec` 本身 24 字节可以不 box，但若 guard 失败则 box 它而不是放宽
guard。`aimux-core/tests/error_value_golden_test.rs` 的 `error_size_is_pinned` 和
`variant_set_is_exactly_thirteen`（改名为 fourteen 并加入 `Retry` 的 golden 行）必须在
第一个提交里就通过——它们就是本节的验收测试，不是迁移尾声再修的东西。

旧路径与本 RFC 同批删除，因此 `request_id` / `retry_after_ms` 不再属于 `ApiCallError`。
failed handler **不得再从 headers 派生它们**：`response_headers` 是 retry hint 的唯一事实源。

### 7.2 生产规则

| 失败 | 错误 |
|---|---|
| DNS/connect/TLS/socket/body transport | no-status、retryable `ApiCallError` |
| 408/409/429/5xx | 默认 retryable `ApiCallError` |
| 其他 4xx | 默认 non-retryable `ApiCallError` |
| 2xx（非流式）body/JSON/schema failure | observed-status、默认 non-retryable `ApiCallError`（AI SDK: `Invalid JSON response`） |
| peek 到的第一条 SSE error（`do_stream` 返回前，§8.3） | 按其 status/code 构造 `ApiCallError`，retryability 照 HTTP 规则；作为 attempt 失败上抛 |
| stream 内单条 chunk JSON/schema failure | `JsonParse` / `InvalidResponseData` 作为流的一个 `Err` 项（AI SDK: `JSONParseError`/`TypeValidationError` error part）；**不是** `ApiCallError`，也不终止流 |
| stream 内 transport failure | no-status、retryable `ApiCallError` 作为 `Err` 项，随后流结束；Core 不重连（§8.3） |
| 200 Provider business error | Provider 构造 `ApiCallError` 并显式决定 retryable |
| 用户参数 / binding wire 错误 | 参数或 boundary error，不是 `ApiCallError` |

`is_retryable` 的含义是“安全地重新执行 enclosing model operation”，不只是“该 HTTP
错误暂时性”。Provider workflow 可以覆盖 HTTP status 的默认判断。

`AiMuxError::JsonParse` / `InvalidResponseData` 两个 variant 保留（它们是 14-variant binding
契约的一部分，stream chunk 行和 replay/tool-args 解析仍生产它们），但**非流式响应路径不再
生产它们**：`From<serde_json::Error> for AiMuxError` 仅供非响应路径使用，provider 代码里
对响应体的 `?` 转换消失（§5.2）。

### 7.3 Redaction

进入错误前必须脱敏：

- URL query；
- `request_body_values`；
- `response_headers`；
- `data` 中的敏感字段。

复用并整理现有 `aimux_core::recording::is_sensitive_key` 规则；logging、recording 和 error
不得维护三套敏感键表。具体：`aimux-provider-utils/src/logging.rs` 里私有的
`is_sensitive_key`（5 个子串）删除，`redact_value` / `redact_request_values` /
`extract_response_headers` 全部调用 `aimux_core::recording::is_sensitive_key`。目前
header 走 core 表、body 走 logging 表，正是本段禁止的状态。现有 `http.rs::redacted_response_headers` 提取成共享 helper，
不重写第四份 header policy。`request_body_values` 和 `data` 还必须经过共享的深度/大小限制：binary、
base64/data URL 和超长字符串只保留类型、长度或安全摘要。`response_body` 继续受现有
size limit 和 UTF-8 安全截断约束，且不得被默认日志输出。

### 7.4 Timeout 与 Abort

保留简单 public variants，并保存实际 timeout 标签/时长：

```rust
AiMuxError::Timeout(String)
AiMuxError::Aborted(String)
```

这对应 AI SDK 的 timeout/abort 错误；调用方取消使用稳定 message `request aborted`，timeout
由触发的 deadline 直接生成 `{label} timeout of {ms}ms exceeded`。不新增只重复 message 的
`TimeoutErrorData`。
两者均不可 retry，且不得进入 `RetryError.errors`。

---

## 8. Timeout 与 stream

### 8.0 `AbortSignal`

既有 `AbortSignal` 就是 `tokio_util::sync::CancellationToken` 的薄包装，这已经是 Web
`AbortSignal` 在 Rust 里的惯用对应。它只表达**调用方主动取消**；timeout 是 Core 的 deadline，
不是写回 signal 的另一种 reason。完整定义（`aimux-core/src/abort_signal.rs`）：

```rust
#[derive(Debug, Clone, Default)]
pub struct AbortSignal {
    token: CancellationToken,
}

impl AbortSignal {
    pub fn new() -> Self;
    pub fn abort(&self);                                  // token.cancel()
    pub fn is_aborted(&self) -> bool;                     // token.is_cancelled()
    pub fn cancelled(&self) -> impl Future<Output = ()> + Send + 'static; // token.clone().cancelled_owned()
}
```

`aimux-core/src/timeout.rs` 在 operation 开始时记录 total/step deadlines，不 spawn timer task：

```rust
struct OperationTimeout {
    total: Option<TimeoutDeadline>,
    step: Option<TimeoutDeadline>,
}

async fn run(operation, caller: Option<&AbortSignal>, timeout: OperationTimeout) {
    tokio::select! {
        biased;
        _ = caller.cancelled() => Err(AiMuxError::Aborted("request aborted".into())),
        _ = sleep_until(timeout.deadline()) => Err(timeout.deadline().error()),
        result = operation => result,
    }
}
```

AI SDK 的 `mergeAbortSignals` / `setAbortTimeout` 是围绕 `AbortController` 的 JS 写法；
Rust future 可以被 drop，因此不需要先把 timeout 转写成另一棵 cancellation tree：外层
`select!` 赢得 deadline/abort 分支时，operation future（包括当前 HTTP read 或 retry delay）立即
被 drop。stream 返回后由 Core stream wrapper 用同一 operation deadline 继续 select。

以下是被拒绝的实现，出现任何一条都算跑偏：

- `AbortSignal` 内部持有 `sources: Arc<[AbortSignal]>`，`is_aborted()` / `cancelled()` 递归
  遍历 sources；
- `AbortSignal` 再加 `parent`、`OnceLock<AbortReason>` 和 `abort_after()`：parent 先取消后，晚到的
  child timeout 仍可写 child-local reason，使已观察到的取消原因从 Aborted 变成 Timeout；
- `deadline` 只在 `is_aborted()` 被调用时惰性比较并触发 cancel；deadline 必须由正在驱动
  operation/stream 的 `sleep_until` 观察；
- `cancelled()` 返回 `Pin<Box<dyn Future>>` 并内部用 `FuturesUnordered` 聚合——既有签名
  `impl Future + Send + 'static` 够用；
- 继续放在 `shared.rs`：`shared.rs` 只放 `SharedHeaders` 等纯数据类型，`AbortSignal` 是
  控制流原语，单独成文件。

Timeout 到期后由 Core timeout/stream wrapper 直接构造 `AiMuxError::Timeout(msg)`；
`AiMuxError::from_abort_signal` 只产生 `Aborted("request aborted")`。Provider Utils 只能观察 caller
signal，不拥有或重建 total/step/semantic timeout。

### 8.1 `TimeoutConfiguration`

```rust
pub struct TimeoutConfiguration {
    pub total_ms: Option<u64>,
    pub step_ms: Option<u64>,
    pub first_chunk_ms: Option<u64>,
    pub chunk_ms: Option<u64>,
}
```

| 字段 | 范围 |
|---|---|
| `total_ms` | 整个用户 operation：attempts、backoff、parse 和 stream 生命周期 |
| `step_ms` | 一个 generation step，包括该 step 内的 attempts/backoff |
| `first_chunk_ms` | 每个 streaming step 从 `do_stream`/setup 开始到第一条 semantic output |
| `chunk_ms` | 同一 streaming step 的 semantic outputs 之间 |

Core 用同一个 `tokio::select!` 同时观察用户 abort、total/step deadline 与当前 operation
future，不构造合并 signal，也不把 timeout 写进 `AbortSignal`。外部 abort 在第一次 attempt
前触发时，不得产生 HTTP exchange。

`first_chunk_ms` 在调用 `do_stream` **之前** arm，因此覆盖 request/setup 以及 §8.3 的
first-event peek；慢握手、200 后无首帧、以及首条 semantic output 迟到都受同一 budget 约束。
被 peek 的正常事件会重新接回流首，第一条 semantic output 对用户可见前清除 timer。aimux
目前是单 step，`step_ms` 与 `total_ms` 作用域重合——照样接受并在同一处 arm，以便字段语义
跨语言一致、将来多 step 时不改 contract。

### 8.2 Semantic output

分类直接跟随 AI SDK `stream-text.ts` 的 `isOutputChunkType`：

| Aimux `StreamPart` | 重置 semantic timeout |
|---|---|
| 非空 `TextDelta` / `ReasoningDelta` / `ToolInputDelta` | 是 |
| `ToolCall` / `File` | 是 |
| start/end、metadata、source、raw、finish、error、空 delta | 否 |

第一条 output 必须先清除 first-chunk timer，再对用户可见；随后启动/reset chunk timer。
SSE keepalive 和原始 network bytes 不得重置 timer。

### 8.3 Stream retry 边界

`model.do_stream` 返回 parsed stream 前可以 retry；返回后：

- 不自动 reconnect；
- mid-stream transport/parse error 不 retry；
- 即使尚未产生 token 也不重放。

**第一条 SSE 事件是 error 时**：aimux 的 provider 现在会在 `do_stream` 返回前 peek 第一条
事件，遇到 error 直接让 `do_stream` 返回 `Err`（RFC-0016 M3）。这发生在 retry 边界**之内**，
所以它就是一次普通的 attempt 失败，按 `is_retryable` 走 Core retry——一个以 SSE error 形式
送达的 429 和以 HTTP 429 送达的是同一件事。AI SDK 的 OpenAI provider 会扫描到首个
semantic output，Aimux 则只检查第一条 event；Aimux 的检查窗口更窄，但两者都把窗口内的
provider error 留在 operation retry 边界内。`do_stream` 返回 `Ok(stream)` 之后的 error 则按
上面的规则不 retry。

---

## 9. Aimux-specific workflows

### 9.1 Submit/poll/download

`VideoModel` 已拆成 `do_start` / `do_status`（对齐 AI SDK generate-video 的
start/status 流），Core 拥有 poll 循环并分别 retry 两个阶段：

- `do_start` 由 Core retry；Core 为它铸造一次 `idempotency-key`（caller 提供的
  优先），同一个 key 覆盖所有 replay，且不泄漏给 `do_status`（AI SDK
  generate-video 同款行为）；
- `do_status` 在同一个 operation reference 上由 Core 独立 retry——poll 失败
  不会重新 submit；
- `poll.timeout` 约束轮询节奏（与 AI SDK 相同，在两次 status 检查之间判定超
  时）；挂起的 status GET 由 provider-utils 的单次交换 30s 响应上限兜底，
  retry 次数有界；
- Provider-specific polling delay 不是通用 exponential retry；
- 耗尽产生的 `RetryError` 原样透传，外层不得再次 submit。

### 9.2 Router / MoA

Composite 外层不重放；重试放在已有语义边界内：

- Router 当前 child 耗尽 retry 后才 fallback，routing 只执行一次；
- Router stream 只 retry 选中 child 的 setup，返回 stream 后不 fallback/reconnect；
- MoA 每个 reference 独立 retry，aggregator generate/stream setup 独立 retry；
- `RouterModel` / `MoaModel` 的 model default 为 0，inner 耗尽产生的 `RetryError`
  原样穿过外层，避免重跑 fallback/fanout。

### 9.3 Realtime transcription

- `stream_transcribe` 是 user operation，`do_stream` 是 Provider SPI；
- live audio 不可重放，所以 session setup 只尝试一次，不使用 operation retry；
- WebSocket 不强行套 POST/GET helper；handshake 仍必须提供等价的 URL/request context 和
  abort/error normalization；
- session 建立后不自动重建；
- `next_part(timeout)` 是会话控制流，不是本 RFC 的 operation Timeout。

### 9.4 Files

`Files::upload_file` 按 AI SDK Provider SPI 本身就是公开 operation，没有对应的
`do_upload_file`；本轮不为它虚构第二层。upload 只执行一次，Provider
`RetryConfig` 不适用于 Files。这与旧 HTTP helper 会对 upload exchange 自动重试的
行为不同；对非幂等 POST 保留该行为可能重复创建文件/上传会话，因此本轮
明确不保留。Google Files 的 poll GET 如需独立 retry，应作为后续幂等
exchange 改动，不得包住整个 upload。

---

## 10. User operations、recording 与 bindings

### 10.1 所有 modality 使用 Core operation

Core 必须提供：

```text
generate_text
stream_text
embed
generate_image
generate_video
generate_speech
transcribe
stream_transcribe
rerank
search
```

所有 binding 调用这些 user operations，不得直接调用 provider-facing `do_*`。`do_*`
继续作为 Provider SPI。

为保留现有 model-level 配置，各 model trait 提供只读 `retry_config()`；只有
Core user operation 读取它。Provider 的 `do_*` 不得重试整个 operation；§9.1 的
已创建 job 安全 exchange 例外除外。

### 10.2 Recording

retry 上移后，记录必须区分：

```text
operation_attempt: 1, 2, 3...
exchange_index: 1, 2, 3... within the attempt
```

一个 attempt 可包含 submit/poll/download 多个 exchanges。最终 outcome 保存完整
`RetryError.errors`，不能只保存最后一个错误。

机制：`RecordingContext` 持有一个跨 root/child 共享的 attempt allocator；Core 的 retry
闭包每次调用前从 allocator 取一个全 call 唯一的 attempt id，并写入**该 context 本地**的
current attempt。单次 exchange primitive 读取 local current attempt，再用同样属于该 context
的 exchange counter 编号。child 共享 allocator，但不共享 current attempt / exchange counter，
因此 MoA 并发或 Router 交错执行时，一个 sibling 的 `start_attempt` 不会重标另一个 sibling
正在进行的 exchange。provider 不碰这些计数器。

旧 `send_with_retry_raw` 在耗尽时打的 `tracing::error!`（RFC-0014 §4.2 failed 行）随
retry 一起上移到 Core 的 retry 包装：每次 attempt 失败 `warn`，最终失败 `error`，
`Aborted` 不算故障不打 error。单次 primitive 只打 exchange 级别的 `info`/`debug`。

### 10.3 Bindings

公开新增：

```text
RetryError extends AimuxError
  reason
  errors
  lastError / last_error
  message
```

每个 nested error 必须恢复真实具体类型。Node/Python 继续使用 napi-rs/PyO3 桥接；
C-derived bindings 的运输由 RFC-0030 处理。本 RFC 不把 `RetryError.errors` 降级为 JSON
envelope，也不重新引入 flat error struct。

`ApiCallError` 的新增字段、`Aborted(String)` 和 `TimeoutConfiguration.step_ms` 必须同步到
Node/Python/Go/Java/Kotlin/Swift/Flutter。

---

## 11. 迁移与改动面

### 11.1 删除映射

| 现有 Aimux | 目标 |
|---|---|
| `send_timed` | 删除；Core timeout + 对应 API helper |
| `send_stream_timed` | 删除；Core semantic timeout + stream response handler |
| public `send` / `send_stream` | 删除；改用 `post_*_to_api` / `get_from_api` |
| `send_with_retry_raw` | 删除 |
| Provider Utils retry execution | 删除；迁入 Core operation |
| `ErrorStructure` / `DEFAULT_ERROR_STRUCTURE` | 删除 |
| `parse_provider_error` | 删除；由 failed response handler 替代 |
| `TimeoutBodyStream` | 删除；由 Core semantic stream timeout 替代 |
| `get_retry_delay_ms_with_jitter` | 删除；Full Jitter 改为 `aimux-core::retry` 里 `get_delay_ms` hook 的默认实现，且不再抖动 server hint（§6.5） |
| Provider `resolve_retry_config` | 删除；effective max retries 由 Core 解析 |
| Provider-owned retry execution | 删除；旧 `RetryConfig` 类型/配置 API 保留，由 Core 执行 |
| `aimux-provider-utils/src/retry.rs` | 仅保留 `RetryConfig` 兼容 re-export；retry 实现只在 `aimux-core::retry` |
| `logging.rs::is_sensitive_key` | 删除；统一用 `aimux_core::recording::is_sensitive_key` |
| `shared.rs::AbortSignal` | 实现迁到 `abort_signal.rs`，旧路径 re-export；timeout 不塞进 signal |

旧 helper 不得保留为并行执行路径。若保留一版 deprecated forwarding wrapper，它只能转发，
不得自行 retry/timeout。

### 11.2 Provider 迁移规则

每个 HTTP operation 按 method/body/protocol 选择 AI SDK 对应组合：

| 请求/响应 | helper + successful handler |
|---|---|
| JSON POST → JSON | `post_json_to_api` + `create_json_response_handler` |
| JSON POST → SSE | `post_json_to_api` + `create_event_source_response_handler` |
| multipart POST → JSON | `post_form_data_to_api` + `create_json_response_handler` |
| POST → binary | `post_to_api` + `create_binary_response_handler` |
| GET | `get_from_api` + 对应 response handler |
| custom protocol | 对应 helper + custom `ResponseHandler` |

每个 operation 同时指定 failed handler。共享只以相同 wire schema 为依据，不预设“一个
Provider 一个 handler”，也不预设“一个调用点一个 handler”。

流式 provider 在拿到 event 流之后、`do_stream` 返回之前保留首条事件的 peek（§8.3）：
handler 负责分帧和解析，peek 与"错误就 `Err` 返回"仍是 provider 的职责，不下沉到 handler
（handler 不知道该 endpoint 的错误载荷长什么样）。

### 11.3 Rust 和 binding 改动面

| 区域 | 必要改动 |
|---|---|
| `aimux-core` | boxed `ApiCallError`、`RetryError`、Core retry/timeout wrappers、各 modality user operation |
| `aimux-provider-utils` | 新 API helpers/handlers；删除旧 send/retry/error-structure 路径 |
| `aimux-providers` | 每个 HTTP operation 迁移 helper 与两个 handlers |
| recording/tracing | operation attempt 与 exchange index 分离 |
| 7 个 bindings | 新错误字段、nested `RetryError`、`Aborted` reason、`step_ms`、改走 Core operations |
| FFI | 仅按 RFC-0030 承载新增 domain error 数据；本 RFC 不改错误运输模型 |

## 12. 实施顺序

0. 先让 `error_value_golden_test`（size guard、variant 数、`Retry`/`Aborted(String)` golden）
   和 §8.0 的 cancellation/deadline 测试通过——它们是后面每一步的回归网；
1. `ApiCallError`/`RetryError`/redaction 与 Core retry 单元测试；
2. Provider Utils API helpers 和标准 response handlers；
3. OpenAI、Anthropic 作为迁移模板；
4. 其余 language providers；
5. multimodal、polling 和 custom-protocol providers；
6. 所有 binding 改走 Core user operations；
7. recording/tracing 迁移；
8. 删除旧 send/retry/`ErrorStructure` 代码；
9. 更新被 supersede 的 RFC 和 API docs。

阶段 1–7 可以在同一分支上分多次提交，但**合并到 master 的那一刻**不得同时存在 HTTP retry
与 Core retry 两套 owner。

---

## 13. 验收条件

### Retry

1. `max_retries=0` 返回原错误；
2. 第一次 non-retryable 返回原错误；
3. `max_retries=2` 的三次 retryable failure 产生长度 3 的 `MaxRetriesExceeded`；
4. retryable 后 non-retryable 产生 `ErrorNotRetryable` 并保存完整历史；
5. Timeout/Aborted 不进入 `RetryError`；
6. `RetryError` 不嵌套且不可重试；
7. `RetryError` message 与 AI SDK fixture 逐字一致；delay 的 **hint 分支**与 AI SDK 一致，
   exponential 分支为 `rand(0..=exp)`（§6.5，测试注入固定 RNG 断言边界）；
8. 503 的 `Retry-After` 生效且不被 jitter 提前；无 hint 时 delay ∈ [0, exponential]；
8a. `[retryable, retryable, non-retryable]` @ `max_retries=2` → `MaxRetriesExceeded`；
8b. primitive 本身不引用 `ApiCallError`（编译期可查：`retry_with_exponential_backoff` 的模块不 `use crate::error::ApiCallError`）。

### HTTP/handlers

9. 每次 API helper 调用只执行一个 fetch attempt 且不 retry；自动 redirect chain 记录为一个 logical exchange；
10. successful/failed handler 只执行被 status 选中的一个；
11. 2xx body/JSON/schema failure 带完整、已脱敏的 API context；
12. 200 Provider business error 可以显式决定 retryability；
13. request context、URL、response headers/data 使用统一 redaction；
14. response body size limit 和 UTF-8 截断正确；
15. custom binary/event-stream protocol 不被误当 SSE；
15a. JSON helper 之后 provider 代码里不存在对响应体的 `serde_json::from_slice`/`from_str`；
15b. `aimux-provider-utils` 不导出任何 provider 名字的 failed handler；
15c. 每次 helper 调用恰好产生一条 exchange recording（含 transport failure）。

### Timeout/stream/workflow

16. total timeout 覆盖 attempts、backoff、parse 和 stream 生命周期；
17. first/chunk timer 只由 semantic output 重置；
18. first timer 在 `do_stream`/setup 前启动，并在第一条 output 可见前清除；
19. stream 返回后的 error 不 reconnect/retry；`do_stream` 返回前 peek 到的 retryable error 走 Core retry；
19a. stream 内 chunk 解析失败是 `JsonParse`/`InvalidResponseData` 的 `Err` 项且流继续；
19b. peek 未消费掉数据：首条事件是正常 chunk 时它仍作为流的第一项被 yield（测试：单事件 SSE 响应，断言流里能拿到该 chunk）；
19c. 首条 SSE error 为 429 时 `stream_text` 重试后成功；为 400 时立即返回原错误不重试；
20. polling failure 在已有 job 上 retry，耗尽也不重复 submit；
21. Router child 在 fallback 前耗尽 retry，MoA reference/aggregator 独立 retry，且不重跑 routing/fanout；
21a. `AbortSignal` 符合 §8.0：只有 caller cancellation；total/step deadline 直接返回对应 Timeout message，不改变 signal；
21b. 一个 operation 结束后没有残留 timer/forwarding task（实现不 spawn，测试以 paused time 验证 drop operation 后不再有可触发状态）。

### Bindings/observability

22. 所有 modality binding 调用 Core user operation；
23. 七个 binding 无损恢复 `RetryError.errors` 的具体错误类型；
24. `ApiCallError` 新字段和稳定的 `Aborted("request aborted")` 跨语言一致；
25. recording 区分 operation attempt 与 exchange index。

---

## 14. 不做与拒绝方案

本 RFC 不做：`cause`、`GatewayError`、mid-stream reconnect、stream 返回后的任何 retry、
circuit breaker、tool execution timeout、per-child Router retry policy。

拒绝：

- **HTTP retry + Core retry**：会产生乘法 attempts、双重 backoff 和嵌套错误；
- **只重命名 `send_with_retry_raw`**：retry 边界仍然错误；
- **`RetryError.errors: Vec<ApiCallError>`**：不等价于 AI SDK 的 `unknown[]`，会丢失
  operation 的异构错误历史；
- **按 raw bytes 重置 stream timeout**：keepalive/framing 不是 semantic output；
- **新的统一 `call_to_api`**：AI SDK 已有 POST/GET helpers 和 ResponseHandler 边界，
  不再发明平行 API（四个 helper 背后的 `pub(crate)` 单次 primitive 不算，见 §5.4）；
- **JSON handler 返回 `Bytes` 让 provider 再解析**：schema 失败会丢掉 API context；
- **把 timeout 合并进 `AbortSignal`**：见 §8.0；JS 需要 AbortController 取消 fetch，Rust 直接
  drop future，更简单也不会产生 parent/child reason 竞争；
- **把第一条 SSE 事件的 peek 扩展为 AI SDK OpenAI provider 的完整扫描窗口**：baseline 会
  扫描到首个 semantic output，Aimux 保留较窄的 first-event peek（§8.3）：同一个 429 不应
  因为 provider 用 SSE 而非 HTTP status 送达就得不到重试，且 C-ABI binding 用一个 nullable
  error 指针即可表达失败，无需在流里再解一层。代价是与 AI SDK 的 stream fixture 不能逐字
  对照——接受。

---

## 15. 对既有 RFC 的影响

| RFC | 影响 |
|---|---|
| RFC-0009 | 保留 shared client/pool/connect safety 与 Full Jitter（改挂在 Core retry 的 `get_delay_ms` 上）；supersede HTTP retry loop；30s client-wide timeout 改为仅包非流式 helper 的 per-exchange whole-response guard（§5.4） |
| RFC-0016 | supersede “retry 在 HTTP 层”和 raw-byte stream timeout 的实现结论；**保留** M3 的首条 SSE 事件 peek，并明确它是 retry 边界内的 attempt 失败（§8.3） |
| RFC-0017 | `max_retries` 仍可作 model default，但由 Core 解析和执行 |
| RFC-0021 | composite 作为一个 attempt；per-child policy 另行设计 |
| RFC-0023 | attempt 拆为 operation attempt + exchange index |
| RFC-0028 | realtime session 控制流保持独立；仅 handshake 使用 operation policy |
| RFC-0030 | 负责 nested errors 的 C ABI 运输；不改变本 RFC 的 domain error 模型 |

---

## 16. 最终契约

> Core 用 AI SDK 同名 retry/timeout 函数包围完整 `model.do_*`；Provider 用 AI SDK
> 同名 POST/GET helpers 和 successful/failed ResponseHandler 完成一次 fetch attempt；
> stream 返回后不重放；旧 `send*`、HTTP retry 和 `ErrorStructure` 路径全部删除。
