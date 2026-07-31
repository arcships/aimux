# 外围基础设施包

> 参考源码：`reference/ai/packages/` 下的 workflow/mcp/otel/gateway/open-responses/sandbox-*/harness/valibot
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. workflow — 持久化（durable）AI Agent

[workflow-agent.ts](../reference/ai/packages/workflow/src/workflow-agent.ts) 的 `WorkflowAgent` 类（L1211）是核心。它**不是**简单的 multi-step agent，而是构建在持久化执行引擎之上的 agent：`do-stream-step.ts:114` 和 `workflow-agent.ts:2623/2645/2669` 多处出现 `'use step';` 指令，把每次 LLM 调用、流关闭、tool-result 写入都标记为可 checkpoint 的持久化步骤。

### API 设计

与 ToolLoopAgent 高度同构（`stream()`/`generate()`、`tools`、`stopWhen`、`prepareStep`、`prepareCall`、`runtimeContext`、`output`、`telemetry`、`experimental_sandbox`），但每个 step 可跨进程恢复。

- `PrepareStepCallback`（L353）可按步覆盖 model/messages/toolChoice/runtimeContext
- `Output` 复用自 core 用于结构化输出
- 模型可传字符串如 `'anthropic/claude-opus'`，内部经 `gateway.languageModel()` 解析（do-stream-step.ts:118）
- `WorkflowChatTransport`（workflow-chat-transport.ts）实现 `ChatTransport`，支持可恢复的 UI 消息流（含 orphan chunk 过滤防止 resume 中途崩溃）

### 存在意义

把 agent loop 跑在 durable workflow 上，实现自动重试、持久化、可恢复的长时任务 agent。

## 2. mcp — Model Context Protocol 客户端

