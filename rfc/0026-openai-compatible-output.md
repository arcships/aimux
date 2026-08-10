# RFC-0026: OpenAI Compatible 输出格式

> 状态:已实现(第一期核心转换层已落地:`aimux-core/src/openai_output.rs` 提供 `to_chat_completion`/`to_chat_completion_stream` + `generate.rs` 的 `generate_text_as_openai`/`stream_text_as_openai`;见 §8 实现计划)
> 日期:2026-08-05
> 依赖:RFC-0005(协议转换)、RFC-0016(对齐 AI SDK)

## 1. 背景与动机

aimux 当前的默认(且唯一)结构化输出格式对齐 Vercel AI SDK V4——
`GenerateResult`(非流式)与 `StreamPart`(流式)。任何 provider 经
`LanguageModel::do_generate` / `do_stream` 后都被统一成这套格式。

但 OpenAI Chat Completions 是 LLM 生态事实上的"通用语":
- 绝大多数 provider(aimux 的 250 个 registry provider)原生就是 OpenAI 格式;
- 大量下游工具(OpenAI SDK、LangChain、各 IDE 插件)只认 OpenAI 格式;
- 网关项目(new-api、one-api、portkey)都以 OpenAI 格式为入站/出站标准。

用户希望 aimux 也能输出 OpenAI Chat Completions 结构,使得:

1. **不管上游接的是什么 provider**(OpenAI / Anthropic / Google / DeepSeek / …),
   下游都能用统一的 OpenAI 数据结构接入;
2. **第一期**:OpenAI 与 OpenAI-compatible provider 能直接用 OpenAI 数据结构。

## 2. 需求拆解

| # | 需求 | 范围 |
|---|------|------|
| R1 | 任何 provider 的非流式结果 → `ChatCompletion` JSON | 完整覆盖 |
| R2 | 任何 provider 的流式结果 → `ChatCompletionChunk` SSE 流 | 完整覆盖 |
| R3 | 所有 `StreamPart` / `GenerateContent` 变体都有明确映射 | 完整覆盖 |
| R4 | 第一期:OpenAI / OpenAI-compatible provider 开箱可用 | 第一期 |
| R5 | 转换是纯函数,8 个 binding 共享同一份 Rust 实现 | 架构约束 |

**非目标(本 RFC 不做)**:
- 不做 HTTP 网关(server)。aimux 是 SDK/访问层,不是网关。网关是 new-api 的事。
- 不做 OpenAI 格式**入站**(即不解析 OpenAI 请求体作为 prompt)——aimux 已有
  `ModelMessage` 作为入站格式;入站 OpenAI 解析是后续可选项。
- 不做直通/零拷贝优化(第二期考虑,见 §10)。

## 3. 参考项目研究总结

研究基于 clone 到 `reference/` 的 4 个项目 + aimux 现有代码。详细报告见研究记录,
此处仅列对设计有直接影响的结论。

### 3.1 aimux 现有基线(反向转换的参照)

aimux 已有完整的 **OpenAI → aimux 内部格式** 解析逻辑,这是正向转换的镜像基线:

