# Vercel AI SDK 架构与功能规划

> 参考源码：`reference/ai/`（vercel/ai 仓库浅克隆）
> 复核状态：✅ 已复核（详见文末复核记录）

## 架构概览

SDK 是 provider-agnostic 的 TypeScript 工具包（[README.md](../reference/ai/README.md)），三大 surface（[content/docs/00-introduction/index.mdx](../reference/ai/content/docs/00-introduction/index.mdx)）：

- **AI SDK Core** — 统一文本/对象/工具/agent 生成
- **AI SDK UI** — 框架无关 hooks，支持 React/Svelte/Vue/Angular
- **AI SDK Harnesses** — 用 `HarnessAgent` 接 Claude Code/Codex/Pi

核心分层是"AI 函数 → 模型规范接口 → Provider 实现"三段解耦（[architecture/provider-abstraction.md](../reference/ai/architecture/provider-abstraction.md)）。用户调 `generateText`/`streamText`，依赖 `LanguageModelV4` 规范，由各 provider 包实现。默认经 Vercel AI Gateway 用字符串 `model: 'anthropic/claude-opus-4.6'` 接入所有主流 provider。

## 核心模块

`packages/ai/src/` 模块及职责（按 [index.ts](../reference/ai/packages/ai/src/index.ts) 导出顺序，附 .ts 文件数）：

