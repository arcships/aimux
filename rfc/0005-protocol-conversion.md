# 协议转换与适配层设计

> 扫描了 reference/ 下 104 个项目，记录各项目的协议转换逻辑和 provider 适配层设计。
> 重点不是厂商清单（见 [0004-provider-inventory.md](0004-provider-inventory.md)），而是**怎么统一不同厂商的协议**。

## 一、aimux 现状

aimux 的适配层在 [aimux-providers/src/openai/](aimux-providers/src/openai/)：
- `model.rs` 的 `execute_generate`/`execute_stream` 是共享入口（free function），Azure 已复用。
- `convert.rs` 把统一消息格式转成 OpenAI 请求体，解析响应回 `GenerateResult`/`StreamPart`。
- 13 个薄封装（groq/deepseek/...）只改网址，不定制。
- 原生协议（anthropic/google/bedrock/cohere/mistral）各有独立 model+convert，不共用 OpenAI 逻辑。
- **没有跨协议转换**（不做 OpenAI↔Anthropic 互转，各厂商只管自己协议的请求构造和响应解析）。

## 二、参考项目的适配层设计模式

### 模式一：OpenAI 兼容共享层（rig / rust-genai / llm-connector / litellm）

把 OpenAI Chat Completions 的请求构造、SSE 解析、工具调用聚合等公共逻辑抽出，兼容厂商只填配置差异。

**rig**（Rust，非 object-safe）：
- `OpenAICompatibleProvider` trait 暴露 const 能力开关（`SUPPORTS_TOOLS`、`SUPPORTS_RESPONSE_FORMAT`、`STREAM_INCLUDE_USAGE`）+ hook 方法。
- `CompatibleStreamProfile` trait + 泛型驱动函数 `send_compatible_streaming_request<T,P>`，共享流式状态机。
- 16 个兼容厂商共用，各自填 const 和 hook。
- 原生协议（Anthropic/Gemini/Bedrock）有平行 trait，不共用 OpenAI 层。

**rust-genai**（Rust）：
- `Adapter` trait 全静态方法 + `dispatch_adapter!` 宏编译期分发。
- `adapter_shared.rs` 是 OpenAI 公共请求构造入口。
- `impl_pass_through_adapter!` 宏生成委托实现（如 MiniMax 委托 Anthropic）。
- `unsupported: [embeddings]` 声明不支持的能力。

**llm-connector**（Rust）：
- `Protocol` trait + 关联类型（`type Request`/`type Response`）。
- `OpenAICompatibleCapabilities` 能力位结构（`content_block_mode`/`supports_tool_choice`/`reasoning_request_strategy` 等）。
- 无状态共享函数（`build_openai_compatible_request_parts`），不是继承。

**litellm**（Python）：
- `BaseConfig` 基类定义 `transform_request`/`transform_response` 抽象方法 + 能力 hook（`should_fake_stream`/`sign_request`/`get_complete_url`）。
- `OpenAIGPTConfig` 被 ~30 个兼容厂商子类化。
- 原生协议（Anthropic/Cohere/Gemini）直接继承 `BaseConfig`。

> **对 aimux 的启示**：RFC-0002 提的"配置描述结构"就是这个模式——但 aimux 要保持 object-safe，不能用 rig 的泛型 trait。litellm 的 `BaseConfig` 继承模式最接近 aimux 能用的方案：一个结构体描述差异，共享函数读它决定行为。

### 模式二：委托外部 SDK（opencode / aider / mastra）

不自己写协议转换，直接用别人 的 provider 层。

**opencode**（190k star）：委托 Vercel AI SDK（`@ai-sdk/*`），自己只做怪癖补丁（`transform.ts` 的 `normalizeMessages`/`sdkKey`/模型特定处理）。

**aider**：完全委托 litellm，自己零协议代码。

**mastra**：复用 Vercel AI SDK provider 层。

> aimux 走的是自研路线，不委托。但 opencode 的"怪癖补丁"思路有参考价值——薄封装的配置描述本质上就是怪癖补丁。

### 模式三：自实现协议适配器（pi / continue / Roo-Code / opencodex）

自己写每家厂商的协议适配，不依赖外部 SDK。

**pi**（79k star，TypeScript）：
- 10 种 API 适配器（`api/anthropic-messages.ts`、`api/openai-completions.ts`、`api/openai-responses.ts`、`api/bedrock-converse-stream.ts`、`api/google-generative-ai.ts` 等）。
- 一个 provider 可挂多 API（如 github-copilot 同时支持 anthropic-messages/openai-completions/openai-responses）。
- 37 个内置 provider。

