# Vercel AI SDK 各 Provider 实现差异

> 参考源码：`reference/ai/packages/openai/`、`anthropic/`、`google/`
> 复核状态：✅ 已复核（详见文末复核记录）

## OpenAI 实现要点

OpenAI provider 同时实现两套语言模型：Chat Completions API 与 Responses API。

- 主文件：[openai-chat-language-model.ts](../reference/ai/packages/openai/src/chat/openai-chat-language-model.ts)（638 行）与 [openai-responses-language-model.ts](../reference/ai/packages/openai/src/responses/openai-responses-language-model.ts)
- 实现 `LanguageModelV4`：`specificationVersion = 'v4'`（chat: L55-56；responses: L194-195）

### 请求构造

`getArgs()`（L89-340）将 `LanguageModelV4CallOptions` 映射为 OpenAI body：

- `model`、`messages`、`max_tokens`/`max_completion_tokens`（reasoning 模型切换）
- `temperature`、`top_p`、`response_format`（`json_schema` / `json_object`，L172-185）
- `tools`/`tool_choice`、`reasoning_effort`、`service_tier`、`prompt_cache_*`
- reasoning 模型移除 `temperature`/`top_p`/`frequency_penalty` 等并发出 warning（L209-293）

消息转换见 [convert-to-openai-chat-messages.ts](../reference/ai/packages/openai/src/chat/convert-to-openai-chat-messages.ts)，system 消息支持 `system`/`developer`/`remove` 三种模式（L34），支持 `prompt_cache_breakpoint`。

### 工具格式

[openai-chat-prepare-tools.ts](../reference/ai/packages/openai/src/chat/openai-chat-prepare-tools.ts) L36-44 包装为：

```ts
{ type: 'function', function: { name, description, parameters: tool.inputSchema, ...(strict != null ? { strict } : {}) } }
```

`toolChoice` 映射为 `auto`/`none`/`required`/`{type:'function',function:{name}}`。

### SSE 流式解析

`doStream()`（L429-621）：

- `createEventSourceResponseHandler`（L452）解析 SSE
- endpoint `/chat/completions`（L353 doGenerate / L443 doStream）
- `TransformStream` 逐 chunk 处理 `choices[0].delta`：
  - `delta.content` → `text-start`/`text-delta`/`text-end`（L567-577）
  - `delta.tool_calls` 交给 `StreamingToolCallTracker` 按 index 累积 arguments（L582）
  - `delta.annotations` 转 `source`
  - `usage` 仅在末尾一次入队 `finish`
- `throwIfOpenAIStreamErrorBeforeOutput`（L459）确保首个 error chunk 提前抛出

## Anthropic 实现要点

- 主文件：[anthropic-language-model.ts](../reference/ai/packages/anthropic/src/anthropic-language-model.ts)（2942 行，单文件实现）
- 实现 `LanguageModelV4`：`specificationVersion = 'v4'`（L154-155）

### 请求构造

- `getArgs()` 方法起于 **L204**，止于 L835。其中 `baseArgs` body 对象字面量起于 **L484**，闭合约在 L640。
- body 构造关键行：
  - `max_tokens`（必填，L489；代码在 L482 提供回退 `maxOutputTokens ?? maxOutputTokensForModel`）
  - `temperature`/`top_k`/`top_p`/`stop_sequences`
  - `system` / `messages`（L591-592，从 `convertToAnthropicPrompt` 拆分）
  - `thinking`（L496-502）
  - `output_config`（含 `effort`/`task_budget`/结构化 `json_schema`，L503-530）
  - `mcp_servers`、`container`、`context_management`（L594+）
- 不支持 `frequencyPenalty`/`presencePenalty`/`seed`（L227-237 发出 warning）

消息格式：system 与 messages 分离。`convert-to-anthropic-prompt.ts` 将 prompt 拆为顶层 `system`（`AnthropicSystemMessage['content']` 数组，支持 `cache_control`、`toolChanges`）与 `messages`（`role: 'user'/'assistant'`）。

### 工具格式

[anthropic-prepare-tools.ts](../reference/ai/packages/anthropic/src/anthropic-prepare-tools.ts) L99-119 产出字段：

