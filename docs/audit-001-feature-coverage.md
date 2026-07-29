# 功能审计报告

> **日期**：2026-07-29
> **审计员**：Liam（独立 agent）
> **范围**：Rust 核心 + Node/Python/C ABI 绑定 + 全部测试 + cassette 数据

---

## 1. 功能清单

### Rust 核心完整支持的能力

| 能力 | 实现位置 | 状态 |
|------|---------|:----:|
| 文本生成 `generate_text` | `generate.rs:159` | ✅ 返回 `GenerateTextResult` |
| 结构化结果 `GenerateResult.content` | `result.rs:57` | ✅ `Vec<GenerateContent>` 5 变体 |
| 流式生成 `stream_text` | `generate.rs:234` | ✅ 返回 `StreamPart` 流 |
| StreamPart 16 个变体 | `stream_part.rs:16` | ✅ 完整覆盖 |
| 多角色消息 Role | `message.rs:16` | ✅ System/User/Assistant/Tool |
| 多部分消息 ContentPart | `content.rs:13` | ✅ 9 个变体 |
| 工具定义 FunctionTool | `tool.rs:10` | ✅ name + JSON Schema |
| 工具选择 ToolChoice | `tool.rs:129` | ✅ Auto/None/Required/Tool |
| 向量嵌入 | `embedding_model.rs:112` | ✅ do_embed |
| 语音合成 (TTS) | `speech_model.rs:132` | ✅ do_generate |
| 语音转文字 (STT) | `transcription_model.rs:271` | ✅ do_generate + do_stream |
| 图像生成 | `image_model.rs:174` | ✅ do_generate |
| 视频生成 | `video_model.rs:196` | ✅ do_generate |
| 重排序 | `reranking_model.rs:119` | ✅ do_rerank |
| 搜索 | `search_model.rs:131` | ✅ do_search |
| 文件上传 | `files_model.rs:93` | ✅ upload_file |

### GenerateContent 变体（结构化结果）

| 变体 | 字段 | 说明 |
|------|------|------|
| `Text` | text: String | 生成的文本 |
| `ToolCall` | tool_call_id, tool_name, input | 模型请求的工具调用 |
| `Source` | id, source_type, url?, title? | 引用/来源 |
| `Reasoning` | text, provider_metadata? | 推理/思考段 |
| `ToolResult` | tool_call_id, tool_name, result | provider 执行的工具结果 |

### StreamPart 变体（流式）

| 变体 | 说明 |
|------|------|
| StreamStart | 流开始（携带 warnings） |
| TextStart / TextDelta / TextEnd | 文本段生命周期 |
| ToolInputStart / ToolInputDelta / ToolInputEnd | 工具调用输入流 |
| ToolCall | 完整工具调用 |
| ToolResult | provider 执行的工具结果 |
| ReasoningStart / ReasoningDelta / ReasoningEnd | 推理段生命周期 |
| ResponseMetadata | 响应元数据 |
| Source | 引用/来源 |
| Finish | 流结束（usage + finish_reason） |
| Error | 流错误 |

### 关键问题：generate_text 压扁了结构化 content

`generate_text` 把 `GenerateResult.content`（`Vec<GenerateContent>`）压扁了：
- `Text` 变体 → 拼接成 `result.text` 字符串
- `ToolCall` 变体 → 提取到 `result.tool_calls` 数组
- **`Source` / `Reasoning` / `ToolResult` 变体 → 被丢弃**

完整结构只在 `result.raw: GenerateResult` 中保留。用户要拿结构化 content 必须访问 `result.raw.content`，不是一等公民。

---

## 2. 绑定层覆盖