| 方向 | 代码位置 | 说明 |
|------|----------|------|
| OpenAI 非流式响应 → `GenerateResult` | [aimux-providers/src/openai/model.rs:253](../aimux-providers/src/openai/model.rs#L253) `execute_generate` | 解析 `ChatCompletionResponse` → `GenerateContent` 数组 |
| OpenAI 流式 SSE → `StreamPart` | [aimux-providers/src/openai/model.rs:404](../aimux-providers/src/openai/model.rs#L404) `execute_stream` | 状态机:`text_started`/`reasoning_started`/`tool_calls` HashMap |
| OpenAI 类型定义(入站解析用) | [aimux-providers/src/openai/types.rs](../aimux-providers/src/openai/types.rs) | `ChatCompletionResponse`/`StreamChunk`/`UsageResponse`/`Delta` |
| usage 转换 OpenAI→aimux | [model.rs:125](../aimux-providers/src/openai/model.rs#L125) `convert_usage` | `prompt_tokens`→`input_tokens.total` 等 |
| finish_reason 解析 | [convert.rs:1483](../aimux-providers/src/openai/convert.rs#L1483) `parse_finish_reason` | `stop`→`Stop` 等 |

**关键洞察**:aimux 的 `convert_usage` 和 `parse_finish_reason` 是可逆的——
逆向函数(`aimux → OpenAI`)可直接据此实现。

### 3.2 new-api(Go 网关,最完整的协议互转)

- OpenAI Chat 的 Go DTO(`GeneralOpenAIRequest`/`OpenAITextResponse`/
  `ChatCompletionsStreamResponse`)可作为 Rust 结构体的蓝本。
- 流式状态机有两种范式:Claude 的"外部 state struct + 无状态逐帧函数"
  和 Gemini 的"自包含 state object"——**后者更干净,aimux 仿照之**
  (参考 `GeminiToChatStreamState`)。
- 末帧 usage 约定:仅当 `stream_options.include_usage=true` 时,最后发一帧
  `choices:[]` + `usage`,然后 `data: [DONE]`。
- reasoning 字段决策:new-api 用**非标准** `reasoning_content`(兼容
  OpenRouter/DeepSeek 生态),同时保留 `reasoning` 别名。

### 3.3 opencodex(TypeScript 转发代理)

- [reference/opencodex/src/chat/outbound.ts](../reference/opencodex/src/chat/outbound.ts)
  展示了完整的"内部 Responses 格式 → Chat Completions SSE/JSON"转换。
- `responsesJsonToChatCompletion`(L553):非流式,遍历 output 数组拼
  `content`/`reasoning_content`/`tool_calls`。
- `responsesSseToChatCompletionsSse`(L155):流式,`ensureRole()` 惰性发首帧,
  `emitContent`/`emitReasoning`/`emitToolCall` 分别发 delta 帧,
  `finish()` 发终止帧 + `[DONE]`。
- tool_call 流式 index 管理:`toolIndexByCallId` Map 保证稳定 index。

### 3.4 ccswitch-deepseek(Node.js 代理)

- `buildNonStreamResponse`([index.js:54](../reference/ccswitch-deepseek/index.js#L54)):
  Chat → Responses 的拆解,给出字段对照表。
- `SseTranslator`([lib/sse.js](../reference/ccswitch-deepseek/lib/sse.js)):
  "每类输出:首个 delta 触发 open → 后续 delta 发增量 → done 发 close"
  的懒启动模型,可复用。
- **缺口警示**:ccswitch-deepseek **未做 `finish_reason` 映射**,aimux 须补上。

### 3.5 cc-switch(Rust+Tauri 配置切换器)

- `model_mapper.rs` **只改 `body["model"]`**([forwarder.rs:1167](../reference/cc-switch/src-tauri/src/proxy/forwarder.rs#L1167)),
  不碰消息体——**关注点分离**原则,aimux 的输出格式器也应只做结构重组。
- `recover.js` 的 `sessionKey` 恒为 `"g"`、内存不持久是**反例**,
  aimux 若做多轮 reasoning 恢复不可照搬。

## 4. 设计方案

### 4.1 模块划分

新增 `aimux-core/src/openai_output.rs` 模块,提供纯转换函数:

```
aimux-core/src/
├── openai_output.rs      ← 新增:aimux → OpenAI Chat Completions 转换层
│   ├── 类型定义(ChatCompletion / ChatCompletionChunk / Usage …)
│   ├── 非流式转换:GenerateResult → ChatCompletion
│   ├── 流式转换:Stream<StreamPart> → Stream<ChatCompletionChunk>
│   └── SSE 编码(可选:chunk → "data: {json}\n\n" 字符串)
├── generate.rs           ← 扩展:新增 generate_text_openai / stream_text_openai
└── lib.rs                ← 扩展:pub mod openai_output
```

**为什么放在 aimux-core 而非 aimux-providers**:
转换的输入是 `GenerateResult`/`StreamPart`(aimux-core 的类型),不依赖任何
provider 实现。放 aimux-core 让 8 个 binding 都能直接调用。

### 4.2 类型定义

> 参考 new-api `relaykit/dto/openai_response.go` + aimux 现有
> `openai/types.rs`,全部 `Serialize`。

```rust
// ── 非流式响应 ──

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletion {
    pub id: String,
    pub object: String,              // "chat.completion"
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: ChatCompletionUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatCompletionMessage,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionMessage {
    pub role: String,                // "assistant"
    pub content: Option<String>,     // None → null(有 tool_calls 时)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,           // "function"
    pub function: ChatCompletionFunction,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionFunction {
    pub name: String,
    pub arguments: String,           // JSON string(非 object)
}
```

```rust
// ── 流式响应(chunk)──

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,              // "chat.completion.chunk"
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatCompletionUsage>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkChoice {
    pub index: u32,
    pub delta: ChatCompletionDelta,
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionChunkToolCall>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkToolCall {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    pub function: ChatCompletionChunkFunction,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionChunkFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}
```

```rust
// ── Usage(共用)──

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct ChatCompletionUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct PromptTokensDetails {
    pub cached_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct CompletionTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
}
```

### 4.3 非流式转换:`GenerateResult → ChatCompletion`

```rust
pub fn to_chat_completion(result: &GenerateResult, model: &str) -> ChatCompletion
```

遍历 `result.content: Vec<GenerateContent>`,按类型累积:

| `GenerateContent` 变体 | → ChatCompletion 字段 | 处理 |
|------------------------|----------------------|------|
| `Text { text }` | `message.content` | 字符串拼接 |
| `Reasoning { text }` | `message.reasoning_content` | 字符串拼接 |
| `ToolCall { tool_call_id, tool_name, input }` | `message.tool_calls[]` | `arguments = serde_json::to_string(&input)` |
| `Source { url, title }` | `message.annotations[]` + `content` | 映射为 `url_citation` annotation(OpenAI 格式),URL 同时追加到 content |
| `File { data, media_type }` | `message.content` | 降级:URL 直接放 content;base64 编为 data URI |
| `ToolResult { tool_call_id, result, .. }` | `message.content` | 降级:`serde_json::to_string(&result)` 放 content(provider-executed 工具结果) |

**finish_reason 映射**(`result.finish_reason.unified`):

| aimux `FinishReasonUnified` | OpenAI `finish_reason` |
|-----------------------------|------------------------|
| `Stop` | `"stop"` |
| `Length` | `"length"` |
| `ContentFilter` | `"content_filter"` |
| `ToolCalls` | `"tool_calls"` |
| `Error` | `"stop"` + 在 content 追加错误说明 |
| `Other` | `raw`(若有)否则 `"stop"` |

**usage 映射**(`result.usage` → `ChatCompletionUsage`):

```
prompt_tokens  = input_tokens.total
completion_tokens = output_tokens.total
total_tokens   = prompt_tokens + completion_tokens
prompt_tokens_details.cached_tokens = input_tokens.cache_read
prompt_tokens_details.cache_write_tokens = input_tokens.cache_write
completion_tokens_details.reasoning_tokens = output_tokens.reasoning
```

> 这是 [model.rs `convert_usage`](../aimux-providers/src/openai/model.rs#L125)
> 的逆函数。aimux 的 `convert_usage` 把 `prompt_tokens` 拆成
> `total`/`no_cache`/`cache_read`/`cache_write`,逆函数把它们合回去。

**id / model / created**:
- `id` = `result.response.id`(若有)否则生成 `chatcmpl-{uuid}`
- `model` = `model` 参数(或 `result.response.model_id`)
- `created` = 当前 Unix 时间戳(或从 `result.response.timestamp` 解析)

**logprobs**:从 `result.provider_metadata["openai"]["logprobs"]` 取回。

### 4.4 流式转换:`Stream<StreamPart> → Stream<ChatCompletionChunk>`

```rust
pub fn to_chat_completion_stream(
    stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>,
    model: &str,
    options: OpenAiStreamOptions,
) -> Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, AiMuxError>> + Send>>

pub struct OpenAiStreamOptions {
    /// 是否在末帧带 usage(对应 stream_options.include_usage)。
    pub include_usage: bool,
    /// 是否输出 reasoning_content 字段(默认 true)。
    pub include_reasoning: bool,
}
```

#### 状态机

参考 new-api `GeminiToChatStreamState`(自包含 state object):

```rust
struct ChatCompletionStreamState {
    id: String,
    model: String,
    created: u64,
    started: bool,               // 是否已发首帧(role:assistant)
    // tool_call 累积:key = tool_call_id
    tool_calls: HashMap<String, ToolCallAccum>,
    tool_call_order: Vec<String>,
    next_tool_index: u32,
    final_usage: Option<Usage>,
    final_finish_reason: Option<FinishReason>,
    finish_emitted: bool,
}

struct ToolCallAccum {
    index: u32,
    id: String,
    name: String,
    arguments: String,
}
```

#### StreamPart → ChatCompletionChunk 映射

| `StreamPart` 变体 | 产出 chunk | 状态更新 |
|-------------------|-----------|----------|
| `StreamStart` | 首帧 `{delta:{role:"assistant", content:""}}` | `started=true` |
| `ResponseMetadata { id, model_id }` | (不发 chunk) | 记录 id/model |
| `ReasoningStart { id }` | (不发 chunk) | — |
| `ReasoningDelta { delta }` | `{delta:{reasoning_content: delta}}` | — |
| `ReasoningEnd { id }` | (不发 chunk) | — |
| `TextStart { id }` | 确保 `started`(惰性首帧) | — |
| `TextDelta { delta }` | `{delta:{content: delta}}` | — |
| `TextEnd { id }` | (不发 chunk) | — |
| `ToolInputStart { id, tool_name }` | `{delta:{tool_calls:[{index, id, type:"function", function:{name, arguments:""}}]}}` | 分配 index,记录 tool_call |
| `ToolInputDelta { id, delta }` | `{delta:{tool_calls:[{index, function:{arguments: delta}}]}}` | 累积 arguments |
| `ToolInputEnd { id }` | (不发 chunk) | — |
| `ToolCall { tool_call_id, tool_name, input }` | 若未通过 Start/Delta 流过,发完整 tool_call chunk | 同 ToolInputStart(分配 index) |
| `ToolResult { .. }` | 降级为 `{delta:{content: ...}}` | — |
| `File { data, media_type }` | 降级为 `{delta:{content: <url 或 dataURI>}}` | — |
| `Source { url, title }` | 降级为 `{delta:{content: <url>}}` | — |
| `Finish { finish_reason, usage }` | 终止帧 `{delta:{}, finish_reason, usage?}` | `finish_emitted=true` |
| `Error { error }` | `{delta:{content: "[error] ..."}, finish_reason:"stop"}` | — |
| `Raw { .. }` | (忽略) | — |

#### 首帧策略(惰性)

参考 opencodex `ensureRole()`:首个内容 delta 到来时才发
`{delta:{role:"assistant", content:""}}`。`StreamStart` 可直接触发首帧。

#### 末帧策略

OpenAI 约定:
1. 终止帧:`{choices:[{index:0, delta:{}, finish_reason:"..."}]}`,
   若 `include_usage` 则带 `usage`。
2. (可选)若 `include_usage` 且终止帧已发,再发一帧空 choices + usage
   (对应 OpenAI 的 `choices:[]` + usage 末帧)。**aimux 采用简化方案**:
   把 finish_reason 和 usage 放同一帧。
3. 最后 SSE 编码层追加 `data: [DONE]`(见 §4.5)。

#### tool_call index 稳定性

OpenAI 流式要求每个 tool_call 有稳定的 `index`。aimux 的 `ToolInputStart`
带 `id`(tool_call_id),以此为 key 分配递增 index,保证:
- 同一 tool_call 的所有 delta 用同一 index;
- 多个 tool_call 按 Start 出现顺序分配 0,1,2,…

### 4.5 SSE 编码(可选辅助)

转换层产出 `ChatCompletionChunk` 结构体。对于需要直接输出 SSE 字节流的场景
(如 binding 层转发),提供辅助函数:

```rust
/// 把 chunk 编码为 SSE data 行:"data: {json}\n\n"
pub fn encode_chunk_sse(chunk: &ChatCompletionChunk) -> String

/// 末尾的 "data: [DONE]\n\n"
pub const DONE_FRAME: &str = "data: [DONE]\n\n";
```

binding 层可据此把 chunk 流转成 `data: {...}\n\n ... data: [DONE]\n\n` 字节流。

### 4.6 用户 API 形态

在 `generate.rs` 新增便利函数(`generate_text` / `stream_text` 的 OpenAI 输出版):

```rust
/// 非流式:返回 ChatCompletion JSON 结构。
pub async fn generate_text_as_openai(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
) -> Result<ChatCompletion, AiMuxError>

/// 流式:返回 ChatCompletionChunk 流。
pub async fn stream_text_as_openai(
    model: &dyn LanguageModel,
    prompt: impl Into<ModelPrompt>,
    options: GenerateTextOptions,
    stream_options: OpenAiStreamOptions,
) -> Result<ChatCompletionStream, AiMuxError>
```

底层实现:
1. 调 `generate_text` / `stream_text` 拿到 `GenerateTextResult` / `StreamTextResult`;
2. 调 `openai_output::to_chat_completion` / `to_chat_completion_stream` 转换。

> 也可在 `GenerateTextOptions` 加 `output_format: Option<OutputFormat>` 枚举,
> 但独立函数更显式、不污染现有 API。**推荐独立函数**。

### 4.7 binding 暴露

以 Node.js 为例(`bindings/node/src/index.ts`):

```typescript
export async function generateTextAsOpenAI(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions,
  signal?: AbortSignal,
): Promise<ChatCompletion>

export async function* streamTextAsOpenAI(
  model: RawModel,
  prompt: string | ModelMessage[],
  options?: GenerateTextOptions & { includeUsage?: boolean },
  signal?: AbortSignal,
): AsyncGenerator<ChatCompletionChunk>
```

其他 binding(Python/Go/Swift/Kotlin/Java/Flutter/C)按各自 FFI 约定暴露。
Rust core 一份实现,8 个 binding 共享。

## 5. 完整转换映射表

### 5.1 非流式 GenerateContent → ChatCompletionMessage

| aimux 内容类型 | OpenAI 字段 | 映射规则 | 有损? |
|---------------|-------------|----------|--------|
| `Text` | `content` | 拼接 | 无损 |
| `Reasoning` | `reasoning_content` | 拼接(非标准扩展) | 语义保留 |
| `ToolCall` | `tool_calls[]` | `input`(Value)→`arguments`(JSON string) | 无损 |
| `Source` | `annotations[]` + `content` | `url_citation` annotation | 无损 |
| `File` | `content` | URL 直放;base64→data URI | 介质类型丢失 |
| `ToolResult` | `content` | `to_string(result)` | 有损(降级) |

### 5.2 流式 StreamPart → ChatCompletionChunk

| aimux StreamPart | OpenAI delta | 发 chunk? | 说明 |
|-----------------|--------------|-----------|------|
| `StreamStart` | `{role:"assistant",content:""}` | ✅ 首帧 | 或惰性到首个 delta |
| `ResponseMetadata` | — | ❌ | 记录 id/model |
| `TextStart` | — | ❌ | 确保首帧 |
| `TextDelta` | `{content}` | ✅ | — |
| `TextEnd` | — | ❌ | — |
| `ReasoningStart` | — | ❌ | — |
| `ReasoningDelta` | `{reasoning_content}` | ✅ | — |
| `ReasoningEnd` | — | ❌ | — |
| `ToolInputStart` | `{tool_calls:[{index,id,type,function:{name}}]}` | ✅ | 分配 index |
| `ToolInputDelta` | `{tool_calls:[{index,function:{arguments}}]}` | ✅ | 增量 |
| `ToolInputEnd` | — | ❌ | — |
| `ToolCall`(完整) | 完整 tool_calls delta | ✅ | 未流式过的兜底 |
| `ToolResult` | `{content}` | ✅ 降级 | provider-executed |
| `File` | `{content}` | ✅ 降级 | — |
| `Source` | `{content}` | ✅ 降级 | — |
| `Finish` | `{}`+`finish_reason`+`usage` | ✅ 终止帧 | — |
| `Error` | `{content}`+`finish_reason:"stop"` | ✅ | — |
| `Raw` | — | ❌ | 忽略 |

### 5.3 finish_reason 双向映射

| aimux `FinishReasonUnified` | OpenAI `finish_reason` |
|-----------------------------|------------------------|
| `Stop` | `stop` |
| `Length` | `length` |
| `ContentFilter` | `content_filter` |
| `ToolCalls` | `tool_calls` |
| `Error` | `stop`(content 带错误说明) |
| `Other` | `raw` 或 `stop` |

> 这是 [convert.rs `parse_finish_reason`](../aimux-providers/src/openai/convert.rs#L1483)
> 的逆映射。

### 5.4 usage 双向映射

aimux `Usage` ↔ OpenAI `ChatCompletionUsage`:

```
prompt_tokens      ⇄ input_tokens.total
completion_tokens  ⇄ output_tokens.total
total_tokens       = prompt_tokens + completion_tokens
cached_tokens      ⇄ input_tokens.cache_read
cache_write_tokens ⇄ input_tokens.cache_write
reasoning_tokens   ⇄ output_tokens.reasoning
```

> 这是 [model.rs `convert_usage`](../aimux-providers/src/openai/model.rs#L125)
> 的逆映射。注意 `no_cache = prompt - cached - cache_write`(saturating)。

## 6. 关键难点与 quirk

基于参考项目研究,以下是最容易出错的点(均有专门测试覆盖):

1. **tool_call.arguments 是 JSON string,aimux ToolCall.input 是 Value**
   - 非流式:`serde_json::to_string(&input)` 转成 string。
   - 流式:`ToolInputDelta` 已是 string 片段,直接透传。
   - 参考:new-api `kitutil.Marshal(message.Input)`、aimux
     [model.rs:695](../aimux-providers/src/openai/model.rs#L695) 逆向
     `serde_json::from_str(args)`。

2. **tool_call 流式 index 稳定性**
   - OpenAI 要求每帧 tool_call 带稳定 `index`。
   - aimux 用 `tool_call_id` 为 key 分配递增 index(参考 opencodex
     `toolIndexByCallId`)。

3. **reasoning 字段决策**
   - 采用 `reasoning_content`(非标准但 DeepSeek/OpenRouter/阿里通义生态用),
     与 aimux 现有 [types.rs `reasoning_content`](../aimux-providers/src/openai/types.rs#L149)
     一致。`include_reasoning` 选项可关闭。

4. **空 content 处理**
   - 有 tool_calls 时 `content = None`(序列化为 `null`),参考 new-api
     `content/tool_calls 互斥`。
   - 无任何内容时 `content = Some("")`(空串),避免 null 歧义。

5. **finish_reason 时序**
   - 流式**恰好发一次** finish_reason(在终止帧)。
   - `ToolCalls` finish_reason 必须在所有 tool_call delta 发完后才发。

6. **末帧 usage 约定**
   - `include_usage=true` 时,终止帧带 `usage`。
   - `include_usage=false` 时,usage 丢弃(OpenAI 约定流式默认不带 usage)。

7. **StreamStart 与首帧**
   - 两种策略:(a) `StreamStart` 立即发首帧;(b) 惰性到首个内容 delta。
   - **选 (a)**:更简单,`StreamStart` 必发,适合大多数客户端。

8. **provider_metadata 透传**
   - logprobs 从 `provider_metadata["openai"]["logprobs"]` 取回。
   - 其他 provider 的 metadata 无法映射到 OpenAI 格式,丢弃。

## 7. 测试策略

### 7.1 单元测试(aimux-core)

- **非流式**:构造各 `GenerateContent` 组合的 `GenerateResult`,断言输出的
  `ChatCompletion` JSON 结构。
- **流式**:构造 `StreamPart` 序列(用 `futures::stream::iter`),驱动
  `to_chat_completion_stream`,收集所有 chunk 断言。
- **覆盖矩阵**:text-only / tool-call / reasoning / mixed / error / 空内容 /
  多 tool_call / Source / File 降级。

### 7.2 往返测试(round-trip)

利用 aimux 现有的 OpenAI cassette(录像):
1. 用 cassette 回放,`do_generate` 得到 `GenerateResult`;
2. `to_chat_completion` 转成 `ChatCompletion`;
3. 断言关键字段与 cassette 原始 OpenAI 响应一致(content / tool_calls /
   usage / finish_reason)。

这验证"OpenAI → aimux → OpenAI"往返的无损性。

### 7.3 对照测试

对照 opencodex / new-api 的转换输出(用相同输入),验证 aimux 转换结果
与参考实现在关键字段上一致。

## 8. 实现计划

### 第一期(核心转换层)

1. `aimux-core/src/openai_output.rs`:类型定义 + 非流式转换 + 单元测试
2. 流式转换状态机 + 单元测试
3. `generate.rs` 便利函数 `generate_text_as_openai` / `stream_text_as_openai`
4. ts-rs 类型导出(`#[ts(export)]`)
5. Node binding 暴露 + 端到端测试
6. 往返测试(用 OpenAI cassette)
7. 文档:`docs/api/openai-output.md`

### 第二期(优化与扩展)

- 其他 6 个 binding 暴露(Python/Go/Swift/Kotlin/Java/Flutter/C)
- 直通模式(见 §10)
- 入站 OpenAI 请求解析(OpenAI messages → ModelMessage)

## 9. 与现有架构的关系

```
                    ┌─ generate_text ─────→ GenerateTextResult (AI SDK 格式)
                    │                         │
用户 ──→ LanguageModel ─┤                      └─ (现有)
                    │
                    └─ generate_text_as_openai ─→ ChatCompletion (OpenAI 格式)
                         ↑ 新增:内部仍走                  ↑ 新增:转换层
                         do_generate,再转换               to_chat_completion
```

- **不改动** `LanguageModel` trait、`do_generate`/`do_stream`、现有
  `generate_text`/`stream_text`——零侵入。
- 转换层是**后处理**:provider 返回 `GenerateResult`/`StreamPart` 后,
  再转成 OpenAI 格式。
- 对所有 provider 统一生效,不区分 provider 类型。

## 10. 开放问题(后续)

### 10.1 直通模式(zero-copy passthrough)

**问题**:OpenAI provider 的原始响应本来就是 OpenAI 格式。当前路径是
"OpenAI 原始 → `GenerateResult` → `ChatCompletion`",有往返转换开销。

**方案(第二期)**:在 `LanguageModel` trait 加可选方法
`do_generate_openai_raw` / `do_stream_openai_raw`,OpenAI 协议 provider
直接返回原始 JSON/SSE,跳过转换。非 OpenAI provider 走转换层。

**第一期不做的原因**:aimux 的 OpenAI 解析(`execute_generate`)已保留
logprobs(provider_metadata)、usage raw 等信息,往返基本无损。直通的收益
主要是性能(避免一次序列化往返),属优化项。

### 10.2 reasoning 字段标准化

`reasoning_content` 是非标准扩展。OpenAI 官方在 Responses API 用
`reasoning` 对象,Chat Completions 无标准字段。当前选 `reasoning_content`
(与 DeepSeek/OpenRouter 生态一致),后续可加选项切换为 `reasoning` 别名
(参考 new-api 两者都输出)。

### 10.3 多模态降级策略

`File` 内容(base64/URL)在 Chat Completions 里无标准位置。当前降级为
content 里的 URL/data URI 字符串。后续可考虑映射到 `image_url` 结构
(若 media_type 是图片)。

## 11. 数据来源

本研究基于以下 reference 项目(已 clone 到 `reference/`,gitignore):

- **new-api**: `reference/new-api/` — Go 网关,最完整的协议互转
- **opencodex**: `reference/opencodex/` — TypeScript 转发代理,Responses↔Chat 转换
- **ccswitch-deepseek**: `reference/ccswitch-deepseek/` — Node.js 代理,Responses↔Chat
- **cc-switch**: `reference/cc-switch/` — Rust 配置切换器,模型名映射

aimux 现有代码:
- [aimux-providers/src/openai/model.rs](../aimux-providers/src/openai/model.rs) — OpenAI→aimux 解析(反向基线)
- [aimux-providers/src/openai/types.rs](../aimux-providers/src/openai/types.rs) — OpenAI 类型定义
- [aimux-providers/src/openai/convert.rs](../aimux-providers/src/openai/convert.rs) — 请求构造 + finish_reason 解析
- [aimux-core/src/stream_part.rs](../aimux-core/src/stream_part.rs) — StreamPart 定义
- [aimux-core/src/result.rs](../aimux-core/src/result.rs) — GenerateResult / GenerateContent 定义
- [aimux-core/src/types.rs](../aimux-core/src/types.rs) — Usage / FinishReason 定义