**continue**（35k star，TypeScript）：
- 独立包 `@continuedev/openai-adapters`，每厂商一个适配器文件。
- 66 个 provider 类。
- `openaiToVercelMessages.ts`/`convertToolsToVercel.ts` 做 Vercel AI SDK 互转。

**Roo-Code**（24k star，TypeScript）：
- 内部规范格式 = **Anthropic Messages**（不是 OpenAI）。
- `transform/` 目录做 Anthropic→各厂商：`openai-format.ts`、`gemini-format.ts`、`bedrock-converse-format.ts`、`mistral-format.ts`、`minimax-format.ts`、`zai-format.ts`。
- `reasoning.rs` 做跨厂商 reasoning 格式转换。

> **关键差异**：aimux 的规范格式是自己的 `LanguageModelPrompt`，各厂商 convert 成自己的格式。Roo-Code 选 Anthropic 作规范格式，pi 选自己的 `pi-messages`。aimux 的选择和 pi 类似——自有中间格式。

## 三、网关项目的协议转换（跨协议互转）

网关项目做的是 SDK 不做的事：**让用户用 A 协议发请求，后端用 B 协议调厂商**。比如用户发 OpenAI 格式，后端调 Anthropic。

### 转换架构分三类

**全连接 mesh（new-api）**：
- 定义 `RelayFormat` 枚举：openai/claude/gemini/openai_responses。
- 注册表 + 多步链式转换：A→C 可自动走 A→B→C。
- 转换器按 `{from}_to_{to}` 文件组织（`oai_chat/to_claude_messages_req.go` 等）。
- 质量分级：good/fair/discouraged。
- **最完整的协议转换实现**，4 种格式任意两两互转。

**Hub-and-spoke，OpenAI 为中心（one-api / portkey / aiproxy / simple-one-api / ferro）**：
- 入站统一为 OpenAI 格式，各厂商 adaptor 把 OpenAI 转成原生格式。
- one-api 不支持 Anthropic/Gemini 作为入站格式。
- portkey 支持多入站 endpoint（OpenAI Chat / Anthropic Messages / Responses），但内部统一转成 OpenAI 中间表示。

**N×M 矩阵（envoy-ai-gateway / axonhub）**：
- envoy：入站协议 × 后端 schema 组合选择 translator，文件名即转换对（`openai_awsbedrock.go`、`anthropic_openai.go`）。
- axonhub：Inbound transformer（客户端→统一 IR）× Outbound transformer（统一 IR→provider），经统一 `llm.Request`/`llm.Response` 互转。

### 关键转换对的支持情况

| 转换对 | new-api | bifrost | envoy | axonhub | higress | portkey | opencodex |
|--------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| OpenAI Chat ↔ Anthropic Messages | ✅直转 | ✅ | ✅ | ✅ | ✅(46KB) | ✅ | ✅(经内部Responses) |
| OpenAI Chat ↔ Gemini | ✅直转 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| OpenAI Responses ↔ Chat | ✅双向 | ✅原生 | ✅入站 | ✅完整 | ❌ | ✅入站 | ✅ |
| Anthropic Messages → Gemini | ✅直转 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Bedrock event stream → SSE | aws channel | ✅eventstream | ✅eventstream | ✅ | ✅(56KB) | bedrock | — |

### 流式转换

所有项目都用**有状态机逐 chunk 转换**：
- new-api：`StreamState` 三段式（`NewStreamState`/`ConvertStreamChunk`/`FinalizeStream`）。
- higress：`ClaudeToOpenAIConverter` 维护 `messageStartSent`/`thinkingBlockIndex`/`toolBlockIndex`/`toolCallStates`。
- portkey：`AnthropicStreamState`。
- bifrost：`chan *BifrostStreamChunk`。
- envoy：extproc 流式 body 逐块翻译。

> **这是跨协议流式的共性难点**：不能简单逐条翻译 SSE 事件，要维护状态（当前在哪个 content block、工具调用分片聚合、thinking 块开始/结束）。aimux 的 `StreamPart` 已经有三段式（Start/Delta/End），如果将来要做跨协议转换，这个状态机设计是基础。

### 工具调用转换

普遍在各 provider 的请求/响应转换函数中处理，无独立统一层（除 new-api 的 `shared/claude/tool_choice.go` 和 axonhub 的 `tools.go`/`tool_blocks.go`）。