| 功能 | Rust 核心 | Node 绑定 | Python 绑定 | C ABI |
|------|:---------:|:---------:|:-----------:|:-----:|
| generate_text（压扁版） | ✅ | ✅ `model.generateText()` | ✅ `model.generate_text()` | ✅ `aimux_generate_text` |
| GenerateResult（结构化 raw） | ✅ raw 字段 | ✅ JSON 含 raw | ✅ 同左 | ✅ 同左 |
| stream_text | ✅ | ✅ `model.streamText()` | ✅ `model.stream_text()` | ✅ `aimux_stream_text` |
| StreamPart 全变体透传 | ✅ | ✅ JSON 序列化全变体 | ✅ 同左 | ✅ 同左 |
| 多角色消息输入 | ✅ | ✅ JSON `[{role, content}]` | ✅ 同左 | ✅ 同左 |
| 工具定义传入 | ✅ | ✅ tools 在 options JSON | ✅ 同左 | ✅ 同左 |
| ToolChoice | ✅ | ✅ 在 options JSON | ✅ 同左 | ✅ 同左 |
| Embedding | ✅ | ✅ `openaiEmbedding` | ✅ `openai_embedding` | ✅ `aimux_embed` |
| Speech (TTS) | ✅ | ✅ `openaiSpeech` | ✅ `openai_speech` | ✅ `aimux_speech_generate` |
| Image | ✅ | ✅ `openaiImage` | ✅ `openai_image` | ✅ `aimux_image_generate` |
| Transcription | ✅ | ✅ `openaiTranscription` | ✅ `openai_transcription` | ✅ `aimux_transcription_generate` |
| Reranking | ✅ | ✅ `cohereReranking` | ✅ `cohere_reranking` | ❌ 缺失 |
| Video | ✅ | ✅ `googleVideo` | ✅ `google_video` | ❌ 缺失 |
| Search | ✅ | ✅ class 暴露 | ✅ class 暴露 | ❌ 缺失 |
| Files | ✅ | ✅ `openaiFiles` | ✅ `openai_files` | ✅ `aimux_file_upload` |

### 缺失

- **C ABI 缺少 Reranking / Video / Search 的 `_new` + 操作函数**（3 组共 6 个函数）
- **Node/Python 绑定没有独立的 `generate()` 方法**直接返回 `GenerateResult`——用户只能通过 `result.raw` 访问结构化 content，不是一等公民

---

## 3. 测试覆盖

### Rust 侧（provider 测试 + e2e 测试）

| 功能场景 | 有测试？ | 测试文件 | 断言内容 |
|---------|:-------:|---------|---------|
| 纯文本生成 | ✅ | `e2e_test.rs` | text + usage + finish_reason |
| 工具调用解析（非流式） | ✅ | `openai_model_test.rs:566`, `e2e_test.rs:82` | `GenerateContent::ToolCall` + `result.tool_calls[0].tool_name` |
| 工具调用解析（流式） | ✅ | `openai_model_test.rs:780` | `StreamPart::ToolCall` + `StreamPart::ToolInputDelta` |
| ToolChoice::None | ✅ | `openai_convert_test.rs`, `cohere_model_test.rs` | 请求体不含 tools |
| ToolChoice::Required | ✅ | `openai_convert_test.rs`, `cohere_model_test.rs` | 请求体含 tool_choice: required |
| ToolChoice::Tool | ✅ | `openai_convert_test.rs` | 请求体含 tool_choice: {type:"tool", toolName} |
| 多角色消息 (System) | ✅ | `anthropic_model_test.rs:573` | Role::System |
| 结构化 content (Source) | ✅ | 多个 provider 测试 | `GenerateContent::Source` |
| 结构化 content (Reasoning) | ✅ | 多个 provider 测试 | `GenerateContent::Reasoning` |
| 结构化 content (ToolResult) | ✅ | `openai_model_test.rs:568` | `GenerateContent::ToolResult` |
| 流式 StreamPart 序列 | ✅ | `e2e_test.rs:300` | StreamStart → TextDelta → Finish 顺序 |
| Embedding (base64 解码) | ✅ | `cassette_multimodal_test.rs` | embeddings 非空 |
| Speech/Image/Transcription/Files | ✅ | `cassette_multimodal_test.rs` | 硬断言 |
| **工具调用完整往返** | ❌ | — | 没有测试：model 返回 ToolCall → 用户执行 → 把 ToolResult 放回 messages → 再次调用 → model 返回最终文本 |
| **多轮对话** | ❌ | — | 没有测试 system + user + assistant(tool_call) + tool(result) 的完整消息序列 |

### Node/Python 绑定测试

| 功能场景 | Node | Python | 问题 |
|---------|:----:|:------:|------|
| 纯文本生成 | ✅ | ✅ | 只用 `"Hello"` prompt |
| 流式文本 | ✅ | ✅ | 只验证 TextDelta + Finish |
| 工具调用 | ❌ | ❌ | **从未传 tools 参数** |
| ToolChoice | ❌ | ❌ | **从未传 tool_choice** |
| 多角色消息 | ❌ | ❌ | **只传单条 "Hello" 字符串** |
| 结构化 content | ❌ | ❌ | **从未检查 result.raw.content** |
| 工具调用流式 | ❌ | ❌ | **从未验证 StreamPart::ToolCall/ToolInputDelta** |
| Embedding | ✅ | ✅ | e2e mock |
| Speech/Image/Transcription | ✅ | ✅ | e2e mock |

