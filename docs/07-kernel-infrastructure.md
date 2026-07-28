# 内核基础设施

> 参考源码：`reference/ai/packages/ai/src/` 下 prompt/registry/telemetry/error/realtime/text-stream/util
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. prompt/ — Prompt → CallOptions 转换管道

所有 generate-* 调用前的必经管道，分四阶段。

### 标准化

[standardize-prompt.ts:34](../reference/ai/packages/ai/src/prompt/standardize-prompt.ts#L34) `standardizePrompt` 接受 `prompt`（string 或 messages 数组）与 `instructions`（别名 `system`）：

- 互斥校验：`prompt` 与 `messages` 不能同传、不能都缺（L41-53）
- 校验 instructions 必须是 string 或全 system 角色数组（L56-65）
- string prompt 包成单条 user message（L67-76）
- `allowSystemInMessages=false` 则禁止 messages 含 system（L85-94，强制走 instructions）
- 用 `safeValidateTypes` 配合 [message.ts:67](../reference/ai/packages/ai/src/prompt/message.ts#L67) 的 `modelMessageSchema` 做结构校验，失败抛 `InvalidPromptError`

### 下载与转换

[convert-to-language-model-prompt.ts:36](../reference/ai/packages/ai/src/prompt/convert-to-language-model-prompt.ts#L36)：

- `downloadAssets`（L442）并行下载模型不支持的 URL 文件为 Uint8Array
- 构建 `approvalIdToToolCallId` 映射处理 tool-approval 流程（L55-85）
- instructions 前置为 system 消息，逐条 `convertToLanguageModelMessage`（L191）转成 `LanguageModelV4Message`
- user 内容统一映射为 `file` part（legacy `image` part 经 `convertImagePartToFilePart` L425 改写）
- assistant 过滤空 text/tool-approval-request 并归一 tool-result 输出（`mapToolResultOutput` L608）
- 合并连续 tool 消息（L103-128）
- tool-call/tool-result 配对完整性检查：未配对抛 `MissingToolResultsError`（L158, L172）

### Tools 准备

[prepare-tools.ts:15](../reference/ai/packages/ai/src/prompt/prepare-tools.ts#L15)：

- 按 `toolOrder` 排序（先 ordered 后按名 alphabetical 的 unordered，L82-106）
- 每个 tool 按 type 分派：`function`/`dynamic`/undefined → `LanguageModelV4FunctionTool`（description 支持函数式动态计算 L108-127）；`provider` 直接转
- **ToolChoice**（[prepare-tool-choice.ts:4](../reference/ai/packages/ai/src/prompt/prepare-tool-choice.ts#L4)）：null→`{type:'auto'}`，string→`{type:toolChoice}`，object→`{type:'tool',toolName}`

### CallOptions 验证

[prepare-language-model-call-options.ts:7](../reference/ai/packages/ai/src/prompt/prepare-language-model-call-options.ts#L7) 逐字段类型/范围校验（maxOutputTokens ≥1 整数、temperature/topP/topK 为 number、seed 整数），失败抛 `InvalidArgumentError`。

[data-content.ts](../reference/ai/packages/ai/src/prompt/data-content.ts) 提供 base64/Uint8Array/ArrayBuffer 互转（L14, L32），非合法 base64 抛 `InvalidDataContentError`。

## 2. registry/ — 模型解析与 provider 聚合

### 字符串 id 解析

[model/resolve-model.ts:29](../reference/ai/packages/ai/src/model/resolve-model.ts#L29)：`resolveLanguageModel` 检测到 `typeof model === 'string'` 时调用 `getGlobalProvider()`（L181），取 `globalThis.AI_SDK_DEFAULT_PROVIDER`，缺省回退到 `@ai-sdk/gateway`（[global.ts:15](../reference/ai/packages/ai/src/global.ts#L15)）。经 `asProviderV4` 规范化后调 `languageModel(modelId)`。

### customProvider

[custom-provider.ts:52](../reference/ai/packages/ai/src/registry/custom-provider.ts#L52) 聚合多类模型表 + 可选 `fallbackProvider`。三段式解析：本地表命中→返回；否则 fallbackProvider→返回；否则抛 `NoSuchModelError`（L122-221）。`ExtractModelId`（L254）把 keys 收窄为字面量联合，使 `languageModel('gpt-4')` 编译期校验。

### createProviderRegistry

[provider-registry.ts:137](../reference/ai/packages/ai/src/registry/provider-registry.ts#L137) `createProviderRegistry` 返回 `DefaultProviderRegistry`（类定义在 L175），用 `${providerId}${separator}${modelId}`（默认 `:`）复合 id。可选 `languageModelMiddleware`/`imageModelMiddleware` 在取模型时通过 `wrapLanguageModel`/`wrapImageModel` 包裹（L272, L312）。

## 3. telemetry/ — 内核钩子（区别于 @ai-sdk/otel 导出器）

### Telemetry 接口

[telemetry.ts:106](../reference/ai/packages/ai/src/telemetry/telemetry.ts#L106)：`onStart/onStepStart/onLanguageModelCallStart/.../onEnd/onError` 生命周期回调 + `executeLanguageModelCall`/`executeTool`（L250, L266）让集成在受控上下文里运行模型/工具调用以建立嵌套 span 父子关系。

### TelemetryDispatcher

[create-telemetry-dispatcher.ts:67](../reference/ai/packages/ai/src/telemetry/create-telemetry-dispatcher.ts#L67)：解析集成——per-call `telemetry.integrations` 覆盖 `getGlobalTelemetryIntegrations()`（[telemetry-registry.ts:13](../reference/ai/packages/ai/src/telemetry/telemetry-registry.ts#L13)，读 `globalThis.AI_SDK_TELEMETRY_INTEGRATIONS`，由 `registerTelemetry` L6 写入）。`isEnabled===false` 返回空派发器（L75）。`executeLanguageModelCall`/`executeTool`（L174, L196）以洋葱圈方式层层包裹 execute。

### tracing-channel 机制

[tracing-channel-publisher.ts:66](../reference/ai/packages/ai/src/telemetry/tracing-channel-publisher.ts#L66) 按需懒加载 `node:diagnostics_channel`（非 Node 运行时直接 passthrough，L29）。无订阅者时直接执行（L75）。`tracePromise` 包裹 execute 并保存 result/error。

### 与 @ai-sdk/otel 的关系

| 维度 | telemetry/（内核） | @ai-sdk/otel（导出器） |
|------|-------------------|----------------------|
| 定位 | provider-agnostic 内核钩子 + diagnostics-channel 发布点 | 订阅 `ai:telemetry` channel 并导出 OTLP span |
| 依赖 | 不依赖 OpenTelemetry SDK | 依赖 OTLP |

## 4. error/ — 自定义错误类型

[index.ts](../reference/ai/packages/ai/src/error/index.ts#L1) 统一导出。全部继承 `@ai-sdk/provider` 的 `AISDKError`，用 `Symbol.for('vercel.ai.error.AI_XxxError')` marker + 静态 `isInstance`/`hasMarker` 实现 cross-realm instanceof。

| 分类 | 错误类型 |
|------|----------|
| **从 provider 包透传** | `AISDKError`、`APICallError`、`EmptyResponseBodyError`、`InvalidPromptError`、`InvalidResponseDataError`、`JSONParseError`、`LoadAPIKeyError`、`LoadSettingError`、`NoContentGeneratedError`、`NoSuchModelError`、`NoSuchProviderReferenceError`、`TooManyEmbeddingValuesForCallError`、`TypeValidationError`、`UnsupportedFunctionalityError`、`DownloadError`、`RetryError` |
| **参数/输入** | `InvalidArgumentError`、`InvalidToolInputError`、`InvalidDataContentError`、`InvalidMessageRoleError`、`MessageConversionError` |
| **Tool 流程** | `NoSuchToolError`、`MissingToolResultsError`、`ToolCallRepairError`、`ToolCallNotFoundForApprovalError`、`InvalidToolApprovalError`、`InvalidToolApprovalSignatureError` |
| **空输出** | `NoImageGeneratedError`、`NoObjectGeneratedError`、`NoOutputGeneratedError`、`NoSpeechGeneratedError`、`NoTranscriptGeneratedError`、`NoVideoGeneratedError` |
| **流/版本** | `InvalidStreamPartError`、`UIMessageStreamError`、`UnsupportedModelVersionError` |

## 5. realtime/ — 实时会话运行时

### AbstractRealtimeSession

[realtime-session.ts:34](../reference/ai/packages/ai/src/realtime/realtime-session.ts#L34) 编排三件套：`RealtimeEventReducer`（状态机）、`BrowserRealtimeTransport`（WS）、`BrowserRealtimeAudio`（音频）。

- `connect`（L106）先 POST 换 token+url+toolDefinitions，再开 WS，open 后发 `session-update`
- 所有 server event 走 `handleServerEvent`（L324）→ reducer.reduce → `applyState` diff 派发 setState → 处理 effects
- 多工具回合协调（L221-233）：用 `toolCallsInResponse`/`submittedToolOutputs`/`responseToolCallsClosed` 三集合，等 `response-done` 且全部 output 提交后只发一次 `response-create`
- `executeToolCall`（L290）支持 human-in-the-loop：onToolCall 返回 undefined 则不自动提交

### RealtimeEventReducer

[realtime-event-reducer.ts:60](../reference/ai/packages/ai/src/realtime/realtime-event-reducer.ts#L60) 状态机：`reduceServerEvent`（L115）按 event.type 分派，维护 `currentAssistantMessageId`/`textAccumulators`/`toolArgAccumulators` 把流式 delta 累积成 `UIMessage[]`。`function-call-arguments-delta` 累积字符串，`-done` 时 `safeParseJSON`。`audio-delta` 不入 state 而发 `play-audio` effect。返回 `{state, effects}` 解耦纯状态与副作用。

### BrowserRealtimeTransport

[browser-realtime-transport.ts:15](../reference/ai/packages/ai/src/realtime/browser-realtime-transport.ts#L15) 封装 WebSocket：`sendEvent`（L78）串行化到 `sendQueue` Promise 链保序，先 `model.serializeClientEvent` 再 sendRaw。

## 6. text-stream/ — stitchable stream 透传

三函数都极薄：

| 函数 | 文件 | 机制 |
|------|------|------|
| `toTextStream` | [to-text-stream.ts:7](../reference/ai/packages/ai/src/text-stream/to-text-stream.ts#L7) | `TransformStream` 过滤只透传 `text-delta` 的 `part.text`，丢弃 reasoning/tool |
| `createTextStreamResponse` | [create-text-stream-response.ts:15](../reference/ai/packages/ai/src/text-stream/create-text-stream-response.ts#L15) | text 流 `pipeThrough(TextEncoderStream)` 转 UTF-8 字节流作 Response body，注入 `content-type: text/plain; charset=utf-8` |
| `pipeTextStreamToResponse` | [pipe-text-stream-to-response.ts:18](../reference/ai/packages/ai/src/text-stream/pipe-text-stream-to-response.ts#L18) | Node `ServerResponse` 版本 |

不缓冲、不解码，保证流式语义透传。

## 7. util/ — 关键工具函数

| 函数 | 文件 | 用途 |
|------|------|------|
| `parsePartialJson` | [parse-partial-json.ts:5](../reference/ai/packages/ai/src/util/parse-partial-json.ts#L5) | 流式 JSON 容错解析：先 safeParseJSON，失败再 fixJson 修复后重试，返回 `{value, state}` |
| `fixJson` | [fix-json.ts:28](../reference/ai/packages/ai/src/util/fix-json.ts#L28) | 单遍线性扫描状态机（16 个 State），栈跟踪嵌套，扫描结束从 lastValidIndex 截断并按栈补全 `"`/`}`/`]`/literal。专治 LLM 流式截断 JSON |
| `cosineSimilarity` | [cosine-similarity.ts:14](../reference/ai/packages/ai/src/util/cosine-similarity.ts#L14) | 单遍点积+模平方，零向量返回 0，长度不等抛 `InvalidArgumentError` |
| `retryWithExponentialBackoffRespectingRetryHeaders` | [retry-with-exponential-backoff.ts:64](../reference/ai/packages/ai/src/util/retry-with-exponential-backoff.ts#L64) | 优先读 `retry-after-ms`/`retry-after`（秒或 HTTP 日期，0-60s 内才采用），否则指数退避；仅对 `isRetryable` 的 `APICallError`/`GatewayError` 重试 |
| `mergeAbortSignals` | [merge-abort-signals.ts:13](../reference/ai/packages/ai/src/util/merge-abort-signals.ts#L13) | 混合 AbortSignal/number（timeout）归一，0/1 个直接返回，多个用 `AbortSignal.any` |
| `simulateReadableStream` | [simulate-readable-stream.ts:12](../reference/ai/packages/ai/src/util/simulate-readable-stream.ts#L12) | pull-based ReadableStream 按序发 chunks，首块前/块间可控 delay |
| `canonical-hash` | [canonical-hash.ts:38](../reference/ai/packages/ai/src/util/canonical-hash.ts#L38) | `canonicalJSON` 递归排序 key 产出确定性序列化，`hashCanonical` 用 SHA-256 + base64url。用于内容寻址/缓存键 |

---

## 复核记录

**复核员**：Quinn | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | standardizePrompt L34，互斥校验 L41-53 | ✅（已修正） | 函数@L34 ✅；互斥字段修正为 **prompt/messages**（非 prompt/instructions） |
| 2 | convert-to-language-model-prompt L36，downloadAssets L442 | ✅（已修正） | 函数@L36 ✅；downloadAssets@L442 ✅；第二处 MissingToolResultsError 修正为 **L172**（非 L171，L171 是 if 条件行） |
| 3 | prepare-tools L15，prepare-tool-choice L4 | ✅ | prepareTools@L15 ✅；三种映射 null→auto/string→type/object→tool+toolName@L10-14 全部精确 |
| 4 | resolve-model L29，getGlobalProvider L181，global.ts L15 | ✅ | resolveLanguageModel@L29 ✅；getGlobalProvider@L181 ✅；global.ts@L15 声明 AI_SDK_DEFAULT_PROVIDER，缺省回退 @ai-sdk/gateway（global.ts 在 src/ 根目录） |
| 5 | customProvider L52，provider-registry L137 | ✅（已修正） | customProvider@L52 ✅；L137 是 createProviderRegistry，DefaultProviderRegistry 类定义修正为 **L175** |
| 6 | telemetry L106，create-telemetry-dispatcher L67 | ✅ | Telemetry 接口@L106 ✅；createTelemetryDispatcher@L67 ✅ |
| 7 | tracing-channel-publisher L66 | ✅ | 懒加载 node:diagnostics_channel@L26-40 ✅；非 Node passthrough@L29 ✅ |
| 8 | realtime-session L34，event-reducer L60，transport L15 | ✅ | 三者行号全部精确 |
| 9 | fixJson 17 个 State L28，parsePartialJson L5 | ✅（已修正） | fixJson@L28 ✅；State 数量修正为 **16**（非 17）；parsePartialJson@L5 ✅ |
