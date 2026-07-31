# 数据类型定义：aimux vs AI SDK V4 对比

> **日期**：2026-07-30（v0.2 修订）
> **范围**：aimux Rust 核心类型（ts-rs 导出 79 个 .ts）vs AI SDK V4 provider 类型（`@ai-sdk/provider` V4）
> **方法**：逐类型对比字段、命名、结构

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，逐类型对比 aimux ts-rs 导出 vs AI SDK V4 provider 类型 |
| 2026-07-30 | v0.2 | 重新核查 Rust 源码。commit `2585b3c7`（"5 语言类型化 wrapper + Rust 核心类型补字段"）已补上 ToolCall.provider_executed/dynamic、TokenUsage.no_cache/cache_read/cache_write/text/reasoning。原文档标记的"缺字段"大多已过时，逐条修正。 |
| 2026-07-30 | v0.3 | commit `af50905f` 补齐剩余差距：GenerateContent/StreamPart 新增 File 变体；ContentPart::ToolResult 补 tool_name/is_error/preliminary/dynamic；全链路 ToolResult 字段名统一 output→result；StreamPart::ToolResult 补 provider_metadata；FileData/FileBytes 补 PartialEq。修复 tool_executor.rs lib 编译错误 + 403→Auth 映射。v0.2 标记的 🔴 差距全部消除。 |

---

## 1. 类型清单

### aimux（ts-rs 自动生成，`aimux-core/bindings/*.ts`）

共 79 个类型文件，按功能分组：

| 分组 | 类型 | 来源 |
|------|------|------|
| **文本生成** | `GenerateTextOptions`, `GenerateTextResult`, `GenerateResult`, `GenerateContent`, `StreamPart`, `FinishReason`, `FinishReasonUnified`, `Usage`, `TokenUsage`, `Warning` | `generate.rs`, `result.rs`, `stream_part.rs`, `types.rs` |
| **消息** | `ModelMessage`, `ModelPrompt`, `MessageContent`, `Role`, `ContentPart`, `LanguageModelPromptMessage` | `message.rs`, `content.rs`, `language_model_message.rs` |
| **工具** | `Tool`, `FunctionTool`, `ProviderTool`, `ToolCall`, `ToolChoice`, `ToolResult` | `tool.rs` |
| **嵌入** | `EmbeddingCallOptions`, `EmbeddingResult`, `EmbeddingResponse`, `EmbeddingUsage` | `embedding_model.rs` |
| **语音** | `SpeechCallOptions`, `SpeechResult`, `SpeechRequest`, `SpeechResponse` | `speech_model.rs` |
| **转录** | `TranscriptionCallOptions`, `TranscriptionResult`, `TranscriptionResponse`, `TranscriptionRequest`, `TranscriptionSegment`, `TranscriptionStreamPart`, `AudioInput`, `AudioData`, `AudioChunk`, `InputAudioFormat` | `transcription_model.rs` |
| **图像** | `ImageCallOptions`, `ImageResult`, `ImageResponse`, `ImageFile`, `ImageFileData`, `ImageOutputs`, `ImageUsage`, `Size`, `AspectRatio` | `image_model.rs` |
| **视频** | `VideoCallOptions`, `VideoResult`, `VideoResponse`, `VideoData`, `VideoFile`, `VideoFileData`, `VideoFrameImage`, `VideoFrameType` | `video_model.rs` |
| **重排序** | `RerankingCallOptions`, `RerankingResult`, `RerankingResponse`, `RerankingRank`, `RerankingDocuments` | `reranking_model.rs` |
| **搜索** | `SearchCallOptions`, `SearchResult`, `SearchResponse`, `SearchResultItem` | `search_model.rs` |
| **文件** | `UploadFileCallOptions`, `UploadFileResult`, `UploadFileData`, `FileBytes`, `FileData` | `files_model.rs`, `shared.rs` |
| **通用** | `CallOptions`, `ResponseFormat`, `ResponseInfo`, `ResponseMetadata`, `RequestInfo`, `ReasoningEffort`, `ModelId`, `AiMuxError` | `options.rs`, `types.rs`, `shared.rs` |

### AI SDK V4（`@ai-sdk/provider`，手工定义）

