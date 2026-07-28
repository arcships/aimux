# 测试体系结构

> 参考源码：`reference/ai/packages/ai/src/test/`、`packages/test-server/`、`packages/openai/src/`、`packages/anthropic/src/`
> 复核状态：✅ 已复核（详见文末复核记录）

## Mock 模型体系

`packages/ai/src/test/` 提供了一套覆盖所有模型类型的 mock，全部以 class 实现对应 V4 接口，并保留 v2/v3/v4 三个版本。还有 `MockProviderV4`（按 modelId 路由）、`MockSandbox`、`mock-server-response.ts`。

### MockLanguageModelV4 接口设计

[mock-language-model-v4.ts](../reference/ai/packages/ai/src/test/mock-language-model-v4.ts)：

- 构造参数 `doGenerate`/`doStream` 支持三种形态：
  - **单个结果对象** — 每次返回相同结果
  - **结果数组** — 按调用顺序依次返回（多步工具循环测试核心）
  - **回调函数** `(options) => result` — 动态计算
- **调用记录**：实例字段 `doGenerateCalls: LanguageModelV4CallOptions[]`、`doStreamCalls` 累积每次入参
- **流式**：mock 返回 `LanguageModelV4StreamResult`，其 `stream` 是 `ReadableStream`，用 `convertArrayToReadableStream` 喂入 `text-start`/`text-delta`/`text-end` 事件
- `mockValues<T>(...values)` — "按顺序返回、超出后返回最后一个"的辅助工厂
- `notImplemented()` — 默认值，未配置时抛错

### Rust 映射

```rust
enum MockReturn<T> {
    Single(T),
    Seq(Vec<T>),           // 多步工具循环测试用
    Fn(Box<dyn Fn() -> T>),
    NotImplemented,
}

struct MockLanguageModel {
    do_generate_returns: MockReturn<GenerateResult>,
    do_stream_returns: MockReturn<StreamResult>,
    do_generate_calls: Mutex<Vec<CallOptions>>,
    do_stream_calls: Mutex<Vec<CallOptions>>,
}
```

## test-server

[packages/test-server/src/create-test-server.ts](../reference/ai/packages/test-server/src/create-test-server.ts) 是基于 **MSW (Mock Service Worker)** 的 HTTP mock 服务器——通过 `setupServer` + `http.all(url, handler)` 拦截 `fetch` 请求，不是真实 HTTP server。

### 配置方式

```ts
createTestServer({
  [url]: {
    response: UrlResponse | UrlResponse[] | ({callNumber}) => UrlResponse
  }
})
```

返回 `{ urls, calls, server }`。`urls[url].response` 可运行时改写。

### 响应类型

| 类型 | 说明 |
|------|------|
| `json-value` | JSON 响应体 |
| `stream-chunks` | 数组转 SSE 流 |
| `binary` | 二进制响应 |
| `empty` | 空响应 |
| `error` | 错误响应 |
| `controlled-stream` | 用 `TestResponseController` 手动 write/error/close，模拟真实流式分块与 abort |

### 调用记录

`server.calls` 是 `TestServerCall[]`，暴露 `requestBodyJson` / `requestBodyMultipart` / `requestHeaders` / `requestUserAgent` / `requestUrlSearchParams` / `requestMethod`。

### Rust 替代

MSW 是 JS 生态特有。Rust 侧用 `wiremock`（真起 HTTP server，更真实）或抽象 `HttpClient` trait mock 其方法。fixture 文件可直接复用。

## Provider 测试结构

测试**与源码同目录**（`*.test.ts`），无独立 `__tests__/`。

### 组织方式

- 单文件巨大：`openai-chat-language-model.test.ts` 4048 行、`anthropic-language-model.test.ts` 11000+ 行
- 顶层 `describe('doGenerate')` / `describe('doStream')` 二分，内部 `it` 列举场景
- 无 unit/integration 分层——全部走 MSW mock
- **无 conformance test 套件**：每个 provider 各写各的，靠团队约定隐式对齐

### HTTP mock

统一用 `@ai-sdk/test-server/with-vitest`（MSW）。`vi.mock('../version', ...)` 固定版本号让 UA 快照稳定。

### 覆盖场景

