# 补充包

> 参考源码：`reference/ai/packages/` 下 openai-compatible/policy-opa/harness-*/langchain/llamaindex/tui/devtools/vercel/workflow-harness
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. openai-compatible — OpenAI 兼容基座 provider

[src/openai-compatible-provider.ts](../reference/ai/packages/openai-compatible/src/openai-compatible-provider.ts) 的 `createOpenAICompatible()` 是所有"长得像 OpenAI"的 provider 的工厂。接收 `OpenAICompatibleProviderSettings`（`baseURL`、`name`、`apiKey`、`headers`、`queryParams`、`fetch`、`includeUsage`、`supportsStructuredOutputs`、`transformRequestBody`、`metadataExtractor`、`convertUsage`），统一构造 `provider/url/headers/fetch` 四件套，实例化四个模型类：`OpenAICompatibleChatLanguageModel`、`OpenAICompatibleCompletionLanguageModel`、`OpenAICompatibleEmbeddingModel`、`OpenAICompatibleImageModel`。

错误结构由 `openai-compatible-error.ts` 的 `openaiCompatibleErrorDataSchema` + `ProviderErrorStructure<T>` 抽象，允许子 provider 替换 schema 和 `errorToMessage`/`isRetryable`。

**继承它的 provider**：baseten、cerebras、deepinfra、deepseek（间接）、fireworks、huggingface、moonshotai、togetherai、vercel、alibaba 等。存在的理由：避免每家 OpenAI-compatible 厂商重写转换层，只在 `transformRequestBody`/`metadataExtractor`/`convertUsage` 处差异化。

## 2. policy-opa — OPA 策略层

### PolicyClient 接口

[src/policy-client.ts](../reference/ai/packages/policy-opa/src/policy-client.ts) 的 `PolicyClient` 只有 `evaluate(path, input)` 一个方法。两个后端实现：`wasmPolicyClient`（用 `@open-policy-agent/opa-wasm` 在进程内跑 `opa build -t wasm` 编译出的 bundle）和 `httpPolicyClient`（远程 OPA 服务）。

### 策略决策

[src/policy-decision.ts](../reference/ai/packages/policy-opa/src/policy-decision.ts) 把 SDK 的 `ToolApprovalStatus` 规范成 `{type:'approved'|'denied'|'user-approval'|'not-applicable'}`。

### opaPolicy

[src/opa/opa-policy.ts](../reference/ai/packages/policy-opa/src/opa/opa-policy.ts) 把 client 包成 `ToolApprovalConfiguration`：每个 tool call 都用 `{tool:{name}, args, messages, runtimeContext}` 调一次 Rego 入口，**fail-closed**——后端报错时返回 `{type:'denied', reason}` 而不是放行。

### 影子模式

[src/shadow.ts](../reference/ai/packages/policy-opa/src/shadow.ts)：照常评估策略并通过 `onDecision` 上报 `PolicyDecisionEvent`，但 `enforce:false` 时永远告诉 SDK"approved"，用于上线前观察策略会拒绝什么。

### wrapMcpTools

[src/wrap-mcp-tools.ts](../reference/ai/packages/policy-opa/src/wrap-mcp-tools.ts)：给一整组已发现 MCP 工具套上 fallback（默认 `user-approval`），让没显式配置规则的工具不会被静默放行。

## 3. harness-* 适配器

共同契约是 `@ai-sdk/harness` 的 `HarnessV1` spec。四个 CLI 类适配器（claude-code、codex、deepagents、opencode）共用同一套骨架：

| 组件 | 说明 |
|------|------|
| **auth** | 解析 anthropic/gateway 环境变量；claude-code 还读 `~/.claude/settings.json` 的 `apiKeyHelper` |
| **bridge-protocol** | 基于 `harnessV1BridgeOutboundMessageSchema` + 各自 `startMessageSchema` 扩展（Claude 的 `thinking`/`maxTurns`/`skills`、Codex 的 `reasoningEffort`/`webSearch`） |
| **instructions** | 仅在新会话首条 user 消息前置，resumed 会话跳过 |
| **SandboxChannel + WebSocket** | 在沙箱内与 CLI 进程双向通信 |
| **commonTool** | 声明原生工具集（`bash`/`read`/`write`/`edit`/`grep`/`glob`/`webSearch`…） |

**各适配器特殊之处**：

