# RFC-0047：错误体系与错误原因（cause）

> **Status**: Draft
>
> **Date**: 2026-10-19
>
> **Scope**: `AiMuxError` 的结构与 `cause`、传输层错误链、ops 错误信封的四个域、#268 生成的错误 DTO、各语言绑定的原生错误、录制里的错误。不改错误名称、错误码、消息文字和 C ABI 函数。
>
> **Baseline**: `master@42208472413a9b157428924e3f4785bb009bc27f`（#268 合并后，DTO 由生成器生成，ts-rs 已删除）。
>
> **Related**: [RFC-0039](0039-ops-bindings-contract.md) §6 错误信封、[RFC-0042](0042-v4-type-boundaries.md) 错误联合归属、RFC-0044（#279）共享错误枚举的归属、RFC-0048（#325）录制 schema 3、#349（信封去掉 `retryAfterMs`）、`docs/error-model.md`、`docs/ai-sdk-request-pipeline.md` §3、#268、#280（本 RFC 的原型实现，拆分后关闭）。
>
> **Amends**: `docs/ai-sdk-request-pipeline.md` §3 与 §14 “`cause` 本轮不加入”。

## 1. 决策摘要

1. **aimux 的错误体系有意和 AI SDK 不同，但 `cause` 要对齐。** AI SDK 的错误是 JS 类，`cause` 可以挂任何对象；aimux 的错误是一个可克隆、可序列化、要跨 C ABI 和 8 种语言的 Rust 枚举。所以 aimux 不能照搬“挂原始对象”，而是把原因记成一条可序列化的链。
2. **AI SDK 哪个错误类接受 `cause`，aimux 对应的变体就有 `cause`，其他变体没有。** 底层错误在构造处手上有的，必须传进去；不得再 `format!` 进消息后丢掉对象。
3. **一个 cause 只有两种：** 嵌套的 `AiMuxError`（原样保留，带自己的 `name`），或 `SourceError { message, cause? }`（OS、TLS、JSON 解析器等外部错误沿 `source()` 一层层抄下来）。
4. **Rust 里 cause 就是 `Error::source()`。** 继续用 thiserror，`cause` 字段标 `#[source]`；只有算出来的 source 才手写：`RetryError` 的 source 是最后一次尝试（AI SDK 的 `lastError`），`ApiCallError` 的 source 是它的 `cause`。
5. **消息文字不变，仍包含底层错误的文字。** C ABI 的 `aimux_error_message` 和只打印消息的调用方只看得到这一行，不能因为有了 cause 就把文字去掉。
6. **ops 四个域用同一个 `cause` 形状。** `aimux` 域就是 `AiMuxError` 的 cause；`recording`、`binding`、`protocol` 域有底层错误时（I/O、serde）也放进 `cause`，没有就不放。各语言只有一条解码路径：读 `error.cause`。
7. **C ABI 不加函数。** cause 只在 `aimux_error_json` 返回的 JSON 里；代码、消息、JSON 三个 getter 保持冻结。
8. **DTO 由 #268 的生成器从 Rust 类型生成。** `ErrorCause`、`SourceError` 是普通的 schemars 类型，生成器按“先 `AiMuxError`、后 `SourceError`”的有序无标签联合生成；Python 的嵌套错误投影把 `cause` 加进每个带 cause 的变体。
9. **宿主语言自己的异常不跨 ABI。** 回调里宿主抛的异常仍在宿主那边挂成原生 cause（例如 Java 的 `CallbackFailure`），不会写进信封。
10. **传输错误先去掉 URL。** URL 的 query 里可能有 API key；cause 链和消息都不得带它。
11. **实现分三个 PR，基于 #268 合并后的 master，用它的生成器。** 每个 PR 带自己的测试，单独过 CI。

## 2. 现状和问题

### 2.0 现在错误怎么走

先看整体：一个非流式调用从入口到调用方，错误在哪里产生、在哪里分类、在哪里重试、最后怎么变成各语言的异常。

```text
generate_text                                   aimux-core/src/generate.rs:954
└ retries.retry(..)                             generate.rs:1133          重试边界
  │ └ retry_with_exponential_backoff            aimux-core/src/retry.rs:103
  │   ├ 可重试：AiMuxError::is_retryable        aimux-core/src/error.rs:691   + 响应头提示 retry_header_delay（retry.rs:225）
  │   └ 次数用完或不可重试 → AiMuxError::Retry(RetryError { reason, errors })   retry.rs:163 / 192
  └ do_generate_with_logging                    generate.rs:2266
    └ OpenAIChatLanguageModel::do_generate      aimux-providers/src/openai/model.rs:166
      └ execute_generate                        openai/model.rs:221
        └ post_json_to_api                      aimux-provider-utils/src/post_to_api.rs:17
          └ execute_exchange                    post_to_api.rs:144
            ├ 传输失败                          http.rs:360 → fetch.rs:430 → map_reqwest_error（fetch.rs:445）
            │ └ fetch_error_to_ai_mux_error     handle_fetch_error.rs:29   → ApiCall，无状态码，可否重试看 FetchError
            ├ 非 2xx                            post_to_api.rs:166
            │ └ failed_response_handler         openai_failed_response_handler（openai/mod.rs:64）
            │   │                                → create_json_error_response_handler（response_handler.rs:220）
            │   └ handler 自己失败 → ApiCall { status_code, is_retryable: is_retryable_status }   post_to_api.rs:172、error.rs:651
            └ 2xx                               post_to_api.rs:188
              └ successful_response_handler 失败 → ApiCall “Failed to process successful response”   post_to_api.rs:193
失败后录制
└ record_outcome(OutcomeRecord::from_error)     generate.rs:1144 → recording.rs:484   存 Display 文字和整个 AiMuxError
```