转换对：
- OpenAI `tool_calls`（`function.arguments` 是 JSON 字符串）↔ Anthropic `tool_use`（`input` 是 JSON 对象）↔ Gemini `functionCall`（`args` 是 JSON 对象）
- 参数类型差异：OpenAI 是 string，Anthropic/Gemini 是 object，需要 `serde_json::from_str`/`to_string` 转换。

## 四、coding agent 与转发服务的协议转换

### 真协议转换代理

**opencodex**（5.2k star，协议转换最完整的转发服务）：
- 内部中间表示 + 双向适配器架构。
- 入站：Codex（OpenAI Responses）→ 直接进内部表示；Claude Code（Anthropic Messages）→ `claude/inbound.ts` 转成内部 Responses body。
- 出站适配器（`adapters/`）：anthropic.ts、openai-chat.ts、openai-responses.ts、google.ts、azure.ts、kiro.ts、cursor.ts（protobuf）。
- 回程桥接：`bridge.ts` 转回 Responses SSE（给 Codex）或 Anthropic Messages SSE（给 Claude Code）。
- 60 个 provider entry。
- OAuth 支持：Codex 订阅、Claude 订阅、GitHub Copilot、Grok Build、Kiro、Antigravity、Cursor。

**claude-worker-proxy**（Cloudflare Worker）：
- Anthropic Messages → OpenAI Chat / Gemini / OpenAI Responses，双向。
- 轻量，单文件，无账号池。

**ccswitch-deepseek**：
- OpenAI Responses → DeepSeek Chat Completions，单向。
- 含 DeepSeek thinking 模式多轮 reasoning 恢复（`recover.js`）。

### 配置切换器（无协议转换）

**cc-switch**（122k star，Rust+Tauri）：
- 改写各 app 的配置文件（`ANTHROPIC_BASE_URL`/`AUTH_TOKEN`/`ANTHROPIC_MODEL`）。
- 可选代理只做模型名映射（haiku/sonnet/opus → 厂商模型名），不改消息体。
- 80+ 家 provider preset。
- 支持 Claude/Codex/Copilot/Grok Build 订阅 OAuth。

**claude-code-router**（36k star）：
- 按客户端协议原样转发到声明了该协议能力的 provider。
- 内核不做 Anthropic↔OpenAI 消息体转换，靠用户写 route script 实现。
- 凭据池（多 key 轮换/冷却/限流）。

**CCSwitcher**（macOS）：
- Claude Code OAuth 账号池切换（Keychain 管理）。
- 无协议转换。

## 五、aimux 与参考项目的差异

| 维度 | aimux | rig | litellm | new-api | opencodex |
|------|---------|-----|---------|---------|-----------|
| 定位 | 服务接入统一层 | Rust LLM 框架 | Python 网关+库 | Go 网关 | 转发代理 |
| 规范格式 | 自有 `LanguageModelPrompt` | 自有 `CompletionRequest` | 自有 `BaseConfig` | OpenAI/Anthropic/Gemini 多格式 | 自有内部 Responses |
| OpenAI 兼容共享 | `execute_generate` free function | `OpenAICompatibleProvider` trait | `OpenAIGPTConfig` 子类化 | relayconvert 注册表 | adapters/openai-chat.ts |
| 跨协议转换 | ❌ 不做 | ❌ 不做 | ❌ 不做 | ✅ 全 mesh | ✅ 经内部中间层 |
| object-safe | ✅ | ❌ | — | — | — |
| 流式状态机 | `StreamPart` 三段式 | `StreamingCompletionResponse` 自带聚合 | provider 各自实现 | `StreamState` 三段式 | bridge 回程桥接 |

**核心结论**：aimux 不做跨协议转换（网关的活），只做"统一接口调各厂商"（SDK 的活）。适配层的改进方向是 RFC-0002 的配置描述结构——让薄封装能表达差异，同时保持 object-safe。

## 六、数据来源

本文档来自以下扫描（2026-07-28）：

- **SDK 协议转换**：rig、rust-genai、llm-connector、edgequake-llm、litellm、pydantic-ai、instructor、eino、langchaingo、langchain4j
- **网关协议转换**：new-api、one-api、portkey-gateway、bifrost、higress、axonhub、ferro-ai-gateway、APIPark、envoy-ai-gateway、uni-api、simple-one-api、aiproxy
- **coding agent provider**：codex、opencode、pi、gemini-cli、cline、aider、continue、Roo-Code、opencode-ai
- **转发服务**：opencodex、claude-code-router、cc-switch、claude-worker-proxy、ccswitch-deepseek、ccs-nicremo、CCSwitcher、oh-my-opencode-slim、agent-of-empires、pinchbench