| 分组 | 类型 | 来源 |
|------|------|------|
| **语言模型** | `LanguageModelV4`, `LanguageModelV4CallOptions`, `LanguageModelV4GenerateResult`, `LanguageModelV4StreamResult`, `LanguageModelV4StreamPart` | `language-model/v4/*.ts` |
| **内容** | `LanguageModelV4Content` = Text \| Reasoning \| File \| ToolCall \| ToolResult \| Source \| ToolApprovalRequest \| CustomContent \| ReasoningFile | `language-model-v4-content.ts` |
| **消息** | `LanguageModelV4Prompt`, `LanguageModelV4Message` | `language-model-v4-prompt.ts` |
| **工具** | `LanguageModelV4FunctionTool`, `LanguageModelV4ProviderTool`, `LanguageModelV4ToolCall`, `LanguageModelV4ToolResult`, `LanguageModelV4ToolChoice` | `language-model/v4/*.ts` |
| **通用** | `LanguageModelV4Usage`, `LanguageModelV4FinishReason`, `LanguageModelV4ResponseMetadata`, `SharedV4Warning`, `SharedV4ProviderOptions`, `SharedV4ProviderMetadata` | `shared/v4/*.ts` |

---

## 2. 逐类型对比

### 2.1 GenerateResult（核心结果）

| 字段 | aimux `GenerateResult` | AI SDK `LanguageModelV4GenerateResult` | 差异 |
|------|----------------------|---------------------------------------|------|
| content | `Array<GenerateContent>` | `Array<LanguageModelV4Content>` | 结构不同（见 2.2） |
| finish_reason | `FinishReason` | `finishReason: LanguageModelV4FinishReason` | 🟡 snake_case vs camelCase |
| usage | `Usage` | `usage: LanguageModelV4Usage` | 结构不同（见 2.8） |
| warnings | `Array<Warning>` | 无（warnings 在 stream-start） | 🟡 aimux 多了非流式 warnings |
| provider_metadata | `JsonValue \| null` | `providerMetadata?: SharedV4ProviderMetadata` | 🟡 命名 + null vs optional |
| response | `ResponseMetadata` | `response?: {...headers, body}` | 🟡 aimux 有独立 ResponseMetadata 类型 |
| request_body | `JsonValue \| null` | `request?: { body?: unknown }` | 🟡 命名 |
| response_headers | `{ [key]: string } \| null` | 合并在 `response.headers` 里 | 🟡 结构 |

**差异级别**：🟡 命名 + 结构细节。核心字段一致。

### 2.2 GenerateContent（内容变体）

**aimux**（外部标签 enum）：
```typescript
type GenerateContent =
  | { "Text": { text: string } }
  | { "ToolCall": { tool_call_id, tool_name, input: JsonValue, provider_executed?, dynamic?, provider_metadata? } }
  | { "Source": { id, source_type, url?, title? } }
  | { "Reasoning": { text, provider_metadata? } }
  | { "File": { data: FileData, media_type, filename?, provider_metadata? } }
  | { "ToolResult": { tool_call_id, tool_name, result: JsonValue, is_error?, preliminary?, dynamic?, provider_metadata? } }
```

**AI SDK V4**（内部标签 `type` 字段）：
```typescript
type LanguageModelV4Content =
  | { type: 'text', text, providerMetadata? }
  | { type: 'reasoning', text, providerMetadata? }
  | { type: 'tool-call', toolCallId, toolName, input: string, providerExecuted?, dynamic?, providerMetadata? }
  | { type: 'tool-result', toolCallId, toolName, result, isError?, preliminary?, dynamic?, providerMetadata? }
  | { type: 'source', sourceType, id, url?, title?, providerMetadata? }
  | { type: 'file', data, mediaType, filename?, providerOptions? }
  | { type: 'tool-approval-request', ... }
  | { type: 'custom', providerOptions? }
  | { type: 'reasoning-file', ... }
```

| 差异点 | aimux | AI SDK V4 |
|--------|-------|-----------|
| 标签方式 | **外部标签**（`{"Text": {...}}`） | **内部标签**（`{type: "text", ...}`） |
| 字段命名 | snake_case（`tool_call_id`） | camelCase（`toolCallId`） |
| ToolCall.input | `JsonValue`（已解析对象） | `string`（stringified JSON） |
| ToolResult | `result: JsonValue` | `result: JSONValue` + `isError?` + `preliminary?` |
| File 变体 | ✅ 有（`{ data: FileData, media_type, filename?, provider_metadata? }`） | ✅ 有（`type: 'file'`） | 🟢 已对齐（commit `af50905f`） |
| ToolApprovalRequest | ❌ 无 | ✅ 有 | 🟡 V4 新增，低优先级 |
| CustomContent | ❌ 无 | ✅ 有 | 🟡 V4 新增，低优先级 |
| ReasoningFile | ❌ 无 | ✅ 有 | 🟡 V4 新增，低优先级 |

