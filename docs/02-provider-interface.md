# Vercel AI SDK Provider 接口设计

> 参考源码：`reference/ai/packages/provider/` 与 `reference/ai/packages/provider-utils/`
> 复核状态：✅ 已复核（详见文末复核记录）

> ⚠️ 重要：仓库中不存在 `LanguageModelV1`，接口已演进到 **V4**。所有路径以 V4 为准。

## LanguageModelV4 接口

定义于 [packages/provider/src/language-model/v4/language-model-v4.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4.ts) L8-61：

```ts
type LanguageModelV4 = {
  readonly specificationVersion: 'v4';        // L12
  readonly provider: string;                   // L14
  readonly modelId: string;                    // L16
  supportedUrls: PromiseLike<Record<string, RegExp[]>> | Record<string, RegExp[]>;  // L36-38
  doGenerate(options: LanguageModelV4CallOptions): PromiseLike<LanguageModelV4GenerateResult>;
  doStream(options: LanguageModelV4CallOptions): PromiseLike<LanguageModelV4StreamResult>;
};
```

`do` 前缀防止用户直接调用。

### ProviderV4 工厂接口

[packages/provider/src/provider/v4/provider-v4.ts](../reference/ai/packages/provider/src/provider/v4/provider-v4.ts) L13：

| 方法 | 可选性 | 行号 |
|------|--------|------|
| `languageModel(id)` | 必选 | L26 |
| `embeddingModel(id)` | 必选 | L38 |
| `imageModel(id)` | 必选 | L48 |
| `transcriptionModel?(id)` | 可选 | L58 |
| `speechModel?(id)` | 可选 | L68 |
| `rerankingModel?(id)` | 可选 | L80 |
| `files?` | 可选 | L88 |
| `skills?` | 可选 | L94 |

## CallOptions 字段

[language-model-v4-call-options.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-call-options.ts) L8：

| 字段 | 类型 | 行号 | 备注 |
|------|------|------|------|
| `prompt` | `LanguageModelV4Prompt` | L17 | 标准化提示，非用户面 prompt |
| `maxOutputTokens?` | `number` | L22 | |
| `temperature?` | `number` | L27 | |
| `stopSequences?` | `string[]` | L34 | |
| `topP?` | `number` | L39 | |
| `topK?` | `number` | L47 | |
| `presencePenalty?` | `number` | L53 | |
| `frequencyPenalty?` | `number` | L59 | |
| `responseFormat?` | `{type:'text'}` \| `{type:'json',schema?,name?,description?}` | L66 | |
| `seed?` | `number` | L91 | |
| `tools?` | `Array<LanguageModelV4FunctionTool \| LanguageModelV4ProviderTool>` | L96 | |
| `toolChoice?` | `LanguageModelV4ToolChoice` | L101 | 默认 `'auto'` |
| `includeRawChunks?` | `boolean` | L106 | 仅 stream |
| `abortSignal?` | `AbortSignal` | L111 | |
| `headers?` | `Record<string, string \| undefined>` | L117 | |
| `reasoning?` | `'provider-default'\|'none'\|'minimal'\|'low'\|'medium'\|'high'\|'xhigh'` | L123 | |
| `providerOptions?` | `SharedV4ProviderOptions` | L137 | 透传 provider 专属参数 |

## GenerateResult

[language-model-v4-generate-result.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-generate-result.ts) L11：

| 字段 | 类型 | 行号 |
|------|------|------|
| `content` | `Array<LanguageModelV4Content>` | L15 |
| `finishReason` | `LanguageModelV4FinishReason` | L20 |
| `usage` | `LanguageModelV4Usage` | L25 |
| `providerMetadata?` | | L32 |
| `request?.body?` | | L37-42 |
| `response?` | 含 `headers?`/`body?` + ResponseMetadata | L47-57 |
| `warnings` | `Array<SharedV4Warning>` | L62 |

### Content 变体

[language-model-v4-content.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-content.ts) L11，联合体共 9 个 variant：

`Text` / `Reasoning` / `CustomContent` / `ReasoningFile` / `File` / `ToolApprovalRequest` / `Source` / `ToolCall` / `ToolResult`

### FinishReason

[language-model-v4-finish-reason.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-finish-reason.ts) L8：

- `unified`: `'stop'` | `'length'` | `'content-filter'` | `'tool-calls'` | `'error'` | `'other'`（L20-26）
- `raw`: `string | undefined`（L32，**必填字段，可为 undefined**，非可选属性 `raw?:`）

### Usage

[language-model-v4-usage.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-usage.ts) L6：

- `inputTokens`: `{ total, noCache, cacheRead, cacheWrite }`（L10-30）
- `outputTokens`: `{ total, text, reasoning }`（L35-50）
- `raw?`: `JSONObject`（L58）

## StreamResult / StreamPart

### StreamResult

[language-model-v4-stream-result.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-stream-result.ts) L7：

| 字段 | 类型 |
|------|------|
| `stream` | `ReadableStream<LanguageModelV4StreamPart>` |
| `request?.body?` | |
| `response?.headers?` | |