流式调用多两点：重试只包住拿到第一个事件之前；之后的错误作为事件交给调用方，不抛出。

```text
stream_text                                     aimux-core/src/generate.rs:1541
└ retries.retry(..)                             generate.rs:1661          只到第一个事件
  └ do_stream → execute_stream                  openai/model.rs:184 / 376
    └ create_event_source_response_handler      aimux-provider-utils/src/response_handler.rs:358
      ├ 流里的 error 帧 → openai_stream_error   openai/mod.rs:91   → ApiCall
      └ 传输中断 SseError → ApiCall             response_handler.rs:409
└ TextStreamPart::Error { error }               generate.rs:1887-1890     作为事件交付
```

跨语言时，同一个错误在 ops 边界投影成信封一次，各语言从信封还原：

```text
aimux_call / aimux_stream                       aimux-ffi/src/abi.rs:138
└ dispatch::call                                aimux-ops/src/dispatch.rs:298
  └ OpsError::project_with_clock                dispatch.rs:338
    └ OpsError::envelope                        aimux-ops/src/envelope.rs:116
        { kind, code, message = Display, error = payload, retryAfterMs }
└ finish                                        abi.rs:35   → aimux_error_t*
  ├ aimux_error_code / _message / _json         abi.rs:364 / 371 / 380
  └ Node bindings/node/src/error.rs:37、Python _errors.py:97 from_envelope、
    Java AimuxResult.decode、Go decodeError、Swift takeError、Dart OpsException.fromJson、C++ aimux::check
```

分类只看结构化字段：HTTP 状态在 `ApiCallError.status_code`，可否重试在 `is_retryable`，重试提示从 `response_headers` 算。整条路上没有任何一处保留底层错误对象，下面逐段看丢在哪里。

下面每段从调用方用的公开函数开始，一路往下到丢掉原因的那一行（标 `✗`），旁边是上游 AI SDK 同一位置的做法，最后是改法。行号都是基线 master 上的。

### 2.1 网络连不上：reqwest 的原因链只剩第一层

```text
generate_text                              aimux-core/src/generate.rs:954
└ do_generate_with_logging                 aimux-core/src/generate.rs:2266
  └ OpenAIChatLanguageModel::do_generate   aimux-providers/src/openai/model.rs:166
    └ post_json_to_api                     aimux-provider-utils/src/post_to_api.rs:17
      └ execute_exchange                   aimux-provider-utils/src/post_to_api.rs:144
        └ send_one_request                 aimux-provider-utils/src/http.rs:360
          ├ ReqwestFetch::fetch            aimux-provider-utils/src/fetch.rs:430
          │ └ ✗ map_reqwest_error          fetch.rs:445-455          error.to_string()
          └ ✗ fetch_error_to_ai_mux_error  handle_fetch_error.rs:29-40   ApiCallError::new(other.to_string(), …)
上游 handle-fetch-error.ts:56-95   new APICallError({ message: 'Cannot connect to API: …', cause: error, isRetryable: true })
```

连一个没开的端口，reqwest 的错误其实是三层：

```text
error sending request for url (http://127.0.0.1:9/x?key=SECRET)   reqwest
└ tcp connect error                                                hyper
  └ Connection refused (os error 111)                              操作系统
```

`map_reqwest_error` 只留第一层，真正的原因没了，而且这一层带着完整 URL，URL 的 query 里可能是 API key。SigV4、Vertex、Azure 的自定义传输里还有同样的 8 处。

**改法**：`FetchError` 改为携带 `SourceError`，先 `reqwest::Error::without_url()` 再把整条 `source()` 链抄下来；`fetch_error_to_ai_mux_error` 把它放进 `ApiCallError.cause`。

### 2.2 成功响应解析失败：serde 错误只留在消息里