**差异级别**：🟡 标签方式 + 命名（设计选择，wrapper 层解决）。aimux 已补 File 变体，仅剩 Custom/ToolApprovalRequest/ReasoningFile 三个边缘变体未实现。

### 2.3 StreamPart（流式 chunk）

**aimux**（外部标签，17 个变体）：
```typescript
type StreamPart =
  | { "TextStart": { id } }
  | { "TextDelta": { id, delta } }
  | { "TextEnd": { id } }
  | { "StreamStart": { warnings } }
  | { "Finish": { finish_reason, usage, provider_metadata? } }
  | { "Error": { error } }
  | { "ToolInputStart": { id, tool_name } }
  | { "ToolInputDelta": { id, delta } }
  | { "ToolInputEnd": { id } }
  | { "ToolCall": { tool_call_id, tool_name, input } }
  | { "ToolResult": { tool_call_id, tool_name, result, is_error?, preliminary?, dynamic?, provider_metadata? } }
  | { "ReasoningStart": { id, provider_metadata? } }
  | { "ReasoningDelta": { id, delta, provider_metadata? } }
  | { "ReasoningEnd": { id, provider_metadata? } }
  | { "File": { data: FileData, media_type, filename?, provider_metadata? } }
  | { "ResponseMetadata": { id?, timestamp?, model_id? } }
  | { "Source": { id, source_type, url?, title? } }
  | { "Raw": { raw_value } }
```

**AI SDK V4**（内部标签，~20 个变体）：
```typescript
type LanguageModelV4StreamPart =
  | { type: 'text-start', id, providerMetadata? }
  | { type: 'text-delta', id, delta, providerMetadata? }
  | { type: 'text-end', id, providerMetadata? }
  | { type: 'reasoning-start', id, providerMetadata? }
  | { type: 'reasoning-delta', id, delta, providerMetadata? }
  | { type: 'reasoning-end', id, providerMetadata? }
  | { type: 'tool-input-start', id, toolName, providerMetadata?, providerExecuted?, dynamic?, title? }
  | { type: 'tool-input-delta', id, delta, providerMetadata? }
  | { type: 'tool-input-end', id, providerMetadata? }
  | { type: 'tool-call', ... }  // LanguageModelV4ToolCall
  | { type: 'tool-result', ... }  // LanguageModelV4ToolResult
  | { type: 'tool-approval-request', ... }
  | { type: 'file', ... }
  | { type: 'source', ... }
  | { type: 'custom', ... }
  | { type: 'reasoning-file', ... }
  | { type: 'stream-start', warnings }
  | { type: 'finish', finishReason, usage, providerMetadata? }
  | { type: 'response-metadata', id?, timestamp?, modelId? }
  | { type: 'error', error }
  | { type: 'raw', rawValue }  // 非标准，部分 provider 有
```

| 差异点 | aimux | AI SDK V4 |
|--------|-------|-----------|
| 标签方式 | 外部标签（`{"TextDelta": {...}}`） | 内部标签（`{type: "text-delta", ...}`） |
| 字段命名 | snake_case | camelCase |
| 变体数 | 18 | ~21 |
| ToolApprovalRequest | ❌ 无 | ✅ 有 |
| CustomContent | ❌ 无 | ✅ 有 |
| File | ✅ 有（commit `af50905f`） | ✅ 有 |
| ReasoningFile | ❌ 无 | ✅ 有 |
| ToolCall.input | `JsonValue` | `string` |
| ToolResult 字段 | `result` + `is_error?` + `preliminary?` + `dynamic?` + `provider_metadata?` | 同 | 🟢 已对齐（commit `af50905f` 统一 output→result + 补 provider_metadata） |

**差异级别**：🟡 标签 + 命名，aimux 已补 File 变体 + ToolResult 字段统一。仅剩 Custom/ToolApprovalRequest/ReasoningFile 三个边缘变体未实现。

### 2.4 ToolCall