| 字段 | 行号 | 可选性 |
|------|------|--------|
| `name` | L100 | 必选 |
| `description` | L101 | 必选 |
| `input_schema` | L102 | 必选 |
| `cache_control` | L103 | 必选 |
| `eager_input_streaming?` | L104 | 可选 |
| `strict?` | L105-107 | 可选 |
| `defer_loading?` | L108 | 可选 |
| `allowed_callers?` | L109-111 | 可选 |
| `input_examples?` | L112-118 | 可选 |

支持 provider-defined tools（`case 'provider'` L132）：`code_execution_*`、`web_search_*`、`web_fetch_*`、`text_editor_*`（后者需 beta header）。工具名映射通过 `createToolNameMapping` 处理 MCP/server tool 名称冲突。

### 流式协议

- endpoint `/v1/messages`（`anthropic-provider.ts:26` baseURL 含 `/v1`，L874 追加 `/messages`）
- `doStream()`（L1506+）按 Anthropic 自定义事件解析：
  - `message_start`、`content_block_start`（按 index 维护 `contentBlocks[]`，case 起于 **L1638**，含 text/thinking/redacted_thinking/compaction/tool_use 等分支，延伸至 L1764+ 的 server_tool_use 等）
  - `content_block_delta`、`content_block_stop`、`message_delta`、`message_stop`
- `tool_use` 块通过 `toolCallTracker` 累积 JSON input 并发出 `tool-input-start/delta/end` 与 `tool-call`
- citations 与 `redactedData` 进 `providerMetadata`

## Google 实现要点

- 主文件：[google-language-model.ts](../reference/ai/packages/google/src/google-language-model.ts)（1611 行）
- 实现 `LanguageModelV4`：`specificationVersion = 'v4'`（L78-79）

### 请求构造

`getArgs()`（L114-376）产出 Gemini body：

- `generationConfig`（L328-350）：`maxOutputTokens`(L330)、`temperature`(L331)、`topK`(L332)、`topP`(L333)、`responseMimeType: 'application/json'`(L340-341)、`responseSchema`(L342)
  - schema 经 `convertJSONSchemaToOpenAPISchema` 转 OpenAPI（L349）
- `contents`、`systemInstruction`（Gemma 模型禁用，L260 `isGemmaModel` / L264 / L364）
- `safetySettings`（L299-306）
- `tools`/`toolConfig`（`functionCallingConfig.mode: VALIDATED/ANY/NONE`）
- `thinkingConfig`、`cachedContent`、`labels`、`serviceTier`

消息转换见 [convert-to-google-messages.ts](../reference/ai/packages/google/src/convert-to-google-messages.ts)：system 拆为 `systemInstruction`，user/model 用 `role:'user'/'model'` + `parts[]`，工具调用以 `functionCall`/`functionResponse` part 表示（L341/633/L351/L81/92）。

### 工具格式

[google-prepare-tools.ts](../reference/ai/packages/google/src/google-prepare-tools.ts) L175-188 将函数工具转为 `{ name, description, parameters(OpenAPI schema) }` 放入 `functionDeclarations`（另有一处同格式构造在 L236-245，为非 Gemini3 路径）。

### 流式协议

- endpoint `:streamGenerateContent?alt=sse`（L598），标准 SSE
- `GoogleJSONAccumulator`（定义于 `google-json-accumulator.ts:27`）跨 chunk 累积 `functionCall.args`：
  - 实例化于 transform 内 **L956**
  - 累积发生在 **L975**（`accumulator.processPartialArgs(partialArgs)`）
  - finalize 辅助函数 `finishActiveStreamingToolCall` 在 L639-673（L647 `active.accumulator.finalize()`），发 `tool-input-end` + `tool-call`
- `thought===true` 的 part 转 `reasoning`（L460-465）
- `executableCode`/`codeExecutionResult` 自动合成 `code_execution` tool-call + tool-result（L424-446）

## 三家差异对比表