```text
post_json_to_api                           aimux-provider-utils/src/post_to_api.rs:17
└ execute_exchange                         post_to_api.rs:144
  └ create_json_response_handler           aimux-provider-utils/src/response_handler.rs:105
    └ ✗ serde_json::from_slice::<T>        response_handler.rs:130-140   format!("Invalid JSON response: {error}")
上游 response-handler.ts:216-226   new APICallError({ message: 'Invalid JSON response', cause: parsedResult.error, … })
```

**改法**：serde 错误作为 cause，消息不变。

### 2.3 handler 自身失败：包成 “Failed to process … response: …”

```text
execute_exchange                           aimux-provider-utils/src/post_to_api.rs:144
├ ✗ 非 2xx 分支                            post_to_api.rs:172-183   format!("Failed to process error response: {error}")
└ ✗ 2xx 分支                               post_to_api.rs:193-200   format!("Failed to process successful response: {error}")
上游 post-to-api.ts:128-161   new APICallError({ message: 'Failed to process … response', cause: error, … })
```

**改法**：原来的 `AiMuxError` 整个作为 cause（保留它的 `name`、状态码）。消息仍带 `: {error}` 后缀，因为 C ABI 只暴露消息。

### 2.4 流式中途断开：SSE 传输错误转成文字

```text
stream_text                                aimux-core/src/generate.rs:1541
└ OpenAIChatLanguageModel::do_stream       aimux-providers/src/openai/model.rs:184
  └ create_event_source_response_handler   aimux-provider-utils/src/response_handler.rs
    ├ ✗ SseError::Stream(error)            response_handler.rs:409-421   ApiCallError::new(error.to_string(), …)
    └ ✗ 其它 SseError                      response_handler.rs:423-436   同上
上游 response-handler.ts:60-80   new APICallError({ message: 'Failed to process successful response', cause: error, … })
```

`SseError` 自己带 `#[source]`，原因链在这一行之前都还在。同类的还有读取响应体（`read_response_with_size_limit.rs:68`）、WebSocket（`ws.rs:157`）、Bedrock event stream（`bedrock/model.rs:50`）、Stability 和 xAI 的 JSON 解析。

**改法**：同 2.2。

### 2.5 JSON 解析和 serde 转换：只留字符串

```text
generate_text（带 output 规格）            aimux-core/src/generate.rs:954
└ ✗ parse_json                             aimux-core/src/output.rs:251-264   JsonParse(format!("… Error message: {message}"))
任意 provider 里的 `?`
└ ✗ impl From<serde_json::Error> for AiMuxError   aimux-core/src/error.rs:636-644   e.to_string()
上游 parse-json.ts:55        throw new JSONParseError({ text, cause: error })
上游 validate-types.ts:28    TypeValidationError.wrap({ value, cause: result.error })
```

**改法**：`JsonParse` 改为 `{ message, cause }`，`From<serde_json::Error>` 把 serde 错误自己作为 cause。

### 2.6 工具参数不合法：cause 是字符串

```text
generate_text                              aimux-core/src/generate.rs:954
└ parse_tool_call                          aimux-core/src/parse_tool_call.rs
  ├ ✗ parse_json_input                     parse_tool_call.rs:299-322   InvalidToolInput { cause: format!("JSON parsing failed: …") }
  └ ✗ schema 校验                          parse_tool_call.rs:281-296   InvalidToolInput { cause: format!("Type validation failed: …") }
上游 parse-tool-call.ts:208/259   new InvalidToolInputError({ toolName, toolInput, cause: JSONParseError | TypeValidationError })
```

**改法**：`InvalidToolInput.cause` 改为 `ErrorCause`，放 `JsonParse` 或 `TypeValidation` 错误对象。`TypeValidation { message, value, cause }` 是新增变体，对应上游 `TypeValidationError`；结构化输出不合 schema 时（`InvalidResponseData`，上游 `NoObjectGeneratedError`）cause 也是它。

### 2.7 参数、prompt、下载：底层错误 format! 进消息

```text
✗ download_guard.rs:65            InvalidArgument(format!("invalid download URL: {error}"))   url::ParseError
✗ language_model_message.rs:780   InvalidPrompt(format!("Invalid URL: {given}: {e}"))        url::ParseError
✗ google/speech.rs:493            InvalidResponseData(format!("invalid base64 audio: {e}"))  base64::DecodeError
✗ download.rs:316                 Other(format!("download failed: {error}"))                 reqwest::Error，文字带 URL
上游 InvalidArgumentError / InvalidPromptError / AISDKError 的构造参数都带 cause
```

这一类在 #268 合并前的 master（`5743a40a`）上共 109 处：`InvalidArgument` 72、`InvalidResponseData` 20、`JsonParse` 15、`InvalidPrompt` 1、`Other` 1；#268 合并后实现时补上 cause 的共 117 处（#290）。它们都是只带一个字符串的变体。

**改法**：六个字符串变体改为 `{ message, cause }`，有底层错误的地方都传进去。

### 2.8 跨语言：Rust 已经只剩文字，其他语言更拿不到