| 字段 | aimux | AI SDK V4 | 差异 |
|------|-------|-----------|------|
| id | `tool_call_id` | `toolCallId` | 🟡 命名 |
| name | `tool_name` | `toolName` | 🟡 命名 |
| input | `JsonValue`（已解析对象） | `string`（stringified JSON） | 🟡 类型不同（aimux 更友好） |
| providerExecuted | ✅ `provider_executed: Option<bool>` | ✅ 有 | 🟡 命名 |
| dynamic | ✅ `dynamic: Option<bool>` | ✅ 有 | 🟡 命名 |
| providerMetadata | ❌ 无（在 ToolCall 变体上无） | ✅ 有 | 🟡 缺字段 |

> **v0.2 修正**：`provider_executed` 和 `dynamic` 已在 commit `2585b3c7` 中补上。原文档标记为 ❌，实际代码已有（[tool.rs](../aimux-core/src/tool.rs)）。仅剩 `providerMetadata` 缺失。

### 2.5 ToolResult

| 字段 | aimux（ContentPart::ToolResult） | AI SDK V4 | 差异 |
|------|------|-----------|------|
| id | `tool_call_id` | `toolCallId` | 🟡 命名 |
| name | `tool_name?`（可选） | `toolName` | 🟡 命名（commit `af50905f` 已补） |
| output | `result: JsonValue` | `result: NonNullable<JSONValue>` | 🟢 已统一（commit `af50905f` output→result） |
| isError | `is_error?` | ✅ 有 | 🟢 已对齐（commit `af50905f` 已补） |
| preliminary | `preliminary?` | ✅ 有 | 🟢 已对齐（commit `af50905f` 已补） |
| dynamic | `dynamic?` | ✅ 有 | 🟢 已对齐（commit `af50905f` 已补） |

> **v0.3 确认**：`ContentPart::ToolResult`、`GenerateContent::ToolResult` 和 `StreamPart::ToolResult` 现已全部对齐——统一字段名 `result`（不再用 `output`），全部拥有 `tool_name`/`is_error`/`preliminary`/`dynamic`（commit `af50905f`）。

### 2.6 FunctionTool

| 字段 | aimux | AI SDK V4 | 差异 |
|------|-------|-----------|------|
| type | `"function"`（外部标签 `Tool` 上） | `'function'` | 🟢 一致 |
| name | `name` | `name` | 🟢 一致 |
| description | `description?` | `description?` | 🟢 一致 |
| inputSchema | `input_schema: JsonValue` | `inputSchema: JSONSchema7` | 🟡 命名 + 类型（JsonValue vs JSONSchema7） |
| strict | `strict?` | `strict?` | 🟢 一致 |
| providerOptions | `provider_options?` | `providerOptions?` | 🟡 命名 |
| inputExamples | `input_examples?: Array<JsonValue>` | `inputExamples?: Array<{input: JSONObject}>` | 🟡 结构不同 |

**差异级别**：🟡 命名 + inputExamples 结构。

### 2.7 ToolChoice

| aimux | AI SDK V4 | 差异 |
|-------|-----------|------|
| `"auto"` | `{ type: 'auto' }` | 🔴 格式不同 |
| `"none"` | `{ type: 'none' }` | 🔴 格式不同 |
| `"required"` | `{ type: 'required' }` | 🔴 格式不同 |
| `{ type: "tool", toolName }` | `{ type: 'tool', toolName }` | 🟢 一致 |

**差异级别**：🔴 auto/none/required——aimux 用裸字符串，AI SDK V4 用 `{type: "auto"}` 对象。Tool 变体一致。

### 2.8 Usage

**aimux**：
```typescript
type Usage = { input_tokens: TokenUsage, output_tokens: TokenUsage, raw?: Value }
type TokenUsage = {
  total: number | null,
  no_cache: number | null,
  cache_read: number | null,
  cache_write: number | null,
  text: number | null,
  reasoning: number | null,
}
```

**AI SDK V4**：
```typescript
type LanguageModelV4Usage = {
  inputTokens: { total, noCache?, cacheRead?, cacheWrite? },
  outputTokens: { total, text?, reasoning? },
  raw?: JSONObject,
}
```