| 模块 | 文件数 | 职责 |
|------|--------|------|
| **generate-text/** | 81（最大） | `generateText`/`streamText`、多步工具循环、tool approval、stop condition、step/result 装配。[stream-text-loop-control.md](../reference/ai/architecture/stream-text-loop-control.md) 描述的 stitchable stream 管道在此 |
| **generate-object/** | 16 | `generateObject`/`streamObject` + `Output.object`，结构化输出（schema 校验、JSON 注入与修复 `repair-text.ts`） |
| **generate-image/** | 4 | 图像生成 |
| **generate-speech/** | 5 | 语音生成 |
| **generate-video/** | 4 | 视频生成 |
| **transcribe/** | 7 | 音频转写 |
| **rerank/** | 5 | 重排序 |
| **embed/** | 8 | `embed`/`embedMany` |
| **agent/** | 13 | `ToolLoopAgent`、`agent.ts` 抽象、`createAgentUIStreamResponse` |
| **model/** | 29 | `as-language-model-v4` 等版本适配器 |
| **ui/** | 31 | `useChat` 服务端 transport |
| **ui-message-stream/** | 27 | UIMessage 流转换、model↔UI message 转换 |
| **middleware/** | 21 | default-settings、extract-json、extract-reasoning 等横切中间件 |

## 功能规划

- **文本**：`generateText`/`streamText`，多步工具循环与 tool approval（`collect-tool-approvals.ts`）
- **结构化**：`generateObject`/`streamObject`，Zod/JSON schema
- **多模态**：图像/语音/视频生成、音频转写、embedding、rerank
- **Agent**：`ToolLoopAgent` + 沙箱/本地 shell 工具
- **Harness**：`HarnessAgent` 接 Claude Code/Codex/Pi
- **沙箱**：`Experimental_SandboxSession` + `HarnessV1NetworkSandboxSession` 两层
- **文件上传**：`uploadFile` + `SharedV4ProviderReference` 跨 provider 复用
- **UI**：框架无关 hooks + 生成式 UI（`UIToolInvocation`）
- **网关**：`createGateway`/`gateway` 默认 Vercel AI Gateway

## 关键设计决策

### 1. 四层消息模型

来源：[architecture/message-layers.md](../reference/ai/architecture/message-layers.md)（标题 "Four level message architecture summary"）

- **UI messages** — 含 data parts，面向 UI 渲染
- **Model messages** — 抽象的、用户友好的版本，用于 generate/stream 调用
- **Language model messages** — 标准化 spec，追求稳定
- **Provider-specific messages** — 最终转换为特定 API 要求

### 2. V4 模型规范统一 8 种模型类型

V4 规范定义了 8 种 `*-model` 类型（`packages/provider/src/` 下带 `specificationVersion: 'v4'` 的 \*-model-v4.ts）：

1. `LanguageModelV4` — language-model/v4/
2. `EmbeddingModelV4` — embedding-model/v4/
3. `ImageModelV4` — image-model/v4/
4. `RerankingModelV4` — reranking-model/v4/
5. `TranscriptionModelV4` — transcription-model/v4/
6. `SpeechModelV4` — speech-model/v4/
7. `VideoModelV4` — video-model/v4/
8. `RealtimeModelV4` — realtime-model/v4/（WebSocket 双向音频/文本）

> 注：`FilesV4`（files/v4/）是文件上传接口，不是模型类型——它没有 modelId 和生成方法。[file-uploads.md](../reference/ai/architecture/file-uploads.md) 将其定义为 "interface that providers implement to support file uploads"。`ProviderV4` 接口本身只暴露 6 个 model 工厂 + files + skills（不含 video/realtime，二者为独立可选规范）。

### 3. streamText 多步循环

来源：[architecture/stream-text-loop-control.md](../reference/ai/architecture/stream-text-loop-control.md)

- **Stitchable stream**（L84-86）— 顺序队列，一次消费一个 step 流
- **Funnel in**（L63-64）— N 个 step 流顺序合并（非并行）
- **Funnel out**（L121-138）— 用 `.tee()` 按需分流，产出 `textStream` / `fullStream` / `partialOutputStream` / `uiMessageStream`（另有 elemStream、consumeStream）

### 4. 两层沙箱 + restricted() 收窄

来源：[architecture/sandbox-abstraction.md](../reference/ai/architecture/sandbox-abstraction.md)、[architecture/harness-abstraction.md](../reference/ai/architecture/harness-abstraction.md)

- basic 只文件+进程；network 加 id/port/lifecycle
- `restricted()` 把 network session 收窄给工具防越权
- `HarnessAgent` 拥有沙箱生命周期，adapter 只操作沙箱
- resume session vs continue turn 分离；OIDC 优先于长密钥

### 5. 文件上传用 provider reference 解耦

来源：[architecture/file-uploads.md](../reference/ai/architecture/file-uploads.md)

- `uploadFile` — 用户面上传函数（L7）
- `SharedV4ProviderReference`（`Record<string, string>`）— 映射 provider 名到 provider 专属文件标识符（L9, L13）
- message 引用而非内联 bytes；不支持 provider 抛 `UnsupportedFunctionalityError`

---

## 复核记录

**复核员**：Dave | **复核方式**：逐条打开源码验证路径/行号/文件数

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | architecture/ 下 6 个文件 | ✅ | glob 命中 6 个，完全一致 |
| 2 | 7 个子目录文件数 | ✅ | PowerShell 递归统计全部吻合 |
| 3 | 四层消息模型 | ✅ | message-layers.md 标题 "Four level message architecture summary"，逐层列出 |
| 4 | V4 规范 8 种模型类型 | ❌→✅ 已修正 | 原报告误把 `files` 当模型类型并漏掉 `realtime`。实际 8 种 \*-model-v4.ts：language/embedding/image/reranking/transcription/speech/video/realtime |
| 5 | stitchable stream / tee / 四流 | ✅ | stream-text-loop-control.md L84-86/L121-138 原文确认 |
| 6 | SharedV4ProviderReference / uploadFile | ✅ | file-uploads.md L7/L9/L13/L32 |
| 7 | repair-text.ts 在 generate-object/ | ✅ | glob 命中 packages/ai/src/generate-object/repair-text.ts |
| 8 | ToolLoopAgent / createAgentUIStreamResponse | ✅ | agent/tool-loop-agent.ts + agent/create-agent-ui-stream-response.ts |