```text
aimux_call                                 aimux-ffi/src/abi.rs:138
└ dispatch::call                           aimux-ops/src/dispatch.rs:298
  └ OpsError::project_with_clock           aimux-ops/src/dispatch.rs:338
    └ OpsError::envelope                   aimux-ops/src/envelope.rs:116   AiMuxError 整个序列化进 error（没有 cause 字段）
      ├ ✗ recording_payload                envelope.rs:204-219   io::Error 的 source.to_string()
      ├ ✗ BindingError::payload            envelope.rs:222       serde 错误早已是字符串（abi.rs:63、dispatch.rs:368）
      └ ✗ ProtocolError::payload           envelope.rs:248       serde 错误早已是字符串（dispatch.rs:221）
aimux_error_json                           aimux-ffi/src/abi.rs:380   各语言从这里拿 JSON
├ ✗ Node    bindings/node/src/error.rs:37           create_error(message)，不设 cause
├ ✗ Python  bindings/python/python/aimux/_errors.py:91   只有 ToolCallRepair 设 __cause__
├ ✗ Java    NativeException.java:12                 super(message)，不 initCause
├ ✗ Go      bindings/go/error.go:15                 没有 Unwrap()
├ ✗ Swift / Flutter                                 只能翻原始 JSON
└ ✗ C++     bindings/c/aimux_raii.hpp:22-26         throw std::runtime_error(message)，连错误码都丢了
上游 JS：error.cause 就是原始错误对象
```

**改法**：见 §5 和 §7。

### 2.9 为什么是问题

- **排查不了。** 地址错、服务没开、防火墙、证书不对，区别都在丢掉的那几层。
- **程序没法按原因处理。** 只能匹配会随版本变化的文字。
- **泄露 key。** 带 URL 的那一行会进日志、录制文件，并跨语言传给调用方。

## 3. aimux 错误体系和 AI SDK 的差异

| 方面 | AI SDK | aimux | 原因 |
|---|---|---|---|
| 形态 | 一组 JS 类，`isInstance` 判断 | 一个封闭的 Rust 枚举 `AiMuxError`，JSON 以 `name` 区分变体（名字与 AI SDK 相同） | 要 `Clone`、`Serialize`，要跨 C ABI |
| 错误名 | `AI_APICallError` 等 | 相同；未知名字在绑定里保留为 Unknown | 跨语言判断只能靠名字 |
| 可否重试 | `isRetryable` 属性 | 同名字段，存在 `ApiCallError` 上 | 一致 |
| 重试提示 | 每次从响应头算 | 信封顶层 `retryAfterMs`，在 ops 边界按一次时钟算一次 | RFC-0039 §6，避免各绑定各算一遍 |
| 消息 | 只有自身消息 | `Display` 带前缀（`API call error: …`），并包含底层错误文字 | C ABI 只暴露消息一行 |
| `cause` | `unknown`，任意对象 | 嵌套 `AiMuxError` 或 `SourceError` 链 | 原始 Rust 错误对象不能克隆、序列化、跨语言 |
| 哪些错误有 `cause` | 构造参数接受 `cause` 的类 | 对应变体（见 §4.2） | 对齐 |
| 错误上下文 | 原样带请求上下文 | 白名单、脱敏、大小上限；传输错误去 URL | 错误会进录制、跨 FFI |
| 错误域 | 只有 SDK 错误 | ops 信封分 `aimux`、`recording`、`binding`、`protocol` 四个域，各有错误码区间 | RFC-0039 §6 |
| 没有的类型 | `GatewayError`、`StreamProviderError` 等 | 不加；流中错误帧映射为 `ApiCallError` | 已登记 |

## 4. Rust 设计

### 4.1 类型

```rust
/// 外部错误（I/O、TLS、解析器……）的快照：自身文字和它 source() 链上每一层的文字。
pub struct SourceError {
    pub message: String,
    pub cause: Option<Box<SourceError>>,
}

/// 一个错误是从什么错误构造出来的：AI SDK 的 `cause`。
#[serde(untagged)]
pub enum ErrorCause {
    AiMux(Box<AiMuxError>),   // 带 name 的 aimux 错误，原样保留
    Source(SourceError),      // { message, cause? }
}
```

`cause` 是变体上的字段，`ErrorCause` 是这个字段的类型：

```rust
pub enum AiMuxError {
    InvalidArgument {
        message: String,
        #[source]
        cause: Option<ErrorCause>,
    },
    // JsonParse、InvalidResponseData、InvalidPrompt、NoOutputGenerated、Other 同形；
    // ApiCallError 结构体上也是 `cause: Option<ErrorCause>`。
    ...
}
```

单独定义类型，是因为这个字段要放两种东西：另一个 aimux 错误（原样嵌套），或外部库的错误（不能克隆、序列化、跨语言，只能抄成 `SourceError`）。JS 的 `cause` 可以是任意对象，Rust 做不到，所以多这一层；序列化后仍是 payload 里的一个 `cause` 字段。

