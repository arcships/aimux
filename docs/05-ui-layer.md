# UI 层与框架绑定

> 参考源码：`reference/ai/packages/ai/src/ui/`、`ui-message-stream/`、`packages/react/`、`rsc/`、`svelte/`、`vue/`、`angular/`
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. useChat 与框架无关的 AbstractChat

核心 UI 层定义在 [packages/ai/src/ui/chat.ts](../reference/ai/packages/ai/src/ui/chat.ts)。`AbstractChat<UI_MESSAGE>` 是框架无关的抽象类，承载全部业务逻辑：`sendMessage` / `regenerate` / `resumeStream` / `addToolOutput` / `addToolApprovalResponse` / `stop`（L237-792）。

### Transport 机制

通过 `ChatTransport<UI_MESSAGE>` 接口（[chat-transport.ts:15](../reference/ai/packages/ai/src/ui/chat-transport.ts#L15)）抽象通信。默认实现 `DefaultChatTransport` 继承 `HttpChatTransport`（[http-chat-transport.ts:116](../reference/ai/packages/ai/src/ui/http-chat-transport.ts#L116)）：

- 向 `/api/chat` POST `{id, messages, trigger, messageId}`（端点 [http-chat-transport.ts:128](../reference/ai/packages/ai/src/ui/http-chat-transport.ts#L128)，body 构造 L175-185，POST 调用 L191-200）
- 响应体经 `parseJsonEventStream` + `uiMessageChunkSchema` 校验后转为 `ReadableStream<UIMessageChunk>`（[default-chat-transport.ts:19-35](../reference/ai/packages/ai/src/ui/default-chat-transport.ts#L19)，该处为 `processResponseStream` 方法）
- `reconnectToStream` 走 `GET /api/chat/{chatId}/stream`，204 表示无活动流（L215-268）
- Transport 可替换为 WebSocket 等自定义实现

### 状态与重渲染解耦

`AbstractChat` 不直接持有响应式状态，而是通过注入的 `ChatState<UI_MESSAGE>`（L138-149）读写 `messages/status/error`，用 `snapshot()` 做深拷贝。`makeRequest` 用 `SerialJobExecutor` 串行化 update job 防止竞态（L689-716）；`write()` 回调在每次 chunk 落地时调用 `state.replaceMessage` 或 `pushMessage`，由具体框架的状态实现触发重渲染。

## 2. UIMessage 流转换管道

### 服务端管道

`packages/ai/src/ui-message-stream/`：

- `toUIMessageStream` 把 `streamText` 产生的 `TextStreamPart<TOOLS>` 经 `toUIMessageChunk` 逐 part 映射为 `UIMessageChunk`（[to-ui-message-chunk.ts:30-386](../reference/ai/packages/ai/src/ui-message-stream/to-ui-message-chunk.ts#L30)）：
  - `text-delta` → `text-delta`
  - `tool-call` → `tool-input-available`
  - `tool-result` → `tool-output-available`
  - `tool-approval-request`/`response` 直传
- `handleUIMessageStreamFinish` 注入 `messageId` 并触发 `onStepEnd`/`onEnd` 回调（[handle-ui-message-stream-finish.ts:13-188](../reference/ai/packages/ai/src/ui-message-stream/handle-ui-message-stream-finish.ts#L13)）
- `createUIMessageStream` 提供手动 `writer` + `merge` 合并子流（[create-ui-message-stream.ts:28-161](../reference/ai/packages/ai/src/ui-message-stream/create-ui-message-stream.ts#L28)）
- 最终经 `JsonToSseTransformStream` 编码为 SSE 响应（[create-ui-message-stream-response.ts:19-44](../reference/ai/packages/ai/src/ui-message-stream/create-ui-message-stream-response.ts#L19)）

### 客户端管道

[process-ui-message-stream.ts](../reference/ai/packages/ai/src/ui/process-ui-message-stream.ts)：`processUIMessageStream` 在 `TransformStream` 内根据 chunk 类型维护 `StreamingUIMessageState`（`activeTextParts` / `partialToolCalls` / `activeReasoningParts`，L34-76）：

- `text-start` push 新 `TextUIPart{state:'streaming'}`
- `text-delta` 累加 `textPart.text += chunk.delta`
- `text-end` 置 `state:'done'`（L423-471）
- 每次变更调用 `write()` 触发 UI 回写

### Tool invocation 在 UI 中的表现

`ToolUIPart` 状态机（转换逻辑在 L577-870，辅助函数 `updateToolPart` L173-286 / `updateDynamicToolPart` L288-397）：

```
input-streaming → input-available → output-available / output-error
                                       ↘ approval-requested → approval-responded / output-denied
```

完整 7 个状态：

| 状态 | 触发位置 | 说明 |
|------|----------|------|
| `input-streaming` | L595/L606 | tool-input-start |
| `input-available` | L666/L677 | tool-input-available |
| `output-available` | L799/L815 | tool-output-available |
| `output-error` | L716/L727/L840/L855 | tool-output-error / tool-input-error |
| `approval-requested` | L743 | tool-approval-request |
| `approval-responded` | L764 | tool-approval-response |
| `output-denied` | L786 | 审批拒绝 |

`addToolOutput` 直接 mutate part 并写回（[chat.ts:531-578](../reference/ai/packages/ai/src/ui/chat.ts#L531)）。

### Data parts

用 `data-${name}` 前缀（[ui-message-chunks.ts:170-179](../reference/ai/packages/ai/src/ui-message-stream/ui-message-chunks.ts#L170)），可声明 schema 验证，通过 `transient:true` 表达不持久化。

## 3. React 绑定

`packages/react/src/` 是薄桥接层。

- `Chat`（[chat.react.ts:114-136](../reference/ai/packages/react/src/chat.react.ts#L114)）继承 `AbstractChat`，注入 `ReactChatState`
- `ReactChatState` 用三个 `Set<() => void>` 回调集合分别管理 messages/status/error 订阅，setter 触发对应回调（L10-112）
- `replaceMessage` 用 `structuredClone` 强制新引用以兼容 React Compiler 的深比较（L62-72）
- `useChat`（[use-chat.ts:65-200](../reference/ai/packages/react/src/use-chat.ts#L65)）用 `useSyncExternalStore` 三次订阅（messages/status/error），通过 `~registerMessagesCallback`（可 throttle）等私有方法对接 `ReactChatState`
- `latestRef` 模式保证 `onToolCall`/`onFinish`/`transport` 等回调始终拿到最新值而无需重建 Chat 实例（L77-122）
- 另提供 `useCompletion`、`useObject`、`useRealtime` 三个额外 hook
- 无自带 component——纯 hook 暴露

## 4. RSC（React Server Components）

`packages/rsc/src/` 提供与 `useChat` 完全不同的范式：服务端直接渲染 ReactNode 流。

### streamUI

[stream-ui.tsx:98-431](../reference/ai/packages/rsc/src/stream-ui/stream-ui.tsx#L98)：

- 直接调用 `model.doStream`（绕过 `streamText`），`tee()` 出两路：一路返回给调用方，一路在 async IIFE 内消费
- 对 `text-delta` 调用 `text` renderer，对 `tool-call` 解析输入后调用该 tool 的 `generate` renderer（L318-383）
- renderer 可以是 generator/asyncGenerator，逐次 `streamableUI.update(node)`，最后一次 `.done(node)`（L242-265）
- 底层 `createStreamableUI` 用 Promise 链把每次 update 串成 suspended chunk 序列，靠 React Suspense 流式渲染（[create-streamable-ui.tsx:55-146](../reference/ai/packages/rsc/src/streamable-ui/create-streamable-ui.tsx#L55)）

### createAI + AI State

[provider.tsx:49-149](../reference/ai/packages/rsc/src/provider.tsx#L49)、[ai-state.tsx](../reference/ai/packages/rsc/src/ai-state.tsx)：

- 用 Node `AsyncLocalStorage` 在 Server Action 内提供 `getAIState`/`getMutableAIState`
- `.done()` 时用 `jsondiffpatch.diff` 计算 delta 回传客户端（L197-204）
- `createStreamableValue` 提供非 ReactNode 的值流（支持 patch/diff），客户端用 `useStreamableValue` 读取

### 与 useChat 的区别

| 维度 | useChat | RSC streamUI |
|------|---------|--------------|
| 状态同步 | 客户端 fetch SSE，自己维护 `UIMessage[]` | 服务端渲染 ReactNode 流，Server Action 同步 AI state |
| 渲染 | 客户端组件 | 服务端 Suspense 流式 |
| model 调用 | 经 streamText | 直接调 `model.doStream` |

## 5. Svelte / Vue / Angular 绑定

三者的特殊之处仅在 `ChatState` 的响应式原语实现，业务逻辑全部复用 `AbstractChat`：

| 框架 | 响应式原语 | 文件 | 特殊处理 |
|------|-----------|------|----------|
| **Svelte** | `$state` runes | [chat.svelte.ts:22](../reference/ai/packages/svelte/src/chat.svelte.ts#L22)（类声明）；`$state` 用法在 L26-30、`$state.snapshot` 在 L49 | `snapshot` 走 `$state.snapshot`；直接 mutate，runes 自动追踪 |
| **Vue** | `shallowRef` + `triggerRef` | [use-chat.ts:90](../reference/ai/packages/vue/src/use-chat.ts#L90)（函数声明）；`shallowRef` 在 L93-95、`triggerRef` 在 L128-139 | `replaceMessage` 浅拷贝规避深响应式对 tool parts 的异常；`watch(toValue(init))` 重建 |
| **Angular** | `signal` | [chat.ng.ts:9](../reference/ai/packages/angular/src/lib/chat.ng.ts#L9)（Chat 类）；`signal` 实际在 `AngularChatState` 类 L23-25 | `.set([...])`/`.update` 不可变更新；`snapshot` 用 `structuredClone` |

三包都没有自定义 transport 或 chunk 处理，仅是响应式原语适配；hook/composable 签名对齐 React `useChat`。

---

## 复核记录

**复核员**：Larry | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | AbstractChat L237-792，ChatTransport L15 | ✅ | 六方法均在范围内（sendMessage@334/regenerate@429/resumeStream@463/addToolApprovalResponse@477/addToolOutput@531/stop@586） |
| 2 | DefaultChatTransport POST /api/chat | ✅（已修正） | POST 逻辑在 http-chat-transport.ts（端点 L128、body L175-185、POST L191-200），非 default-chat-transport.ts L19-35（该处为 processResponseStream） |
| 3 | toUIMessageChunk 映射 L30-386 | ✅ | text-delta→text-delta@L70、tool-call→tool-input-available@L209、tool-result→tool-output-available@L274 |
| 4 | ToolUIPart 状态机 | ✅（已修正） | 完整 7 个状态（input-streaming/input-available/output-available/output-error/approval-requested/approval-responded/output-denied）；转换逻辑在 L577-870，辅助函数 L173-397 |
| 5 | ReactChatState Set<callback> L10-112 | ✅ | 三 Set@L17-19，structuredClone 经 snapshot@L72 |
| 6 | useChat useSyncExternalStore L65-200 | ✅ | 三次订阅@L146/152/158，latestRef@L77-122 |
| 7 | streamUI 直接调 model.doStream L98-431 | ✅ | model.doStream@L287-303（无 streamText），tee()@L306 |
| 8 | createStreamableUI Promise 链 L55-146 | ✅ | createResolvablePromise + next 字段串成 suspended chunk 链 |
| 9 | Svelte $state / Vue shallowRef / Angular signal | ✅（已修正） | 三原语均使用；行号修正：$state@L26-30、shallowRef@L93-95/triggerRef@L128-139、signal@L23-25（AngularChatState 类内） |