---

## 4. 缺失功能

### 绑定层 API

1. **C ABI 缺少 Reranking / Video / Search** 的 `_new` + 操作函数（3 组共 6 个函数）
2. **Node/Python 绑定没有独立的 `generate()` 方法**直接返回 `GenerateResult`（含完整 `content: Vec<GenerateContent>`）——用户只能通过 `result.raw` 间接访问结构化 content

---

## 5. 缺失测试

### Rust 侧缺失

1. **工具调用完整往返**——model 返回 ToolCall → 用户执行工具 → 把 ToolResult 作为 `ContentPart::ToolResult` 放回 messages → 再次调 `generate_text` → model 返回最终文本
2. **多轮对话**——system + user + assistant(tool_call) + tool(result) 的完整消息序列
3. **cassette 回放只用 "Hello"**——`cassette_exhaustive_test.rs` 虽然 803 个 cassette 都回放了，但全用 `GenerateTextOptions::default()`（无 tools），从未匹配到工具调用/多轮/推理的 cassette

### Node/Python 绑定缺失

1. **工具调用 e2e**——mock 返回含 `tool_calls` 的响应，验证 `result.tool_calls` + `result.raw.content` 含 `ToolCall` 变体
2. **多角色消息 e2e**——传 `[{role: "system", content: "..."}, {role: "user", content: "..."}]`，验证 provider 正确处理
3. **流式工具调用 e2e**——mock 返回含 `tool_calls` 的 SSE，验证 `StreamPart::ToolCall` / `ToolInputDelta`
4. **ToolChoice e2e**——传 `tool_choice: "required"` / `"none"` / `{type: "tool", toolName: "..."}`，验证请求体正确
5. **结构化 content 验证**——验证 `result.raw.content` 数组含多种 `GenerateContent` 变体
6. **cassette 回放**——用含 tools 的 cassette 测试，而非只匹配文本生成 cassette

---

## 6. cassette 闲置

| 场景 | 可用 cassette 数 | 实际被测试选中 | 闲置 |
|------|:----------------:|:------------:|:----:|
| 工具调用 (tool_call/tool_use/function_call) | 269 | 0 | 269 |
| 多轮对话 (chain/multi_turn/followup/multi_step) | 124 | 0 | 124 |
| 推理/思考 (reasoning/thinking) | 175 | 0 | 175 |
| 结构化输出 (structured/json_schema/response_format) | 36 | 0 | 36 |
| **合计闲置** | **604** | **0** | **604** |

这些 cassette 录制了真实的工具调用、多轮对话、推理思考、结构化输出场景，但因为测试全用 `"Hello"` + 默认 options，replay 按 model 匹配后回退到最简单的文本生成 cassette，这些场景丰富的录像从未被选中。

---

## 7. 总结

**Rust 核心层功能完整**——8 个模态 trait + 16 个 StreamPart 变体 + 5 个 GenerateContent 变体 + 4 个 ToolChoice + 4 个 Role + 9 个 ContentPart 变体，全部定义且有 serde。

**Rust provider 测试覆盖较好**——工具调用解析、ToolChoice 各值、多角色消息、结构化 content 都有测试，分布在 30+ 个测试文件中。**但缺少工具调用完整往返和多轮对话的 e2e 测试**。

**Node/Python 绑定测试严重不足**——全部测试只用 `"Hello"` 纯文本 prompt + 默认 options，**从未测试工具调用、多角色消息、ToolChoice、结构化 content、流式工具调用**。绑定层 API 能力都已暴露（tools 在 options JSON 里可传入），但没有任何测试验证这些能力从 Node/Python 侧能正常工作。

**604 个场景丰富的 cassette 完全闲置**——269 个工具调用、124 个多轮对话、175 个推理思考、36 个结构化输出的真实录像从未被任何测试选中回放。

**最关键的缺口**：工具调用完整往返（ToolCall → ToolResult → 继续生成）在所有层都没有端到端测试。这是 LLM 应用的核心场景——agent loop 的基础。

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，基于独立 agent 全面审计 |