`SourceError` 和 `ErrorCause` 都实现 `std::error::Error`，`source()` 返回下一层，所以整条链对 `anyhow`、`{:#}`、`tracing` 都可见。`ErrorCause::source(&e)` 是唯一的入口：`e` 若能 downcast 成 `AiMuxError` 就原样嵌套，否则抄成 `SourceError`。

JSON 形状：

```json
{ "name": "AI_APICallError", "message": "Cannot connect to API: tcp connect error", "isRetryable": true,
  "cause": { "message": "tcp connect error", "cause": { "message": "Connection refused (os error 111)" } } }
```

没有原因时不输出 `cause`。

### 4.2 哪些变体有 cause

| aimux 变体 | 对应的 AI SDK 类 | cause |
|---|---|---|
| `ApiCall` | `APICallError` | 可选（`ApiCallError.cause`，`.caused_by(..)`） |
| `JsonParse` | `JSONParseError` | 可选 |
| `InvalidResponseData` | `InvalidResponseDataError`；结构化输出不合 schema 时对应 `NoObjectGeneratedError` | 可选（不合 schema 时是 `TypeValidation`） |
| `TypeValidation`（新增） | `TypeValidationError`，带 `value` | 可选（校验器错误）；只作为 cause 出现，ops 错误码与 `InvalidResponseData` 共用 3 |
| `InvalidArgument` | `InvalidArgumentError` | 可选 |
| `InvalidPrompt` | `InvalidPromptError` | 可选 |
| `NoOutputGenerated` | `NoOutputGeneratedError` | 可选 |
| `Other` | `AISDKError` | 可选 |
| `InvalidToolInput` | `InvalidToolInputError` | 必有，改为错误对象 |
| `ToolCallRepair` | `ToolCallRepairError` | 必有（已有），接上 `source()` |
| `Retry` | `RetryError` | 不加字段；`source()` 是最后一次尝试 |
| 其余（`Timeout`、`Aborted`、`TokenExpired`、`NoSuchModel` 等） | 不接受 `cause` | 没有 |

只带一个字符串的六个变体改成 `{ message, cause }`，配同名构造函数（`AiMuxError::invalid_argument(..)`、`json_parse(..)` 等）。新增 `TypeValidation { message, value, cause }`，名字 `AI_TypeValidationError`；上游的 `context`（field、entityName、entityId）aimux 现在没有地方填，不加。

### 4.3 传输层

`FetchError` 的连接、I/O、其他三类改为携带 `SourceError`，用 `reqwest::Error::without_url()` 构造。`fetch_error_to_ai_mux_error` 把它作为 `ApiCallError.cause`。SSE、WebSocket、成功响应解析失败、handler 失败同样把原错误作为 cause，消息文字不变。

改完以后 §2.1 那条路径：

```text
ReqwestFetch::fetch                        aimux-provider-utils/src/fetch.rs
└ map_reqwest_error                        error.without_url() → FetchError::Connect(SourceError::new(&error))
fetch_error_to_ai_mux_error                aimux-provider-utils/src/handle_fetch_error.rs
└ ApiCallError { is_retryable, cause: Some(ErrorCause::source(&fetch_error)), .. }
调用方看到：
API call error: Cannot connect to API: tcp connect error      ← Display，消息不变
└ cause: tcp connect error                                    ← source()
  └ cause: Connection refused (os error 111)                  ← URL 已去掉
```

## 5. ops 层

### 5.1 信封

信封结构不变：`{kind, code, message, error, retryAfterMs?}`。`message` 仍是完整 `Display`；`error` 是该域的 payload，`cause` 是 payload 里的可选字段。

| 域 | payload 里的 cause | 来源 |
|---|---|---|
| `aimux` | `AiMuxError` 自己的 `cause` | §4 |
| `recording` | `Init`、`OpenFile`、`Spawn` 的 `std::io::Error` | 现在是字符串字段 `source`，改为 `cause: SourceError`（见 §10 问题 2） |
| `binding` | `InvalidWireJson`、`ResultSerialization` 的 serde 错误 | 现在只有文字 |
| `protocol` | 一般没有；`InvalidParams` 的 serde 错误 | 现在只有文字 |

四个域同一个形状，绑定只需要一条解码路径。错误码、名字和消息都不变。

改完以后同一条路径是这样的：

```text
aimux_call                                 aimux-ffi/src/abi.rs:138
└ dispatch::call                           aimux-ops/src/dispatch.rs:298
  └ OpsError::envelope                     aimux-ops/src/envelope.rs:116
    ├ aimux 域：serde_json::to_value(AiMuxError)   cause 随 AiMuxError 一起序列化
    ├ recording_payload                    io::Error → cause: SourceError::new(source)
    ├ BindingError::payload                InvalidWireJson / ResultSerialization 保存 serde 错误 → cause
    └ ProtocolError::payload               InvalidParams 保存 serde 错误 → cause
aimux_error_json                           aimux-ffi/src/abi.rs:380   不变，JSON 里多了 error.cause
```

