# 错误模型与兼容性约定

> 本文记录错误模型迁移后的稳定公开契约。本次迁移本身包含破坏性变更；
> 此后的演进应保持向后兼容，否则需要按主版本变更处理。

## 三类错误

| 来源 | 对外形态 |
|---|---|
| 核心操作 | `AiMuxError` 及各语言的原生子类型 |
| 录制 | 独立的 `RecordingError`，不属于 `AiMuxError` |
| 绑定边界 | 各语言的参数、状态或运行时错误；C ABI 使用 `200..206` |

三类错误不相互伪装，也不再用序列化后的错误 JSON 作为第二套协议。

## 各语言的映射

- Node.js 和 Python 抛出真实的原生错误实例，可直接使用 `instanceof` 和
  `isinstance`。
- C ABI 成功时返回 `NULL` 并写入出参；失败时返回一个由调用方释放一次的
  不透明 `aimux_error_t *`。
- Go、Java、Kotlin、Swift 和 Flutter 将 C 错误还原为本语言错误。错误码
  `1..20` 属于核心，`100..105` 属于录制，`200..206` 属于绑定边界。

结构化字段只放在真正拥有它的错误上。例如，HTTP 状态和响应头属于
`APICallError`，不放进通用基类（retry hint 与 request id 从 `response_headers` 读取）。

## 错误原因（cause）

AI SDK 构造时接受 `cause` 的错误，在 aimux 里都有可选的 `cause`；它就是 Rust
的 `Error::source()`，序列化后在错误 payload 的 `cause` 字段。`cause` 有两种：
带 `name` 的嵌套 aimux 错误，或者没有 `name` 的底层错误 `{message, cause?}`
（OS、TLS、JSON 解析器等），后者的 `cause` 继续往下一层。`Retry` 没有 `cause`
字段，它的 `source()` 是最后一次尝试（AI SDK 的 `lastError`），各语言的原生
cause 也取 `errors` 的最后一个。

| 语言 | 原生入口 |
|---|---|
| Rust | `source()` 链，`anyhow` / `{:#}` 可直接打印 |
| Node.js | 标准 `error.cause`：嵌套错误是 `AimuxCauseError`，底层错误是 `AimuxSourceError` |
| Python | `__cause__`：嵌套错误是对应子类，底层错误是 `SourceError` |
| Java / Kotlin | `getCause()` 链上的 `ErrorCause` |
| Go | `Unwrap()` 返回 `*aimux.Cause`，`errors.Unwrap` / `errors.As` 可用 |
| Swift / Flutter | `cause` 属性 |
| C / C++ | 错误 JSON 的 `error.cause`；C++ 的 `aimux::NativeError::json()` |

ops 信封的四个域用同一个形状：`aimux` 域是上面的错误 payload，`recording` 域的
`Init`、`OpenFile`、`Spawn` 带 I/O 错误，`binding` 域的 `InvalidWireJson`、
`ResultSerialization` 和 `protocol` 域的 `InvalidParams` 带 JSON 解析或序列化错误；
没有底层错误的变体没有 `cause`。

message 仍包含原来的底层错误文字，所以只看 message 的调用方不会少信息。

值不符合调用方给的 schema 时，cause 是 `TypeValidation`（`AI_TypeValidationError`，
带 `value`），和上游一样：工具输入不符合 `inputSchema` 时是
`InvalidToolInput.cause`，结构化输出不符合 schema 时是 `InvalidResponseData.cause`。
它和 `InvalidResponseData` 共用错误码 3。响应体本身形状不对仍是
`InvalidResponseData`。

## 兼容性约定

1. 已有错误名称和错误码不改名、不改号、不复用。
2. `aimux_error_t` 保持不透明，所有权和调用约定保持不变。
3. 新字段只加到对应错误；C ABI 通过新增 getter 扩展，不暴露结构体布局。
4. `RecordingError` 保持独立，绑定错误不冒充核心错误。
5. 调用方按原生类型、错误码或类型化字段判断，不解析错误消息。
6. 未知的非零 C 错误码视为头文件与动态库版本不匹配。

新增错误类型或可选字段属于兼容扩展；删除类型、改号、移动字段或改变所有权
属于破坏性变更。

## 参考方案

- [Vercel AI SDK](https://ai-sdk.dev/docs/reference/ai-sdk-errors)：类型化错误体系。
- [libsignal FFI](https://github.com/signalapp/libsignal/blob/main/rust/bridge/ffi/impl/src/error.rs)：不透明错误指针、getter 和显式释放。
- [Node-API](https://nodejs.org/api/n-api.html#napi_new_instance) 与
  [napi-rs](https://docs.rs/napi/latest/napi/bindgen_prelude/struct.Function.html#method.new_instance)：创建真实 JavaScript 实例。
- [PyO3](https://pyo3.rs/main/exception.html)：原生 Python 异常。
- [openai-go](https://github.com/openai/openai-go#errors)：`errors.As` 风格判断。

这些方案只作为设计参考，最终契约以 aimux 自身架构为准。
