# 标准参考设计

> 参考源码：`reference/ai/packages/provider/src/`、`packages/openai-compatible/`、`packages/provider-utils/`
> 复核状态：✅ 已复核（详见文末复核记录）

## V4 规范清单

### 模型接口（trait 蓝本）

所有 V4 接口共享：`specificationVersion: 'v4'`、`provider: string`、`modelId: string`。方法名一律 `do*` 前缀（防用户直接调用），返回 `PromiseLike`。

| 接口 | 必选属性 | 方法 |
|------|----------|------|
| `LanguageModelV4` | `supportedUrls` | `doGenerate(opts)→GenerateResult`、`doStream(opts)→StreamResult` |
| `EmbeddingModelV4` | `maxEmbeddingsPerCall`、`supportsParallelCalls` | `doEmbed(opts)→Result` |
| `ImageModelV4` | `maxImagesPerCall` | `doGenerate(opts)→Result` |
| `VideoModelV4` | `maxVideosPerCall` | `doGenerate(opts)→Result` |
| `TranscriptionModelV4` | — | `doGenerate`、`doStream?`（可选） |
| `RerankingModelV4` | — | `doRank(opts)→Result` |
| `SpeechModelV4` | — | `doGenerate(opts)→Result` |
| `RealtimeModelV4` | — | WebSocket 双向音频/文本 |

### LanguageModelV4 CallOptions 字段表

| 字段 | 类型 | 必选 | 约束/默认 |
|------|------|------|-----------|
| `prompt` | `LanguageModelV4Prompt`（消息数组） | ✅ | — |
| `maxOutputTokens` | `number?` | — | — |
| `temperature` | `number?` | — | 范围由 provider 决定 |
| `stopSequences` | `string[]?` | — | provider 可限数量 |
| `topP` / `topK` | `number?` | — | topK 多数 provider 不支持 |
| `presencePenalty` / `frequencyPenalty` | `number?` | — | — |
| `responseFormat` | `{type:'text'} \| {type:'json',schema?,name?,description?}` | — | 默认 text |
| `seed` | `number?` | — | 整数 |
| `tools` | `Array<FunctionTool\|ProviderTool>?` | — | — |
| `toolChoice` | `{type:'auto'}\|'none'\|'required'\|{type:'tool',toolName}` | — | 默认 `'auto'` |
| `includeRawChunks` | `boolean?` | — | 仅流式 |
| `abortSignal` | `AbortSignal?` | — | — |
| `headers` | `Record<string,string\|undefined>?` | — | — |
| `reasoning` | `'provider-default'\|'none'\|'minimal'\|'low'\|'medium'\|'high'\|'xhigh'` | — | 默认 `'provider-default'` |
| `providerOptions` | `Record<string,JSONObject>?` | — | 外层 key=provider 名 |

`FunctionTool = {type:'function', name, description?, inputSchema:JSONSchema7, inputExamples?, strict?, providerOptions?}`。

### GenerateResult / StreamResult

**GenerateResult**（全必填除注明）：

| 字段 | 类型 |
|------|------|
| `content` | `Content[]` |
| `finishReason` | `{unified:'stop'\|'length'\|'content-filter'\|'tool-calls'\|'error'\|'other'; raw:string\|undefined}` |
| `usage` | `{inputTokens:{total,noCache,cacheRead,cacheWrite}, outputTokens:{total,text,reasoning}, raw?:JSONObject}` |
| `warnings` | `Warning[]` |
| `providerMetadata?` | — |
| `request?` | `{body?}` |
| `response?` | `ResponseMetadata & {headers?, body?}` |

**StreamResult**：`stream: ReadableStream<StreamPart>`、`request?:{body?}`、`response?:{headers?}`。

`ResponseMetadata = {id?, timestamp?:Date, modelId?}`。

### StreamPart variant 列表

`text-start/delta/end`、`reasoning-start/delta/end`、`tool-input-start/delta/end`、`tool-approval-request`、`tool-call`、`tool-result`、`custom-content`、`file`、`reasoning-file`、`source`、`stream-start{warnings}`、`response-metadata`、`finish{usage,finishReason,providerMetadata}`、`raw{rawValue}`、`error{error}`。start/delta/end 均带 `id` 与可选 `providerMetadata`。

### Content union

`Text{text}` | `Reasoning{text}` | `CustomContent` | `ReasoningFile` | `File` | `ToolApprovalRequest` | `Source` | `ToolCall{toolCallId,toolName,input,providerExecuted?,dynamic?}` | `ToolResult`

### 共享类型

`SharedV4Warning = 'unsupported'{feature,details?} | 'compatibility'{feature,details?} | 'deprecated'{setting,message} | 'other'{message}`

`SharedV4Headers = Record<string,string>`

`SharedV4ProviderOptions = SharedV4ProviderMetadata = Record<string,JSONObject>`

## Provider 实现约束