`BindingError::InvalidWireJson`、`ResultSerialization`、`ProtocolError::InvalidParams` 现在构造时就把 serde 错误转成了字符串（`abi.rs:63`、`dispatch.rs:221`、`dispatch.rs:368`），改为保存 `SourceError`。

### 5.2 C ABI

不加函数。`aimux_error_code`、`aimux_error_message`、`aimux_error_json` 三个 getter 冻结；cause 只在 JSON 里。C++ 包装 `aimux::check` 改抛 `aimux::NativeError`（继承 `std::runtime_error`），带错误码和完整 JSON。

### 5.3 宿主异常

宿主回调（流消费者）抛出的异常不跨 ABI。绑定把它挂成本语言的 cause（Java `CallbackFailure.initCause`，Go 的 callback 错误），信封 JSON 里没有它。以后支持宿主实现 provider（#274）时，宿主异常链转成 `SourceError` 再传进来，形状与 §4.1 相同。

### 5.4 重试提示

`retryAfterMs` 仍只在顶层是 `ApiCall` 时给出。cause 里嵌套的 `ApiCall` 不产生顶层提示：调用方要重试的是外层错误。

## 6. DTO（#268 生成器）

#268 用 `aimux-codegen` 从 Rust 类型的 schemars schema 生成各语言 DTO，ts-rs 和 `scripts/gen_ts_types.py` 被删除。错误 DTO 已经出现在 `TextStreamPart::Error`、`RetryError.errors`、`ToolCallRepair` 里，并有 `contract-tests/dto/AiMuxError` 向量。

生成链路（master `42208472`）：

```text
cargo run -p aimux-codegen                 tools/aimux-codegen/src/main.rs:32
└ aimux_ops::manifest()                    aimux-ops/src/manifest.rs:41   typeDefs = 各 Rust 类型的 schemars schema
  └ generate                               tools/aimux-codegen/src/lib.rs:190
    ├ 模型：anyOf + "x-aimux-codec": "ordered-untagged" → Kind::Untagged   tools/aimux-codegen/src/model.rs:731
    └ 各语言 emitter                         tools/aimux-codegen/src/emit/{node,python,java,go,swift,dart}.rs
      └ Python：_NESTED_ERROR_FIELDS        tools/aimux-codegen/src/emit/python.rs:391   按“字段类型含错误”逐变体推出
运行时 Python 还原
└ from_envelope                            bindings/python/python/aimux/_errors.py:99
  └ payload_fields                         _error_payloads.py（生成）  只把 _NESTED_ERROR_FIELDS 里的字段还原成异常
```

所以只要 Rust 类型对，DTO 不用手改：

1. `ErrorCause`、`SourceError` 派生 `JsonSchema`，作为普通类型进入 manifest `typeDefs`，不手写任何语言的类型。
2. `ErrorCause` 标 `#[schemars(extend("x-aimux-codec" = "ordered-untagged"))]`（`content.rs:508` 已有同样用法），生成为**有序无标签联合**：先试 `AiMuxError`（必有 `name`），再试 `SourceError`（只要求 `message`）。生成器已有 `Kind::Untagged` 和按必需字段选分支的逻辑；`AiMuxError` 已经通过 `RetryError.errors` 递归，所以递归不是新能力。
3. Python 的 `_NESTED_ERROR_FIELDS` 由 `emit/python.rs:391` 的 `contains_error` 推出，要让它把 `ErrorCause` 也算作错误类型，每个带 cause 的变体都含 `cause`，嵌套的 aimux 错误还原成对应异常类，`SourceError` 还原成 `aimux.SourceError`。
4. DTO 向量新增三类：带传输链的 `ApiCall`、cause 是嵌套 aimux 错误、没有 cause；每种语言的生成 codec 都要往返通过（`scripts/check_dto_vectors.py`）。
5. 输入方向（`AiMuxErrorInput`）接受并保留 `cause`，规则与输出相同。
6. `SourceError.cause` 的类型是 `SourceError` 自己。Swift 的 struct 不能内联存放自身类型，生成器为这种“可选、类型是所在 struct 本身”的字段生成 `WireIndirect` 盒子，属性名和 JSON 不变（#289，`tools/aimux-codegen/src/emit/swift.rs` 的 `boxed`）。其他语言的生成物没有这个问题。

## 7. 各语言