| 差异点 | aimux | AI SDK V4 |
|--------|-------|-----------|
| 命名 | `input_tokens` / `output_tokens` | `inputTokens` / `outputTokens` |
| 子字段 | `total` + `no_cache` + `cache_read` + `cache_write` + `text` + `reasoning` | `total` + `noCache?` + `cacheRead?` + `cacheWrite?` + `text?` + `reasoning?` |
| raw | ✅ `raw?: Value` | ✅ `raw?: JSONObject` |

> **v0.2 修正**：`TokenUsage` 的 `no_cache`/`cache_read`/`cache_write`/`text`/`reasoning` 和 `Usage.raw` 已在 commit `2585b3c7` 中全部补上。原文档标记为"缺 cache/text/reasoning 细分 + raw"，实际代码已完整。**仅剩命名差异（snake_case vs camelCase）。**

### 2.9 FinishReason

| | aimux | AI SDK V4 |
|---|-------|-----------|
| 结构 | `{ unified: FinishReasonUnified, raw: string \| null }` | `{ unified: 'stop'\|'length'\|..., raw: string \| undefined }` |
| 命名 | `unified` / `raw` | `unified` / `raw` | 🟢 一致 |
| null vs undefined | `raw: string \| null` | `raw: string \| undefined` | 🟡 TS 风格差异 |

**差异级别**：🟢 基本一致（仅 null vs undefined）。

### 2.10 Role

| aimux | AI SDK V4 | 差异 |
|-------|-----------|------|
| `"system" \| "user" \| "assistant" \| "tool"` | `'system' \| 'user' \| 'assistant' \| 'tool'` | 🟢 完全一致 |

### 2.11 ModelMessage / Prompt

**aimux**：
```typescript
type ModelMessage = { role: Role, content: MessageContent }
type MessageContent = string | Array<ContentPart>
type ModelPrompt = string | Array<ModelMessage>
```

**AI SDK V4**（provider-facing，非用户面）：
```typescript
type LanguageModelV4Prompt = Array<LanguageModelV4Message>
type LanguageModelV4Message = { role: 'system', content: string } | { role: 'user', content: Array<TextPart | FilePart> } | ...
```

| 差异点 | aimux | AI SDK V4 |
|--------|-------|-----------|
| 层次 | 用户面（`ModelPrompt`） | provider 面（`LanguageModelV4Prompt`） |
| system content | `string`（via `MessageContent`） | `string` | 🟢 一致 |
| user content | `string \| Array<ContentPart>` | `Array<TextPart \| FilePart>` | 🟡 aimux 多了 string 便捷形式 |
| ContentPart 变体 | 9 个（text/image/file/file_base64/file_url/file_reference/reasoning/tool_call/tool_result） | 5 个（text/file/custom/reasoning 等） | 🟡 aimux 更细 |

**差异级别**：🟡 aimux 是用户面（有便捷 string），AI SDK V4 是 provider 面（严格 array）。设计层次不同。

### 2.12 ContentPart

**aimux**（9 个变体，内部标签 `type`）：
```typescript
type ContentPart =
  | { type: 'text', text, provider_options? }
  | { type: 'image', image: number[], media_type, provider_options? }
  | { type: 'file', data: number[], media_type, filename?, provider_options? }
  | { type: 'file_base64', data: string, media_type, ... }
  | { type: 'file_url', url, media_type, ... }
  | { type: 'file_reference', reference, media_type, ... }
  | { type: 'reasoning', text, signature?, provider_options? }
  | { type: 'tool_call', tool_call_id, tool_name, input, provider_options? }
  | { type: 'tool_result', tool_call_id, output, provider_options? }
```

**AI SDK V4**（用户面 `ContentPart`，内部标签）：
```typescript
type ContentPart =
  | { type: 'text', text, providerOptions? }
  | { type: 'file', data: FileData, mediaType, filename?, providerOptions? }
  | { type: 'reasoning', text, providerOptions? }
  | { type: 'tool-call', ... }
  | { type: 'tool-result', ... }
  | { type: 'source', ... }
  | { type: 'custom', providerOptions? }
```

| 差异点 | aimux | AI SDK V4 |
|--------|-------|-----------|
| 标签方式 | 内部标签 `type` | 内部标签 `type` | 🟢 一致 |
| 命名 | snake_case | camelCase | 🟡 |
| File 变体 | 4 个（file/file_base64/file_url/file_reference） | 1 个（`file`，data 是 tagged union） | 🟡 aimux 拆成 4 个，AI SDK 合 1 个 |
| Image | 独立变体 `type: 'image'` | 合在 `type: 'file'` 里（mediaType 区分） | 🟡 |
| Source | ❌ 无（在 GenerateContent/StreamPart 有） | ✅ 有 | 🟡 |
| Custom | ❌ 无 | ✅ 有 | 🟡 |