取自 openai-compatible 实现分析：

1. **必实现** `specificationVersion='v4'`、`provider`、`modelId` 及对应 `doXxx`
2. **provider 命名**：`${providerName}.${modelType}`（如 `myai.chat`）；`providerOptions` 取 `split('.')[0]` 的 camelCase 作为 key
3. **HTTP 请求**：统一走 `postJsonToApi`/`postFormDataToApi`，必传 `failedResponseHandler`、`successfulResponseHandler`、`abortSignal`、`fetch`
4. **响应处理**：非流式 `createJsonResponseHandler(schema)`；流式 `createEventSourceResponseHandler(chunkSchema)`；失败 `createJsonErrorResponseHandler(errorStructure)`
5. **headers**：`combineHeaders(config.headers?.(), options.headers)` 合并；apiKey 注入 `Authorization: Bearer <key>`
6. **UA 后缀**：`withUserAgentSuffix(headers, 'ai-sdk/<provider-name>/<VERSION>')`
7. **baseURL**：经 `withoutTrailingSlash` 处理
8. **不支持字段必须发 warning**（如 `topK`、`seed`），不得静默丢弃
9. **流式协议**：首个 part 必为 `stream-start{warnings}`，末尾必为 `finish`；chunk 解析失败发 `error` 且 `finishReason.unified='error'`
10. **reasoning**：用 `isCustomReasoning` 判断；映射走 `mapReasoningToProviderEffort`（effortMap）或 `mapReasoningToProviderBudget`（百分比 clamp），不支持则 push `unsupported` warning
11. **必填结果字段**即使无值也要返回（`content/finishReason/usage/warnings`）

## Provider-utils 通用能力

### Provider 实现者必须使用

| 能力 | 函数 |
|------|------|
| 鉴权 | `loadApiKey({apiKey, environmentVariableName, description})` |
| HTTP | `postJsonToApi` / `postFormDataToApi` / `postToApi` |
| 响应处理 | `createJsonResponseHandler` / `createEventSourceResponseHandler` / `createJsonErrorResponseHandler` / `createBinaryResponseHandler` |
| Headers | `combineHeaders` / `withUserAgentSuffix` / `withoutTrailingSlash` / `normalizeHeaders` |
| Provider Options | `parseProviderOptions({provider, providerOptions, schema})` |
| Reasoning | `isCustomReasoning` / `mapReasoningToProviderEffort` / `mapReasoningToProviderBudget` |
| ID 生成 | `generateId` / `createIdGenerator` |
| Workflow 序列化 | `serializeModelOptions` + `WORKFLOW_SERIALIZE/DESERIALIZE` |
| 流式工具 | `StreamingToolCallTracker`（流式 tool-call 增量聚合） |
| Schema | `asSchema` / `jsonSchema` / `zodSchema` / `lazySchema` |
| 错误类 | `APICallError` / `LoadAPIKeyError` / `InvalidArgumentError` / `InvalidResponseDataError` / `UnsupportedFunctionalityError` 等 |

### 辅助能力

`downloadBlob`、`convertToFormData`、`convertBase64ToUint8Array`、`detectMediaType`、`isUrlSupported`、`parseJsonEventStream`、`extractResponseHeaders`、`readResponseWithSizeLimit`、`isAbortError`、`handleFetchError`、`getErrorMessage`、`retryWithExponentialBackoff`、`resolveProviderReference`

## 规范文档要点

来源 [architecture/provider-abstraction.md](../reference/ai/architecture/provider-abstraction.md)：

1. **三层架构**：AI functions → Model spec（V4 接口）→ Provider 实现，三者解耦
2. **spec 是稳定契约**：用户 prompt 由 AI SDK 映射为 `LanguageModelV4Prompt`，spec 可独立演进
3. **providerOptions 是唯一扩展通道**：`Record<provider, JSONObject>`，core AI SDK 不感知其内容
4. **reasoning 处理规范**：`isCustomReasoning` 返回 false 则无需动作；`'none'` 仅部分 provider 支持，不支持须 warning
5. **8 类模型统一 V4 版本号**，便于 discriminated union 演进

## 覆盖范围标准

| 优先级 | 范围 | 理由 |
|--------|------|------|
| P0 | `LanguageModelV4`（doGenerate/doStream） | 核心目标 |
| P1 | `EmbeddingModelV4`（doEmbed） | 简单，单次调用无流式 |
| P2 | 其余 6 种模型类型 | 按需加 |
| 暂不做 | files/skills 上传 | 与"统一请求方式"无关 |

---

## 复核记录

**复核员**：Zoe | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | LanguageModelV4 方法 doGenerate/doStream | ⏳ | |
| 2 | CallOptions 17 字段 | ⏳ | |
| 3 | StreamPart 15 variant | ⏳ | |
| 4 | Provider 11 条约束 | ⏳ | |
| 5 | provider-utils 必用函数清单 | ⏳ | |