| 语言 | 原生入口 |
|---|---|
| Rust | `source()` 链 |
| Node | 标准 `error.cause`：嵌套 aimux 错误是 `AimuxCauseError`（`name` 是 AI SDK 名字，`error` 是整个 payload），外部错误是 `AimuxSourceError`；两者都继承 `Error`，有自己的 `cause` 和 stack |
| Python | `__cause__`；嵌套错误是对应子类，外部错误是 `SourceError` |
| Java / Kotlin | `getCause()` 链上的 `ErrorCause`（`getErrorName()` 对外部错误为空）；Kotlin 跟随 Java |
| Go | `(*Error).Unwrap()` 返回 `*aimux.Cause`，`errors.Unwrap` / `errors.As` 可用 |
| Swift / Flutter | `cause` 属性 |
| C / C++ | JSON 的 `error.cause`；C++ `aimux::NativeError::json()` |

`Retry` 没有 `cause` 字段，wire 上只有 `errors`。它的 Rust `source()` 是最后一次尝试（AI SDK 的 `lastError`），所以每个绑定从 `errors` 的最后一个合成原生 cause：Node `error.cause`、Python `__cause__`（同 `last_error`）、Java / Kotlin `getCause()`、Go `Unwrap()`、Swift / Flutter `cause`；`errors` 为空时没有 cause。嵌套在别处的 `Retry` 同样处理。每种绑定都有这条测试（#293）。

Java 有两个同名类型，分工不同：`ai.arcships.aimux.types.ErrorCause` 是生成的 DTO，用在 `RetryError.errors`、事件里的错误等数据字段，按 §6 的有序无标签联合解码；`ai.arcships.aimux.ErrorCause` 是手写的 `RuntimeException`，只为挂在 `getCause()` 上，包住同一段 JSON（`getErrorName()`、`getError()`），不重新解码。Node 的 `AimuxCauseError`、`AimuxSourceError`，Python 的 `SourceError`，Go 的 `Cause` 是同样的手写异常层；生成的 DTO 类型名不变。

## 8. 录制与回放

`Outcome::from_error` 把整个 `AiMuxError` 写进 `error_value`，所以 cause 链会进录制文件。在 master 上（schema 2）这是可选字段：没有它的旧文件照样能读，旧读取器遇到它会忽略（错误类型没有 `deny_unknown_fields`）。

RFC-0048（#325）落地后录制只有 schema 3，schema 1、2 的读取删除（RFC-0048 §1 第 1 条），上一段的“旧文件可读”随之只适用于 RFC-0048 之前。schema 3 的 `outcome.errorValue` 仍是 `AiMuxError` 的 JSON，`cause` 照样在里面，且只在 `payload` 策略下存（RFC-0048 §3）；这条不依赖 schema 3，也不需要等它。回放出来的错误带着录下来的 cause。

RFC-0048 的回放引擎把录下的传输终态映射回 `FetchError`，并用 `FetchError::Other("<Code>: …")` 报引擎错误。本 RFC 之后 `FetchError` 的连接、I/O、其他三类携带 `SourceError`（§4.3），所以那里写成 `FetchError::Other(SourceError::from("<Code>: …"))`；录下的终态只有 `kind` 和 `message`，回放出来的是一层没有下级的 `SourceError`。

## 9. 安全

- 传输和下载错误一律用 `without_url()` 构造，消息和 cause 都不带 URL（URL 的 query 里可能有 API key）。
- `SourceError` 抄的是外部错误的 `Display` 文字，可能包含数据。serde_json 的 data 类错误会引用违规的值（如 ``invalid type: string "…", expected u32``、``unknown variant `…` ``），值来自调用方的参数或模型的响应。这些文字在本 RFC 之前就在消息里（`dispatch.rs:221`、`abi.rs:63` 的 `error.to_string()`），cause 不新增暴露面，但也没有减少它。url、base64、io 的错误文字不含输入内容；新增外部错误类型时要确认这一点。
- `TypeValidation.value` 是不符合 schema 的整个值，和 AI SDK 一样，没有大小上限，也不脱敏。
- 信封现在没有大小上限，也不对值脱敏。录制对 `error_value` 做按键名的脱敏（`recording.rs` 的 `redact_json`，凭据类键名的值被替换），会作用到 cause 里的同名键，但不处理文字里引用的值。

因此错误 JSON（包括 cause 和 `value`）和请求、响应体同样敏感：写日志、发往第三方前由调用方决定是否保留。是否在 aimux 里加大小和脱敏边界见 §10 问题 5。

## 10. 待讨论