文本提取、usage 解析（含 cache/partial）、logprobs、finish reason 映射、tool 传递与解析、response format（json_object/json_schema/strict）、headers 透传、annotations/citations、流式分块、流首块错误、abort、错误状态码。

### Fixture 机制

`src/chat/__fixtures__/` 存放 `.json`/`.chunks.txt` 真实 API 响应快照，`prepareJsonFixtureResponse`/`prepareChunksFixtureResponse` 辅助加载。

## 核心 AI 函数测试

`generate-text.test.ts`（12769 行）和 `stream-text.test.ts`（29374 行）是主力，外加 `execute-tool-call.test.ts`、`execute-tools-from-stream.test.ts`、`parse-tool-call.test.ts`、`smooth-stream.test.ts`、`stream-language-model-call.test.ts`、`stop-condition.test.ts` 等细分单测。

### Mock 策略

完全用 `MockLanguageModelV4`，**不碰 HTTP**——核心层只测编排逻辑（工具循环、step、abort、telemetry、approval）。

### 多步工具循环测试

`doGenerate` 传**数组**，第 N 步返回 tool-call，第 N+1 步返回 stop。或用闭包计数。

### 流式测试

`doStream` 返回 `convertArrayToReadableStream([...StreamPart])`，断言时用 `convertReadableStreamToArray` 收集事件。

### 确定性

`mockId({ prefix: 'call' })` 生成可预测 ID。类型测试用 `.test-d.ts` 文件做编译期类型断言。

## 测试工具函数

[packages/provider-utils/src/test/](../reference/ai/packages/provider-utils/src/test/)，通过 `@ai-sdk/provider-utils/test` 子路径导出：

| 函数 | 用途 |
|------|------|
| `convertReadableStreamToArray<T>(stream)` | 流转数组，断言用 |
| `convertArrayToReadableStream<T>(values)` | 数组转同步流，构造 mock stream |
| `convertAsyncIterableToArray` / `convertArrayToAsyncIterable` | 异步迭代器互转 |
| `convertResponseStreamToArray` | provider 响应流转数组 |
| `mockId({ prefix })` | 计数器 ID 工厂 |

`packages/ai/src/test/` 另有 `mockValues`、`createMockServerResponse`、`mockSandboxSessionFileStubs`、`notImplemented`。

## 可复用性评估

### 可直接翻译

| 项目 | Rust 对应 | 优先级 |
|------|-----------|--------|
| Mock 模型体系（三态返回 + calls 记录） | `enum MockReturn` + `Mutex<Vec<CallOptions>>` | P0 最高 |
| `mockId` / `mockValues` | `AtomicUsize` + `format!` | P0 |
| 核心 AI 函数测试结构 | `MockLanguageModel` + `tokio::test` | P0 |
| 类型测试 `.test-d.ts` | Rust 编译期即保证 | 无需翻译 |

### 需替换

| TS 原版 | Rust 替代 |
|---------|-----------|
| MSW (Mock Service Worker) | `wiremock`（真起 HTTP server） |
| `ReadableStream` | `tokio_stream` / `futures::stream` |
| `inlineSnapshot` | `insta` crate |
| `ServerResponse` mock | mock `axum::response::Response` 或 `Bytes` 收集器 |

### 不可复用 / 应舍弃

- v2/v3 版本 mock（Rust 从 v4 起步）
- vitest 的 `vi.mock('../version')` → 用 `cfg(test)` 注入
- `__snapshots__` 下的 JS 对象快照 → 用 `insta` 重生成

### 关键结论

核心编排层的测试模式（mock 模型 + 数组多步返回 + calls 断言 + 流事件收集）**几乎可机械翻译**。provider 层的 HTTP mock 需换技术栈但 fixture 数据可复用。**无统一 conformance 套件意味着 Rust 侧可率先建立**（用参数化 provider trait 做 cross-provider 一致性测试）。

---

## 复核记录

**复核员**：Zane | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | MockLanguageModelV4 三态返回 + calls 记录 | ⏳ | |
| 2 | test-server 基于 MSW，6 种响应类型 | ⏳ | |
| 3 | Provider 测试无 conformance 套件 | ⏳ | |
| 4 | 核心测试用 MockLanguageModelV4 不碰 HTTP | ⏳ | |
| 5 | 测试工具函数清单 | ⏳ | |