**差异级别**：🟡 命名 + File 拆分方式不同。

---

## 3. 命名差异汇总

| aimux (snake_case) | AI SDK V4 (camelCase) | 出现位置 |
|---------------------|----------------------|---------|
| `tool_call_id` | `toolCallId` | ToolCall, ContentPart, StreamPart |
| `tool_name` | `toolName` | ToolCall, StreamPart |
| `input_schema` | `inputSchema` | FunctionTool |
| `provider_options` | `providerOptions` | 多处 |
| `provider_metadata` | `providerMetadata` | 多处 |
| `input_tokens` | `inputTokens` | Usage |
| `output_tokens` | `outputTokens` | Usage |
| `finish_reason` | `finishReason` | GenerateResult, FinishReason, StreamPart |
| `max_output_tokens` | `maxOutputTokens` | GenerateTextOptions |
| `stop_sequences` | `stopSequences` | GenerateTextOptions |
| `top_p` / `top_k` | `topP` / `topK` | GenerateTextOptions |
| `model_id` | `modelId` | ResponseMetadata |
| `source_type` | `sourceType` | Source |
| `raw_value` | `rawValue` | StreamPart::Raw |
| `input_examples` | `inputExamples` | FunctionTool |
| `response_headers` | `response.headers` | GenerateResult |
| `request_body` | `request.body` | GenerateResult |

**规律**：全部是 snake_case → camelCase 转换。wrapper 层统一做映射即可。

---

## 4. 结构差异汇总（v0.3 重新核查）

| 差异点 | aimux | AI SDK V4 | 影响范围 | 状态 | 修复方案 |
|--------|-------|-----------|---------|:----:|---------|
| **标签方式** | 外部标签（`{"TextDelta": {...}}`） | 内部标签（`{type: "text-delta", ...}`） | GenerateContent, StreamPart | 🟡 设计差异 | wrapper 做转换；或 Rust 加 `#[serde(tag = "type")]`（breaking change） |
| **字段命名** | snake_case | camelCase | 全部 | 🟡 设计差异 | wrapper 做 snake↔camel 映射（各语言已有或可加） |
| **ToolCall.input** | `JsonValue`（已解析） | `string`（stringified） | ToolCall, ContentPart, StreamPart | 🟢 aimux 更友好 | 不改 |
| **ToolCall.provider_executed/dynamic** | ✅ 已有 | ✅ 有 | ToolCall | 🟢 已对齐 | commit `2585b3c7` 已补 |
| **ToolResult 字段名** | `result`（统一） | `result` | ContentPart, StreamPart | 🟢 已对齐 | commit `af50905f` 统一 output→result |
| **ToolResult 缺字段** | ✅ 已全部补齐（`is_error`/`preliminary`/`dynamic`/`tool_name`） | — | ContentPart, GenerateContent, StreamPart | 🟢 已对齐 | commit `af50905f` 已补 |
| **ToolChoice 格式** | `"Auto"` 裸字符串 | `{type: "auto"}` 对象 | ToolChoice | 🟡 格式 | wrapper 转换；或 Rust 加 `#[serde(rename_all = "lowercase")]` |
| **File 拆分** | 4 变体（file/file_base64/file_url/file_reference） | 1 变体（data 是 tagged union） | ContentPart | 🟡 设计差异 | wrapper 合并；或 Rust 合并变体 |
| **Usage 细分** | ✅ 已完整（no_cache/cache_read/cache_write/text/reasoning/raw） | 有 | Usage | 🟢 已对齐 | commit `2585b3c7` 已补 |
| **缺变体：File** | ✅ 已补（GenerateContent/StreamPart 均有 File 变体） | 有 | GenerateContent, StreamPart | 🟢 已对齐 | commit `af50905f` 已补 |
| **缺变体：Custom** | 无 | 有 | GenerateContent, StreamPart | 🟡 可选 | 低优先级，V4 新增 |
| **缺变体：ToolApprovalRequest** | 无 | 有 | GenerateContent, StreamPart | 🟡 可选 | 低优先级，V4 新增 |
| **缺变体：ReasoningFile** | 无 | 有 | GenerateContent, StreamPart | 🟡 可选 | 低优先级，V4 新增 |