1. **cause 链要不要设深度上限。** 现在由 `source()` 链自然决定，`Retry` 的每次尝试各自带链。倾向不设：网络错误一般三四层，重试只有几次尝试，整条链也就几十行文字。
2. **`recording` 域的 `source` 字符串改名为 `cause`。** 已采纳（维护者 review 建议标为已采纳，#293 已实现）：四个域同一个形状；`recording-*.json` 黄金样例随之改变，属于发布前的破坏性变更（§11）。
3. **`binding`、`protocol` 域要不要带 cause。** 已采纳（同上，#293 已实现）：有 serde 错误时带，没有就不带；`BindingError`、`ProtocolError` 的 `source()` 也返回它。
4. **C++ 要不要提供解析 cause 的辅助函数。** 倾向不提供，C++ 没有 JSON 依赖，调用方自己解析 `json()`。
5. **错误 JSON 要不要加大小和脱敏边界（§9）。** 两种做法：
   - A（倾向）：只限大小，不改文字。`SourceError` 每层 `message` 最多 1 KiB（按字符边界截断，末尾加 `…`）；`TypeValidation.value` 序列化后超过 16 KiB 时写 `null`，`message` 末尾注明原大小。消息里已有的 serde 文字不变，调用方按 §9 把错误 JSON 当作请求、响应体看待。
   - B：在 A 之外，serde_json 的 data 类错误（`classify() == Category::Data`）构造 `SourceError` 和消息时只留类别和位置（`invalid data at line L column C`），不留引用的值。代价是 `InvalidParams`、`InvalidWireJson`、响应解析失败的消息文字改变，丢掉 `expected u32` 这类提示。

## 11. 兼容性

不只是新增可选字段，下面每一条都写入 CHANGELOG 的 Breaking：

- **错误名、错误码不变。** 新增 `AI_TypeValidationError`，与 `InvalidResponseData` 共用错误码 3，只作为 cause 出现。
- **消息文字**：只有传输和下载错误变化，reqwest 原来的 `for url (…)` 不再出现（§9）。只按消息匹配的调用方要改。
- **wire JSON 的形状变化**：
  - `InvalidToolInput.cause` 从字符串变为错误对象（`AI_JSONParseError` 或 `AI_TypeValidationError`），按字符串读它的代码要改。
  - `recording` 域 `Init`、`OpenFile`、`Spawn` 的 `source` 字符串改为 `cause: { message, cause? }`（§10 问题 2），`recording-*.json` 黄金样例随之改变。
  - 其余是新增可选字段 `cause`，按 `docs/error-model.md` 属于兼容扩展；旧绑定读到它会保留或忽略。
- **Rust API**：六个字符串变体改为结构体变体；`InvalidToolInput.cause` 类型变化；`FetchError` 的载荷类型变化；新增变体 `TypeValidation`（对 `AiMuxError` 穷尽匹配的代码要加一支）；`BindingError::InvalidWireJson`、`ResultSerialization`、`ProtocolError::InvalidParams` 改为带 `cause` 的结构体变体。
- **绑定**：Node 的 `error.cause`、Python 的 `__cause__`、Java 的 `getCause()`、Go 的 `Unwrap()` 从空变为有值，`Retry` 的是最后一次尝试（§7）；C++ `aimux::check` 改抛 `aimux::NativeError`（`std::runtime_error` 的子类，原来的 `catch` 仍然有效）。

### 11.1 与其他 RFC 的关系

- **RFC-0044（#279）**：L7 把共享错误枚举放进 `aimux-provider`。`ErrorCause`、`SourceError` 和 `AiMuxError` 在同一个文件（`aimux-core/src/error.rs`），随它一起搬，不单独定归属；本文写 `aimux-core` 只是因为基线还没拆分。RFC-0044 也写明每个错误补 `cause` 另开小 PR，不放进拆分步骤，与本文的三个 PR 一致。
- **RFC-0048（#325）**：schema 1、2 读取删除，见 §8；回放引擎的 `FetchError::Other` 载荷随 §4.3 变为 `SourceError`。
- **#349**：信封去掉 `retryAfterMs` 和时钟（`project_with_clock`、`Projected`）。§5.1 的信封结构和 §5.4 以它合并前的 master 为准；#349 合并后信封是 `{kind, code, message, error}`，cause 的规则不变。#293 和 #349 都改 `aimux-ops/src/envelope.rs`，后合并的一个解决冲突。

## 12. 实施

三个 PR，一个叠一个，最底下一个（#289）基于 #268 合并后的 master，用它的生成器：

1. **核心类型和网络层**（#289）：`ErrorCause`、`SourceError`、`ApiCallError.cause`、`source()`、`FetchError` 错误链去 URL、SSE / WebSocket / 响应处理器、生成的 DTO 和向量、文档登记。
2. **字符串变体**（#290）：六个变体改形，117 处补上原因，`InvalidToolInput.cause` 改成错误对象，新增 `TypeValidation`，下载错误去 URL。
3. **ops 和绑定**（#293）：§5.1 四个域、8 种绑定的原生 cause、C++ `NativeError`。

每个 PR 带自己的测试：Rust 的 `source()` 链和 JSON 往返（有、无 cause）、真实被拒连接（链完整、没有 URL）、DTO 向量、每种绑定一个 cause 测试。#280 是这三步的原型，拆分后关闭。

## 13. 不做

- 不加 `GatewayError`、`StreamProviderError`。
- 不改 C ABI 函数。
- 不把宿主异常写进信封。
- 除去掉 URL 外不改消息文字。