[mcp-client.ts:217](../reference/ai/packages/mcp/src/tool/mcp-client.ts#L217) 的 `createMCPClient()` 是入口，返回 `MCPClient`，其 `.tools()` 方法返回 `McpToolSet`——直接可作为 AI SDK `ToolSet` 使用。`mcpToModelOutput`（L147）把 MCP `CallToolResult`（text/image content）转成 AI SDK `ToolResultOutput`。

### 传输层

[mcp-transport.ts:125](../reference/ai/packages/mcp/src/tool/mcp-transport.ts#L125) 支持：

| 传输 | 说明 |
|------|------|
| `sse` | Server-Sent Events |
| `http` | HTTP |
| 自定义 | 可注入实现 `MCPTransport` 接口 |

> 注：内置传输仅支持 `sse`/`http`，配置类型 `MCPTransportConfig.type` 为 `'sse' \| 'http'`。`stdio` 不在内置支持范围内。`mcp-stdio/` 子目录提供 stdio 相关的辅助类型，但需自行实现 `MCPTransport` 接口。

### 其他能力

- **OAuth**（`oauth.ts`）：支持需认证的 MCP server
- **MCP Apps**（`mcp-apps.ts`）：通过 `MCP_APP_MIME_TYPE` 让 MCP server 提供 UI 资源，含 fingerprint 漂移检测
- **Elicitation**（`types.ts` 的 `ElicitationRequestSchema`）：server 可向用户请求输入
- 瞬时错误（408/409/429/5xx 及 ECONNRESET 等）走指数退避重试

## 3. otel — OpenTelemetry 遥测

[open-telemetry.ts:108](../reference/ai/packages/otel/src/open-telemetry.ts#L108) 的 `OpenTelemetry` 类实现 AI SDK 的 `Telemetry` 接口。

### Span 层级

追踪 6 类 span（[supplemental-attributes.ts:24](../reference/ai/packages/otel/src/supplemental-attributes.ts#L24)）：

```
operation > step > languageModel / tool / embedding / reranking
```

### 属性

遵循 GenAI SemConv：

- `gen_ai.client.operation.duration`
- `time_to_first_chunk`、`time_per_output_chunk`（L98）
- 模型 ID、provider、usage
- system/input/output messages（经 `gen-ai-format-messages.ts` 格式化）

### 接入方式

```ts
new OpenTelemetry({ tracer?, enrichSpan?, usage?, providerMetadata?, ... })
```

传给 `streamText({ telemetry })`。`enrichSpan` 回调可注入自定义属性；`selectAttributes` 按 `recordInputs/Outputs` 过滤敏感数据。`LegacyOpenTelemetry`（legacy-open-telemetry.ts）保留旧实现作向后兼容。

## 4. gateway — Vercel AI Gateway 统一路由

[gateway-provider.ts:285](../reference/ai/packages/gateway/src/gateway-provider.ts#L285) 的 `createGateway()` 是**单一 provider 暴露所有模态**：languageModel/embedding/image/video/reranking/speech/transcription/realtime。

### 模型路由

模型 ID 为 `provider/model` 格式（如 `anthropic/claude-sonnet-4.5`），通过 `ai-language-model-id` header 路由到 `/language-model` 端点（[gateway-language-model.ts:228](../reference/ai/packages/gateway/src/gateway-language-model.ts#L228)）。

### 特殊之处

| 能力 | 文件 | 说明 |
|------|------|------|
| 计费/限额 | `gateway-spend-report.ts:75` | `getSpendReport` 按 day/user/model/tag/provider/credential_type 聚合 cost/tokens/requests；`getCredits` 查余额 |
| 服务端工具 | `gateway-tools.ts` | exaSearch/parallelSearch/perplexitySearch |
| Realtime | `gateway-realtime-auth.ts` | WebSocket 子协议，支持团队级 token |
| 错误层级 | `errors/` | `GatewayRateLimitError`/`GatewayAuthenticationError`/`GatewayModelNotFoundError` 等 |
| Workflow 序列化 | — | `GatewayLanguageModel` 实现 `WORKFLOW_SERIALIZE/DESERIALIZE`，可跨持久化 workflow 边界序列化 |
| 凭证类型 | — | 区分 `byok` vs `system` 凭证 |

## 5. open-responses — OpenAI Responses API 兼容层

[open-responses-provider.ts:46](../reference/ai/packages/open-responses/src/open-responses-provider.ts#L46) 的 `createOpenResponses({ url, name, apiKey })` 返回 provider，`languageModel(modelId)` 返回 `OpenResponsesLanguageModel`（[open-responses-language-model.ts:42](../reference/ai/packages/open-responses/src/open-responses-language-model.ts#L42)），向指定 URL 发送 Responses API 请求（SSE 流、tool calls、reasoning）。

- **只支持 languageModel**，embedding/image 抛 `NoSuchModelError`（L92）
- 同样实现 `WORKFLOW_SERIALIZE` 以支持 durable workflow
- 存在意义：让任何自托管的、兼容 OpenAI Responses API 的服务端作为 AI SDK provider 接入

## 6. sandbox-vercel vs sandbox-just-bash

两者都实现 `HarnessV1SandboxProvider` 接口（`harness-sandbox-v1` spec）。

| 维度 | sandbox-vercel | sandbox-just-bash |
|------|----------------|-------------------|
| 文件 | [vercel-sandbox.ts:80](../reference/ai/packages/sandbox-vercel/src/vercel-sandbox.ts#L80) | [just-bash-sandbox.ts:51](../reference/ai/packages/sandbox-just-bash/src/just-bash-sandbox.ts#L51) |
| 基础 | `@vercel/sandbox` | `just-bash` |
| 快照模板 | ✅ `Sandbox.getOrCreate` 创建持久化 template，session 从 snapshot fork（L166-214） | ❌ 无 |
| 端口暴露 | ✅ `bridgePorts` | ❌ 无端口 |
| resume | ✅ 按 name 恢复 | ❌ |
| 超时 | 默认 30 分钟 | — |
| bootstrap | template 预置 | 每次内联执行（L83） |
| 适用场景 | bridge-backed harness adapter（Claude Code/Codex） | 本地执行、给 AI SDK tools 提供 `SandboxSession.restricted()` |

bridge-backed adapter 会拒绝 sandbox-just-bash provider。

## 7. harness — 第三方 agent 运行时抽象层

`packages/harness/` 是把**外部 coding-agent 运行时**（Claude Code、Codex）当作 AI SDK `Agent` 来驱动的抽象层，分两层：

### HarnessV1 spec

[v1/harness-v1.ts:21](../reference/ai/packages/harness/src/v1/harness-v1.ts#L21)：adapter 契约——`harnessId`、`builtinTools`、`doStart() → HarnessV1Session`。仿 `LanguageModelV4` 设计：tagged spec version + 可选方法表（不支持的能力靠抛 `HarnessCapabilityUnsupportedError`）。

包含：
- bridge protocol（`harness-v1-bridge-protocol.ts`，host 与沙箱内 runtime 的 JSON 消息协议）
- stream parts、lifecycle state（resume/continue）
- permission mode、skills、bootstrap recipe

### HarnessAgent

[agent/harness-agent.ts:117](../reference/ai/packages/harness/src/agent/harness-agent.ts#L117)：实现 AI SDK `Agent` 接口。

- 无状态定义 + 显式 session（`createSession`/`stream`/`generate`）
- 跨进程恢复（`resumeFrom`/`continueFrom`）
- **host tool 执行**：用户工具在 host 跑，结果经 `submitToolResult` 回灌
- 合并 harness builtinTools + 用户 tools
- 沙箱传播；bridge port registry 管理并发 session
- observability（trace-tree/file reporter）

### 与 Agent 的区别

| 维度 | Agent (ToolLoopAgent) | HarnessAgent |
|------|----------------------|--------------|
| 抽象对象 | model provider | 整个 agent runtime（自带工具、prompt、bridge 协议） |
| 工具来源 | 用户定义 | harness builtinTools + 用户 tools 合并 |
| 执行位置 | 同进程 | host tool 在 host 跑，结果回灌 |

## 8. valibot — Schema 适配器

[valibot-schema.ts](../reference/ai/packages/valibot/src/valibot-schema.ts) 仅 15 行。`valibotSchema()` 通过 `@valibot/to-json-schema` 把 valibot schema 转成 JSON Schema，再用 `jsonSchema()` 包装，`validate` 回调用 `v.safeParse`。

存在意义：AI SDK 的标准 schema 接口基于 `jsonSchema`/zod，此包让已用 valibot 的用户无需迁移即可接入。与 `zodSchema` helper 模式一致。

---

## 复核记录

**复核员**：Mike | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | WorkflowAgent L1211，'use step' 指令 | ✅ | L1211 类声明；do-stream-step.ts L114 + workflow-agent.ts L2623/2645/2669 均为 'use step'（另 L2702 也有） |
| 2 | createMCPClient L217，传输层 L125 | ✅（已修正） | createMCPClient@L217 ✅、mcpToModelOutput@L147 ✅；**stdio 不支持**，内置仅 sse/http（MCPTransportConfig.type 为 'sse'\|'http'） |
| 3 | OpenTelemetry L108，6 类 span | ✅ | OpenTelemetry@L108；supplemental-attributes.ts L24-30 定义 6 类 span |
| 4 | createGateway L33，模型路由 L226 | ✅（已修正） | createGateway 实为 **L285**（非 L33）；ai-language-model-id 实为 **L228**（非 L226）；getSpendReport 实为 **L75**（非 L13） |
| 5 | createOpenResponses L46，只支持 languageModel L92 | ✅ | createOpenResponses@L46；L92-94 embeddingModel 抛 NoSuchModelError，L95-97 imageModel 抛 NoSuchModelError |
| 6 | sandbox-vercel 快照 L166-214，sandbox-just-bash 无端口 | ✅ | vercel-sandbox.ts@L80 基于 @vercel/sandbox；快照@L166-214；just-bash-sandbox.ts@L51 无端口无快照 |
| 7 | HarnessV1 spec L21，HarnessAgent L117 | ✅ | harness-v1.ts@L21 specificationVersion='harness-v1'；harness-agent.ts@L117 implements Agent |
| 8 | valibot-schema 15 行，@valibot/to-json-schema | ✅ | 文件 16 行；L1 import @valibot/to-json-schema，L8 jsonSchema(valibotToJsonSchema(...)) |