| 适配器 | 特殊点 |
|--------|--------|
| `harness-deepagents` | bootstrap 阶段校验和安装 ripgrep 静态二进制，skills 写到 `$HOME/.agents/skills` |
| `harness-pi` | **无 bridge**——Pi 作为 in-process Node 库运行，无 `port`/`startupTimeoutMs`，session 由 `createPiSession` 直接创建 |

## 4. langchain / llamaindex — 框架互操作

### langchain

[src/adapter.ts](../reference/ai/packages/langchain/src/adapter.ts)、[transport.ts](../reference/ai/packages/langchain/src/transport.ts)、[stream-callbacks.ts](../reference/ai/packages/langchain/src/stream-callbacks.ts)：

- `toBaseMessages(UIMessage[])` → LangChain `BaseMessage[]`（经 `convertToModelMessages` 中转）
- `toUIMessageStream()` 把 LangGraph 事件流（`AIMessageChunk`、tool calls、reasoning、citations）转成 AI SDK `UIMessageStream`
- `LangSmithDeploymentTransport` 实现 `ChatTransport`，内部包 `@langchain/langgraph/remote` 的 `RemoteGraph`，让 `useChat` 直连 LangSmith 部署的 LangGraph agent
- `StreamCallbacks` 提供 `onStart/onToken/onText/onFinal/onFinish/onError/onAbort` 钩子

### llamaindex

[src/llamaindex-adapter.ts](../reference/ai/packages/llamaindex/src/llamaindex-adapter.ts) 极薄：单个 `toUIMessageStream(stream: AsyncIterable<EngineResponse>)` 把 LlamaIndex 的 `{delta}` 流 trim 首部空白后包成 `text-start/text-delta/text-end` 的 `UIMessageChunk` 流。

## 5. tui — 终端 agent 运行器

[src/run-agent-tui.ts](../reference/ai/packages/tui/src/run-agent-tui.ts) 的 `runAgentTUI(options)` 实例化 `AgentTUIRunner`：

- 两种模式：本地 `agent: AgentTUIAgent` + 可选 `sandbox`，或远程 `transport: ChatTransport<UIMessage>`
- 渲染由 `TerminalRenderer` + `TerminalFrameBuffer` + markdown 处理
- 每个 part（tool call / reasoning）的展示模式可配 `full|collapsed|auto-collapsed|hidden`
- 可显示 output token 数或 tokens/sec，可对照 `contextSize` 显示上下文占用百分比
- `AgentTUIRenderer` 接口抽象出 `readPrompt`、`readToolApproval`、`renderStream`，便于测试注入 mock

## 6. devtools — DevTools 中间件 + 持久化 + 查看器

### 中间件

[src/middleware.ts](../reference/ai/packages/devtools/src/middleware.ts) 的 `devToolsMiddleware()` 是 `LanguageModelV4Middleware`，在 `NODE_ENV==='production'` 下直接抛错。每次调用生成 `runId`，`wrapGenerate`/`wrapStream` 写 `Step` 记录（`type`、`model_id`、`provider`、`input`、`output`、`usage`、`raw_request`、`raw_response`、`raw_chunks`）。流式用 `TransformStream` 收集 chunks，注册 `SIGINT`/`SIGTERM` 处理器在进程退出前 flush。

### 持久化

[src/db.ts](../reference/ai/packages/devtools/src/db.ts) 写入 `process.cwd()/.devtools/generations.json`（最多 100 MB），结构 `{runs:[], steps:[]}`。**不是 IndexedDB，是文件型 JSON。**

### 查看器

[src/viewer/server.ts](../reference/ai/packages/devtools/src/viewer/server.ts) 是 Hono 服务（默认 4983 端口），通过 SSE 广播 DB 变更；中间件通过 `fetch('http://localhost:4983/api/notify')` fire-and-forget 通知重载。

## 7. vercel — v0 模型 provider（不是 Gateway）

> ⚠️ 这个包**不是** Vercel AI Gateway 的客户端。

[src/vercel-provider.ts](../reference/ai/packages/vercel/src/vercel-provider.ts) 的 `createVercel()` 默认 `baseURL='https://api.v0.dev/v1'`，用 `VERCEL_API_KEY` 鉴权，**只**实例化 `OpenAICompatibleChatLanguageModel`，模型 id 是 `v0-1.0-md`/`v0-1.5-md`/`v0-1.5-lg`。`embeddingModel`/`imageModel` 直接抛 `NoSuchModelError`。