---

## 5. 各语言类型定义方案

### 5.1 Node（已有，直接用）

ts-rs 已生成 79 个 .ts 类型。wrapper 直接 import 即可。命名映射在 wrapper 层做。

### 5.2 Python（Pydantic model）

手写 Pydantic model，镜像 Rust 类型。字段名用 snake_case（Python 惯例，与 Rust 一致）。

```python
class GenerateTextResult(BaseModel):
    text: str
    tool_calls: list[ToolCall]
    finish_reason: FinishReason
    usage: Usage
    raw: GenerateResult
```

### 5.3 Swift（Codable struct）

手写 Codable struct，字段名用 camelCase（Swift 惯例），CodingKeys 映射 snake_case。

```swift
struct GenerateTextResult: Codable {
    let text: String
    let toolCalls: [ToolCall]
    let finishReason: FinishReason
    let usage: Usage
    let raw: GenerateResult

    enum CodingKeys: String, CodingKey {
        case text, toolCalls = "tool_calls", finishReason = "finish_reason", usage, raw
    }
}
```

### 5.4 Kotlin（data class + @SerialName）

手写 data class，字段名用 camelCase，`@SerialName` 映射 snake_case。

```kotlin
@Serializable
data class GenerateTextResult(
    val text: String,
    @SerialName("tool_calls") val toolCalls: List<ToolCall>,
    @SerialName("finish_reason") val finishReason: FinishReason,
    val usage: Usage,
    val raw: GenerateResult,
)
```

### 5.5 Dart（class + @JsonKey）

手写 class + json_serializable，字段名用 camelCase，`@JsonKey` 映射 snake_case。

```dart
@JsonSerializable()
class GenerateTextResult {
  final String text;
  @JsonKey(name: 'tool_calls') final List<ToolCall> toolCalls;
  @JsonKey(name: 'finish_reason') final FinishReason finishReason;
  final Usage usage;
  final GenerateResult raw;
}
```

---

## 6. 总结（v0.3）

| 维度 | 差异程度 | 说明 |
|------|:---:|------|
| **字段命名** | 🟡 | 全部 snake_case vs camelCase，wrapper 统一映射 |
| **标签方式** | 🟡 | 外部标签 vs 内部标签（GenerateContent/StreamPart），wrapper 转换 |
| **ToolCall.input** | 🟢 | aimux 用 `JsonValue` 更友好，不改 |
| **ToolCall 字段** | 🟢 | `provider_executed`/`dynamic` 已补，完全对齐（仅剩命名） |
| **Usage** | 🟢 | `no_cache`/`cache_read`/`cache_write`/`text`/`reasoning`/`raw` 已补，完全对齐（仅剩命名） |
| **ToolResult** | 🟢 | 字段全对齐：`result`（统一）/`is_error`/`preliminary`/`dynamic`/`tool_name`/`provider_metadata`（commit `af50905f`） |
| **ToolChoice** | 🟡 | 裸字符串 vs 对象格式，wrapper 转换 |
| **File 变体** | 🟢 | GenerateContent/StreamPart 已补 File 变体（commit `af50905f`） |
| **缺变体** | 🟡 | 仅剩 Custom/ToolApprovalRequest/ReasoningFile（V4 新增边缘变体，低优先级） |
| **核心结构** | 🟢 | GenerateResult / Usage / FinishReason / Role / ToolResult / File 核心字段一致 |

**v0.3 结论**：commit `af50905f` 补齐了 v0.2 标记的全部 🔴 差距——GenerateContent/StreamPart 新增 File 变体、ContentPart::ToolResult 补齐缺失字段、全链路 ToolResult 字段名统一 `output`→`result`。同时修复了 `tool_executor.rs` 的 lib 编译错误（ToolResult 缺 `is_error`/`preliminary`）和 403→Auth 映射。

**当前剩余差距全部是设计选择或边缘变体**：
1. **命名/标签差异** — Rust serde 默认外部标签 + snake_case，wrapper 层解决，不改 Rust 核心。
2. **Custom/ToolApprovalRequest/ReasoningFile** — V4 新增的边缘变体，低优先级，按需实现。
3. **ToolChoice 裸字符串格式** — wrapper 可转换，Rust enum 序列化更自然，不改。