### StreamPart 类型

[language-model-v4-stream-part.ts](../reference/ai/packages/provider/src/language-model/v4/language-model-v4-stream-part.ts) L14，完整 variant 列表：

| 类型 | 行号 |
|------|------|
| `text-start` / `text-delta` / `text-end` | L16-31 |
| `reasoning-start` / `reasoning-delta` / `reasoning-end` | L34-49 |
| `tool-input-start` / `tool-input-delta` / `tool-input-end` | L52-71 |
| `tool-approval-request` | L72 |
| `tool-call` | L73 |
| `tool-result` | L74 |
| `custom-content` | L75 |
| `file` | L78 |
| `reasoning-file` | L79 |
| `source` | L80 |
| `stream-start`（带 warnings） | L83-86 |
| `response-metadata` | L90 |
| `finish`（usage + finishReason） | L93-98 |
| `raw` | L101-104 |
| `error` | L107-110 |

## Provider 工具层

[packages/provider-utils/src/](../reference/ai/packages/provider-utils/src/)

### HTTP 调用

- `postJsonToApi` / `postFormDataToApi` / `postToApi` — [post-to-api.ts](../reference/ai/packages/provider-utils/src/post-to-api.ts)（L14/L47/L77）
- `getFromApi` — [get-from-api.ts](../reference/ai/packages/provider-utils/src/get-from-api.ts) L16
- 统一注入 `ai-sdk/provider-utils/<VERSION>` UA，失败走 `failedResponseHandler`，异常包成 `APICallError`

### Fetch 抽象

- `FetchFunction = typeof fetch` — [fetch-function.ts](../reference/ai/packages/provider-utils/src/fetch-function.ts) L4
- `fetchWithValidatedRedirects` + `validateUrl`/`credentialedOrigin`/`trustedOrigin` 防 SSRF — fetch-with-validated-redirects.ts

### SSE 解析

- `parseJsonEventStream` — [parse-json-event-stream.ts](../reference/ai/packages/provider-utils/src/parse-json-event-stream.ts) L11
- 基于 `eventsource-parser` 的 `EventSourceParserStream`，自动跳过 `[DONE]`（L25）

### 响应处理器工厂

[response-handler.ts](../reference/ai/packages/provider-utils/src/response-handler.ts)：

| 工厂 | 行号 |
|------|------|
| `createJsonErrorResponseHandler` | L35 |
| `createEventSourceResponseHandler` | L101 |
| `createJsonResponseHandler` | L121 |
| `createBinaryResponseHandler` | L152 |
| `createStatusCodeErrorResponseHandler` | L187 |

### 鉴权 / 配置

- `loadApiKey` — [load-api-key.ts](../reference/ai/packages/provider-utils/src/load-api-key.ts) L3（参数→环境变量回退）
- `loadSetting` / `loadOptionalSetting` — load-setting.ts:12 / load-optional-setting.ts:8
- `validateBaseURL` — validate-base-url.ts:3
- `withoutTrailingSlash` — without-trailing-sl/without-trailing-slash.ts:1

### Schema

- `jsonSchema`(L104) / `zodSchema`(L274) / `asSchema`(L141) / `lazySchema`(L53) — schema.ts
- `to-json-schema/` 子目录：zod→JSON Schema 转换

### 流式工具

- `StreamingToolCallTracker` — streaming-tool-call-tracker.ts:77（按 index 累积 tool call arguments）
- `convertAsyncIteratorToReadableStream` — convert-async-iterator-to-readable-stream.ts:8
- `delay` / `DelayedPromise` — delay.ts:8 / delayed-promise.ts:6
- `createIdGenerator` — generate-id.ts:13

---

## 复核记录

**复核员**：Eve | **复核方式**：逐条打开源码验证行号

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | language-model-v4.ts spec v4 / 行 8-61 / supportedUrls | ✅ | L8 类型开始，L61 闭合，L12 specVersion='v4'，L36-38 supportedUrls |
| 2 | provider-v4.ts:13 ProviderV4 工厂 | ✅ | 6 必选 + 2 可选工厂，行号全部精确 |
| 3 | call-options 15 个字段行号 | ✅ | 全部 15 个字段行号精确吻合 |
| 4 | generate-result.ts:11 + content.ts:11 变体 | ✅ | content 联合体 9 个 variant |
| 5 | finish-reason.ts:8 unified 枚举 | ✅（已修正） | 枚举值完整准确；`raw` 修正为 `raw: string \| undefined`（必填可空）非 `raw?: string` |
| 6 | usage.ts:6 结构 | ✅ | inputTokens/outputTokens 细分 cache，全部吻合 |
| 7 | stream-part.ts:14 类型列表 | ✅（已修正） | 原报告遗漏 3 个 variant，已补充 tool-approval-request/custom-content/reasoning-file |
| 8 | provider-utils 5 处行号导出 | ✅ | post-to-api/get-from-api/fetch-function/parse-json-event-stream/load-api-key 行号全部精确 |
| 9 | response-handler 5 个工厂 | ✅ | 5 个工厂全部存在，行号精确 |