与 `@ai-sdk/gateway` 的区别：

| 维度 | vercel 包 | gateway 包 |
|------|-----------|------------|
| 面向 | v0 单一聊天模型 | 所有模态（language/embedding/image/video/...） |
| baseURL | `api.v0.dev/v1` | `ai-gateway` 服务 |
| 能力 | 仅 chat | 完整模型族 + 计费/限额/团队 header |

## 8. workflow-harness — 把 harness 接入 Workflow DevKit

### HarnessWorkflowState

[harness-workflow-state.ts](../reference/ai/packages/workflow-harness/src/harness-workflow-state.ts) 定义纯 JSON-serializable 状态机：`sessionId`、`prompt`、`messages`、`status: 'running'|'timed_out'|'awaiting_tool_approval'|'finished'|'failed'`、`resumeFrom`（跨进程 warm session 句柄）、`continueFrom`（同一 turn 的 suspended 句柄）、`streamContext`、`finalResult`。

### runHarnessAgentSlice

[run-harness-agent-slice.ts](../reference/ai/packages/workflow-harness/src/run-harness-agent-slice.ts) 的 `runHarnessAgentSlice()` 是用户 `'use step'` 函数体的预期实现：

1. 用 `resumeFrom`/`continueFrom` 重连（或创建）`HarnessAgentSession`
2. 调 `agent.stream({session, prompt})` 或 `agent.continueStream({session})`
3. chunks 写入 workflow 的 `getWritable()`
4. **race** turn 与 wall-clock budget（默认 750s，在 Vercel Fluid Compute ~800s 回收之前）
5. budget 先到则 `session.suspendTurn()` 返回 `timed_out`（沙箱继续跑），turn 先结束返回 `finished`

把 harness 的有状态长会话嫁接到 Workflow DevKit 的持久化检查点模型上，让 serverless 函数能跨实例续跑 agent turn。

---

## 复核记录

**复核员**：Rachel | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | createOpenAICompatible + 4 模型 + errorDataSchema 抽象 | ✅ | createOpenAICompatible@L123；四模型@L170-193；openai-compatible-error.ts@L2 errorDataSchema + ProviderErrorStructure@L19-23 |
| 2 | PolicyClient.evaluate + wasm/http + fail-closed + shadow + wrap-mcp-tools | ✅ | policy-client.ts@L9-19 仅 evaluate；wasm@L28/http@L17；opa-policy.ts@L77-86 fail-closed；shadow.ts@L87/94 enforce:false→approved；wrap-mcp-tools.ts@L59 fallback |
| 3 | harness-claude-code 五要素 + deepagents ripgrep + pi in-process | ✅ | auth@claude-code-auth.ts:100 读 settings.json；bridge-protocol 存在；SandboxChannel+WebSocket@L31/41；commonTool@L5/103；deepagents@L58/66-81 装 ripgrep；pi@L19-20 无 bridge in-process |
| 4 | langchain toBaseMessages + LangSmithDeploymentTransport + StreamCallbacks | ✅ | toBaseMessages@adapter.ts:67；LangSmithDeploymentTransport@transport.ts:48 实现 ChatTransport；StreamCallbacks@stream-callbacks.ts:4 七钩子 |
| 5 | tui runAgentTUI + TerminalRenderer + 四种展示模式 | ✅ | runAgentTUI@run-agent-tui.ts:103；TerminalRenderer@terminal-renderer.ts:147；展示模式@L18-22 full\|collapsed\|auto-collapsed\|hidden |
| 6 | devtools production 抛错 + JSON 文件 + Hono 4983 | ✅ | middleware.ts@L100-106 production 抛错；db.ts@L4-5/L130 写 .devtools/generations.json（非 IndexedDB）；server.ts@L3 Hono/@L59/@L323 默认 4983 |
| 7 | vercel createVercel v0.dev/v1 + 仅 Chat + embedding/image 抛错 | ✅ | vercel-provider.ts@L56-58 baseURL api.v0.dev/v1；@L86-90 仅 OpenAICompatibleChatLanguageModel；@L96-102 embedding/image 抛 NoSuchModelError |
| 8 | workflow-harness JSON 状态 + status 枚举 + 750s race | ✅ | harness-workflow-state.ts@L24-29 status 枚举；@L56-60 JSON-serializable；run-harness-agent-slice.ts@L26 DEFAULT_SLICE_TIMEOUT_SECONDS=750；@L170-172 setTimeout race |
