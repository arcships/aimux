# 核心 AI 函数实现机制

> 参考源码：`reference/ai/packages/ai/src/`
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. generate-text 多步工具循环

入口 [generate-text/generate-text.ts](../reference/ai/packages/ai/src/generate-text/generate-text.ts)。没有独立的 `run-tools-loop.ts`，循环内联在 `generateText` 主函数中。

### 停止条件

- 默认 `stopWhen = isStepCount(1)`（L240），即默认单步。多步需调用方显式传入 `stopWhen`，如 `isStepCount(5)` 或 `hasToolCall('done')`。
- 停止条件语义（[stop-condition.ts:6-13](../reference/ai/packages/ai/src/generate-text/stop-condition.ts#L6)）：循环持续直到以下任一发生——模型返回非 `tool-calls` 的 finish reason、调用了没有 `execute` 的工具、工具调用需要审批、或某个 `StopCondition` 返回 true。
- 内置工厂：`isStepCount`、`hasToolCall`、`isLoopFinished`；`isStopConditionMet` 用 `Promise.all` + `.some` 短路求值（L64-77）。

### 循环体

`do { ... } while`（L797-1375）：

1. 每步先调可选的 `prepareStep`（[prepare-step.ts:33](../reference/ai/packages/ai/src/generate-text/prepare-step.ts#L33)），允许按步覆盖 model / instructions / messages / activeTools / toolChoice / runtimeContext / toolsContext，覆盖值**沿用到后续步**。
2. 调 `model.doGenerate`，拿到 `currentModelResponse`。
3. 从响应里解析 `stepToolCalls`；逐个走 `resolveToolApproval`，需审批的进入 `toolApprovalRequests` 并加入 `blockedToolCallIds`（L1086-1161）。
4. 客户端工具调用（`!providerExecuted`）通过 `executeTools` 执行，过滤掉 invalid 与 blocked（L1194-1226）；结果转成 `tool-result` 消息并入 `messagesForNextStep`（L1350），喂回下一步模型。
5. provider-executed 且 `supportsDeferredResults` 的工具进入 `pendingDeferredToolCalls`，收到结果才删除（L1260-1283）。

### while 条件

L1365-1375：继续当且仅当（所有客户端工具调用都已执行或被拒）或（有待处理的 deferred 结果），**且** `!isStopConditionMet(...)`。无硬编码上限，最大步数由 `stopWhen` 控制。

streamText 的等价循环在 `stream-text.ts`，工具执行走 `execute-tools-from-stream.ts`（流式）。

## 2. generate-object 的 JSON 修复

### schema 注入

`generateObject` **不直接拼 prompt**，而是把 schema 通过 `responseFormat: { type: 'json', schema, name, description }` 传给 `model.doGenerate`（[generate-object.ts:388-402](../reference/ai/packages/ai/src/generate-object/generate-object.ts#L388)）。是否注入 prompt 由 provider 决定——不支持原生结构化输出的 provider 调用 `injectJsonInstruction` 把 schema + "You MUST answer with a JSON object..." 拼进 prompt（[inject-json-instruction.ts:8-30](../reference/ai/packages/ai/src/generate-object/inject-json-instruction.ts#L8)）。

### OutputStrategy

[output-strategy.ts:30-63](../reference/ai/packages/ai/src/generate-object/output-strategy.ts#L30)：四种策略 `object`/`array`/`enum`/`no-schema`，各实现 `jsonSchema()`、`validatePartialResult`、`validateFinalResult`、`createElementStream`。

- `array` 把元素包成 `{elements:[...]}` 对象（多数模型无法直接生成顶层数组，L142-155）
- `enum` 包成 `{result: string}`

### 流式部分 JSON 解析

[stream-object.ts:660-728](../reference/ai/packages/ai/src/generate-object/stream-object.ts#L660)：

- 每个 text-delta 累加到 `accumulatedText`，调 `parsePartialJson` 得到 `currentObjectJson` + `parseState`
- 与 `latestObjectJson` 用 `isDeepEqualData` 去重后，交给 `outputStrategy.validatePartialResult`
- 校验通过且与上次 partial 不同才发出 `object` 与 `text-delta` 事件
- `array` 策略对最后一个未完成元素跳过校验（L185-187），避免半截元素报错

### Zod 校验

`object` 策略 `validateFinalResult` 用 `safeValidateTypes({ value, schema })`（L120-124），partial 不做 Zod 校验（注释明说，L113）。

### 损坏 JSON 修复

[parse-and-validate-object-result.ts:77-111](../reference/ai/packages/ai/src/generate-object/parse-and-validate-object-result.ts#L77)：

`parseAndValidateObjectResultWithRepair` 先 `safeParseJSON`，失败抛 `NoObjectGeneratedError`；若调用方提供了 `repairText` 且错误是 `JSONParseError` 或 `TypeValidationError`，调 `repairText({text, error})` 拿回修复文本，再重新 parse+validate；返回 `null` 则抛原错误。`RepairTextFunction` 类型见 [repair-text.ts:9-12](../reference/ai/packages/ai/src/generate-object/repair-text.ts#L9)。

## 3. middleware 系统

### 注册机制

中间件**无全局注册表**，通过 `wrapLanguageModel({ model, middleware })` 显式包成新 model（[wrap-language-model.ts:25-42](../reference/ai/packages/ai/src/middleware/wrap-language-model.ts#L25)）。多个 middleware 先 `reverse()` 再 `reduce`，所以**数组第一个最先 transform 输入、最后包住输出**，最后一个最贴近裸 model（L37-41）。

### 可拦截的四件事

`LanguageModelMiddleware` 接口（L44-113）：

| 钩子 | 作用 |
|------|------|
| `transformParams` | 改请求参数 |
| `wrapGenerate` | 包 `doGenerate`（可前后处理） |
| `wrapStream` | 包 `doStream`（可前后处理） |
| `overrideProvider`/`overrideModelId`/`overrideSupportedUrls` | 覆盖 model 元信息 |

### 内置中间件

| 中间件 | 文件 | 机制 |
|--------|------|------|
| `defaultSettingsMiddleware` | [default-settings-middleware.ts:8-33](../reference/ai/packages/ai/src/middleware/default-settings-middleware.ts#L8) | 仅 `transformParams`，`mergeObjects(settings, params)` 合并默认值 |
| `extractJsonMiddleware` | [extract-json-middleware.ts](../reference/ai/packages/ai/src/middleware/extract-json-middleware.ts) | 剥 markdown ``` 代码围栏；流式用三态机 `prefix\|streaming\|buffering`，留 12 字节 suffix buffer 防截断（L78-161） |
| `extractReasoningMiddleware` | [extract-reasoning-middleware.ts](../reference/ai/packages/ai/src/middleware/extract-reasoning-middleware.ts) | 把 `<tagName>...</tagName>` 包裹文本抽成 `reasoning` content；流式用 `getPotentialStartIndex` 增量识别标签（L188-242） |
| `simulateStreamingMiddleware` | | 模拟流式 |
| `addToolInputExamplesMiddleware` | | 添加工具输入示例 |

另有 `wrapEmbeddingModel`/`wrapImageModel`/`wrapProvider`（同样 reverse-reduce 包装模式）。

## 4. agent 系统

### Agent 接口

[agent.ts:184-219](../reference/ai/packages/ai/src/agent/agent.ts#L184)：`version: 'agent-v1'`、只读 `id`/`tools`、`generate(options)` 与 `stream(options)`。`AgentCallParameters` 基本是 `generateText` 的调用子集（prompt/messages、abortSignal、timeout、各 onStart/onStepStart/onToolExecutionStart/onStepEnd/onEnd 回调、`experimental_sandbox`），可带 `callOptionsSchema` 校验过的 `options`。

### ToolLoopAgent

[tool-loop-agent.ts:39-323](../reference/ai/packages/ai/src/agent/tool-loop-agent.ts#L39) 是内置实现，**本质就是 `generateText`/`streamText` 的预设封装**：

- 构造时存 `ToolLoopAgentSettings`（model、tools、instructions、prepareCall、stopWhen 等）
- `generate`/`stream` 先 `prepareCall`：用 `callOptionsSchema` 校验 `options`，合并 `settings.stopWhen ?? isStepCount(20)`（**默认 20 步**，L132），调可选 `prepareCall` 钩子改写参数
- 通过 `mergeCallbacks` 把 agent 级回调与本次调用级回调合并（L224-248）
- `agentHeaders` 给请求加 `ai-sdk-agent/tool-loop` UA 后缀用于用量归因（L185-192）

### 与直接调 streamText 的区别

| 维度 | 直接 streamText | ToolLoopAgent |
|------|----------------|---------------|
| model/tools/instructions | 每次调用传入 | 固化成可复用对象 |
| 默认步数 | `isStepCount(1)`（单步） | `isStepCount(20)` |
| 调用时参数注入 | 无 | `prepareCall` 钩子 + `callOptionsSchema` 校验 |
| 回调叠加 | 单层 | agent 级 + 调用级合并 |

其余循环、工具执行、停止逻辑全部复用 `generateText`/`streamText`。

## 5. embed / rerank / transcribe

三者都是单次模型调用、**无工具循环、无多步、无 streaming 文本流**，接口围绕各自的 `doEmbed`/`doRank`/`doTranscribe`。

| 函数 | 文件 | 模型类型 | 签名要点 |
|------|------|----------|----------|
| `embed` | [embed.ts:41-269](../reference/ai/packages/ai/src/embed/embed.ts#L41) | `EmbeddingModel` | `embed({ model, value: string, ... })` → `{ value, embedding, usage, ... }`；内部 `retry(() => model.doEmbed({ values:[value] }))` 取 `embeddings[0]` |
| `rerank` | [rerank.ts:38-53](../reference/ai/packages/ai/src/rerank/rerank.ts#L38) | `RerankingModel` | `rerank<VALUE>({ model, documents: VALUE[], query, topN?, ... })`；泛型 `VALUE` 为 `JSONObject \| string` |
| `transcribe` | [transcribe.ts:34-52](../reference/ai/packages/ai/src/transcribe/transcribe.ts#L34) | `TranscriptionModel` | `transcribe({ model, audio: DataContent\|URL, ... })`；`streamTranscribe` 是流式变体 |

与 generate-text 的区别：没有 `tools`/`toolChoice`/`steps`/`stopWhen`/`prepareStep`/`output`，没有消息四层模型，没有 stitchable stream；只有 retry、telemetry、UA 后缀、`onStart`/`onEnd` 这套公共骨架。

---

## 复核记录

**复核员**：Kevin | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | stopWhen 默认 isStepCount(1)，L240 | ✅ | generate-text.ts:240 `stopWhen = isStepCount(1),` |
| 2 | 循环体 L797-1375 | ✅ | do@L797，while@L1365-1375；while 条件含 clientToolCalls + pendingDeferredToolCalls + isStopConditionMet |
| 3 | generateObject 通过 responseFormat 传 schema，L388-402 | ✅ | L390-395 `responseFormat: { type: 'json', schema: jsonSchema, name, description }` |
| 4 | injectJsonInstruction L8-30 | ✅ | L8 函数声明；L5 `DEFAULT_SCHEMA_SUFFIX = 'You MUST answer with a JSON object that matches the JSON schema above.'` |
| 5 | OutputStrategy 四策略，L30-63 | ✅ | L31 `type: 'object' \| 'array' \| 'enum' \| 'no-schema'`；array 包 {elements:[...]}@L142-155 |
| 6 | wrapLanguageModel reverse-reduce，L25-42 | ✅ | L37-41 reverse().reduce()；L44-113 四类钩子 transformParams/wrapGenerate/wrapStream/override* |
| 7 | ToolLoopAgent 默认 isStepCount(20)，L132 | ✅ | tool-loop-agent.ts:132 `stopWhen: this.settings.stopWhen ?? isStepCount(20),` |
| 8 | embed/rerank/transcribe 无工具循环 | ✅ | embed.ts:41 函数声明；L185 retry(async () => model.doEmbed({ values:[value] }))；L207 embeddings[0] |