| 维度 | OpenAI | Anthropic | Google Gemini |
|------|--------|-----------|---------------|
| **消息结构** | `messages: [{role, content}]`，system 内嵌为首条 | `system` 顶层 + `messages` 分离 | `contents:[{role:'user'\|'model',parts}]` + 顶层 `systemInstruction` |
| **system 模式** | `system`/`developer`/`remove` | 顶层 system 数组，支持 `cache_control`、`toolChanges` | `systemInstruction` 仅允许在对话开头 |
| **tool 定义** | `{type:'function',function:{name,description,parameters,strict}}` | `{name,description,input_schema,cache_control,strict?,eager_input_streaming?,defer_loading?,allowed_callers?,input_examples?}` + provider-defined tools | `{name,description,parameters(OpenAPI)}` 放入 `functionDeclarations` |
| **tool 调用返回** | `message.tool_calls[].function.arguments`(JSON 字符串) | `content_block` `tool_use` 块，input 分片流式累积 | `functionCall` part，args 用 `GoogleJSONAccumulator` 累积 |
| **流式协议** | 标准 SSE，`choices[0].delta`，usage 末段 | 自定义事件 `content_block_start/delta/stop`、`message_delta` | 标准 SSE `?alt=sse`，逐 chunk candidates parts |
| **endpoint** | `/chat/completions`（或 Responses API） | `/v1/messages` | `:generateContent` / `:streamGenerateContent?alt=sse` |
| **特有** | `reasoning_effort`、`prompt_cache_*`、`service_tier`、annotations→source | `thinking`、`mcp_servers`、`container`、`context_management`、citations | `safetySettings`、`thinkingConfig`、`cachedContent`、`retrievalConfig`、`mediaResolution` |

## Provider 生态全景

`packages/` 下共 **70** 个包（分类清单全部经核对存在）：

### 大模型 LLM（25）

`openai`、`anthropic`、`anthropic-aws`、`amazon-bedrock`、`google`、`google-vertex`、`azure`、`alibaba`、`mistral`、`cohere`、`deepseek`、`groq`、`cerebras`、`fireworks`、`togetherai`、`deepinfra`、`baseten`、`huggingface`、`perplexity`、`xai`、`moonshotai`、`bytedance`、`quiverai`、`openai-compatible`、`vercel`

### 图像/视频生成（6）

`black-forest-labs`、`fal`、`klingai`、`luma`、`prodia`、`replicate`

### 语音/转写（8）

`elevenlabs`、`cartesia`、`lmnt`、`hume`、`deepgram`、`assemblyai`、`gladia`、`revai`

### 嵌入（1）

`voyage`

### 网关/兼容层（2）

`gateway`、`open-responses`

### 核心与基础设施（6）

`ai`（核心 SDK）、`provider`（接口定义）、`provider-utils`、`valibot`、`otel`、`mcp`

### 框架 UI 绑定（5）

`angular`、`react`、`svelte`、`vue`、`rsc`

### Agent/Sandbox（14）

`harness`、`harness-claude-code`、`harness-codex`、`harness-deepagents`、`harness-opencode`、`harness-pi`、`workflow`、`workflow-harness`、`sandbox-just-bash`、`sandbox-vercel`、`tui`、`devtools`、`test-server`、`codemod`

### 集成/策略（3）

`langchain`、`llamaindex`、`policy-opa`

---

## 复核记录

**复核员**：Frank | **复核方式**：逐条打开源码验证行号/行数/数字

| # | 声明 | 结论 | 证据/修正 |
|---|------|------|-----------|
| 1 | OpenAI chat 行号 | ✅ | 638 行；spec L55-56；getArgs L89-340；response_format L172-185；doStream L429-621；throwIfOpenAIStreamErrorBeforeOutput L459 — 全部精确 |
| 2 | OpenAI 工具格式 | ✅ | openai-chat-prepare-tools.ts L36-44 格式准确 |
| 3 | Anthropic 行号 | ⚠️→✅ 已修正 | 2942 行 ✅；spec L154-155 ✅；但 getArgs 实际起于 L204（非 484-592），484 是 baseArgs body 对象起始，闭合约 L640；content_block_start case 起于 L1638 ✅，但止点延伸至 L1764+（非 1749） |
| 4 | Anthropic 工具字段 | ⚠️→✅ 已修正 | 所列 8 字段全对；已补充遗漏的 `input_examples`（L112-118） |
| 5 | Google 行号 | ⚠️→✅ 已修正 | 1611 行 ✅；spec L78-79 ✅；getArgs L114-376 ✅；GoogleJSONAccumulator 累积点修正为 L956(实例化)/L975(processPartialArgs)，L639-673 是 finalize 辅助函数而非累积处 |
| 6 | Google 工具格式 | ✅ | google-prepare-tools.ts L175-188 格式准确 |
| 7 | 包总数 | ❌→✅ 已修正 | 报告称 71，实际 **70**；分类清单全部正确 |
| 8 | endpoint 路径 | ✅ | /chat/completions(L353/443)、/v1/messages(provider.ts:26+L874)、:streamGenerateContent?alt=sse(L598) 全部准确 |
