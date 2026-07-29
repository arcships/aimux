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
| StreamPart 17 个变体（含 `Raw`） | `stream_part.rs:16` | ✅ 完整覆盖 |
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
| Raw | provider 原始 chunk（调试用，`include_raw_chunks` 时） |

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
| Reranking | ✅ | ✅ `cohereReranking` | ✅ `cohere_reranking` | ✅ `aimux_cohere_reranking_new` + `aimux_rerank`（2026-07-29 补） |
| Video | ✅ | ✅ `googleVideo` | ✅ `google_video` | ✅ `aimux_google_video_new` + `aimux_video_generate`（2026-07-29 补） |
| Search | ✅ | ✅ class 暴露 | ✅ class 暴露 | ✅ `aimux_tavily_search_new` + `aimux_search`（2026-07-29 补） |
| Files | ✅ | ✅ `openaiFiles` | ✅ `openai_files` | ✅ `aimux_file_upload` |

### 缺失

- ~~**C ABI 缺少 Reranking / Video / Search 的 `_new` + 操作函数**（3 组共 6 个函数）~~ ✅ **已补（2026-07-29）**：`aimux_cohere_reranking_new`/`aimux_rerank`、`aimux_google_video_new`/`aimux_video_generate`、`aimux_tavily_search_new`/`aimux_search`，符号已导出、编译通过。
- **Node/Python 绑定没有独立的 `generate()` 方法**直接返回 `GenerateResult`——用户只能通过 `result.raw` 访问结构化 content，不是一等公民

---

## 3. 测试覆盖

### Rust 侧（provider 测试 + e2e 测试）

| 功能场景 | 有测试？ | 测试文件 | 断言内容 |
|---------|:-------:|---------|---------|
| 纯文本生成 | ✅ | `e2e_test.rs` | text + usage + finish_reason |
| 工具调用解析（非流式） | ✅ | `openai_model_test.rs:566`, `e2e_test.rs:82` | `GenerateContent::ToolCall` + `result.tool_calls[0].tool_name` |
| 工具调用解析（流式） | ✅ | `openai_model_test.rs:780` | `StreamPart::ToolCall` + `StreamPart::ToolInputDelta` |
| ToolChoice::None | ✅ | `openai_convert_test.rs`, `cohere_model_test.rs` | ⚠️ 勘误：断言 `tool_choice=="none"`/`"NONE"`；cohere 测试里 **tools 仍在请求体内**，并非"不含 tools" |
| ToolChoice::Required | ✅ | `openai_convert_test.rs`, `cohere_model_test.rs` | 请求体含 tool_choice: required |
| ToolChoice::Tool | ✅ | `openai_convert_test.rs` | ⚠️ 勘误：实际序列化为 OpenAI 格式 `{type:"function", function:{name}}`，非 `{type:"tool", toolName}` |
| 多角色消息 (System) | ✅ | `anthropic_model_test.rs:573` | Role::System |
| 结构化 content (Source) | ✅ | 多个 provider 测试 | `GenerateContent::Source` |
| 结构化 content (Reasoning) | ✅ | 多个 provider 测试 | `GenerateContent::Reasoning` |
| 结构化 content (ToolResult) | ✅ | `xai_responses_test.rs:1391` | ⚠️ 勘误：原称 `openai_model_test.rs:568` 有误——该处 `should_parse_tool_results` 断言的是 `GenerateContent::ToolCall`（与上方"工具调用解析"同测试），非 `ToolResult`。真实 `ToolResult` 断言在 xai/google provider 测试 |
| 流式 StreamPart 序列 | ✅ | `e2e_test.rs:300` | StreamStart → TextDelta → Finish 顺序 |
| Embedding (base64 解码) | ✅ | `cassette_multimodal_test.rs` | embeddings 非空 |
| Speech/Image/Transcription/Files | ✅ | `cassette_multimodal_test.rs` | 硬断言 |
| **工具调用完整往返** | ✅ 已补 | `e2e_test.rs:e2e_openai_tool_call_round_trip` | 2026-07-29 新增：ToolCall → 执行 → ToolResult 回填 → 第二次 generate_text → 最终文本，并验证第二次请求体含 assistant(tool_calls)+tool(result) |
| **多轮对话（e2e 往返）** | ✅ 已补 | `e2e_test.rs:e2e_openai_multi_turn_dialog` | ⚠️ 勘误：转换层**原有**多轮 tool_call+tool_result 序列化测试（`anthropic_convert_test.rs:935`、`open_responses_test.rs:909`、`alibaba_test.rs:265`），审计初稿将其忽略。真实缺口是 e2e 多轮往返流（经 generate_text 的端到端），2026-07-29 已补 `system+user+assistant+user` 四轮 e2e 测试 |

### Node/Python 绑定测试

> **勘误（2026-07-29 验证）**：原表将 Embedding/Speech/Image/Transcription 标为 ✅（e2e mock）有误。这些模态在 Node/Python 绑定层 **API 已暴露**（见第 2 节），但 **绑定测试目录里完全没有对应测试**——仅在 Rust 核心 `cassette_multimodal_test.rs` 测过。下表已更正。

| 功能场景 | Node | Python | 问题 |
|---------|:----:|:------:|------|
| 纯文本生成 | ✅ | ✅ | prompt 为纯字符串（如 "What is Rust?"），非多角色 |
| 流式文本 | ✅ | ✅ | 只验证 TextDelta + Finish（+ cassette 里 StreamStart） |
| 工具调用 | ❌ | ❌ | **从未传 tools 参数** |
| ToolChoice | ❌ | ❌ | **从未传 tool_choice** |
| 多角色消息 | ❌ | ❌ | **只传单条字符串 prompt，从未传 [{role,content}] 数组** |
| 结构化 content | ❌ | ❌ | **从未检查 result.raw.content** |
| 工具调用流式 | ❌ | ❌ | **从未验证 StreamPart::ToolCall/ToolInputDelta** |
| Embedding | ❌ | ❌ | 绑定层**无测试**（仅 Rust 核心 cassette 测过） |
| Speech/Image/Transcription | ❌ | ❌ | 绑定层**无测试**（仅 Rust 核心 cassette 测过） |

---

## 4. 缺失功能

### 绑定层 API

1. ~~**C ABI 缺少 Reranking / Video / Search** 的 `_new` + 操作函数（3 组共 6 个函数）~~ ✅ **已补（2026-07-29）**
2. **Node/Python 绑定没有独立的 `generate()` 方法**直接返回 `GenerateResult`（含完整 `content: Vec<GenerateContent>`）——用户只能通过 `result.raw` 间接访问结构化 content

---

## 5. 缺失测试

### Rust 侧缺失

> **状态更新（2026-07-29 验证）**：第 1、2 项已补测试（见下方）。转换层其实**原已存在**多轮 tool_call+tool_result 序列化测试（`anthropic_convert_test.rs:935`、`open_responses_test.rs:909`、`alibaba_test.rs:265`），初稿未计入。

1. ~~**工具调用完整往返**~~ ✅ **已补** `e2e_test.rs:e2e_openai_tool_call_round_trip`
2. ~~**多轮对话**~~ ✅ **已补** `e2e_test.rs:e2e_openai_multi_turn_dialog`（system+user+assistant+user 经 generate_text 端到端）
3. **cassette 回放只用 "Hello"**——`cassette_exhaustive_test.rs` 虽然 803 个 cassette 都回放了，但全用 `GenerateTextOptions::default()`（无 tools），从未匹配到工具调用/多轮/推理的 cassette

### Node/Python 绑定缺失

1. **工具调用 e2e**——mock 返回含 `tool_calls` 的响应，验证 `result.tool_calls` + `result.raw.content` 含 `ToolCall` 变体
2. **多角色消息 e2e**——传 `[{role: "system", content: "..."}, {role: "user", content: "..."}]`，验证 provider 正确处理
3. **流式工具调用 e2e**——mock 返回含 `tool_calls` 的 SSE，验证 `StreamPart::ToolCall` / `ToolInputDelta`
4. **ToolChoice e2e**——传 `tool_choice: "required"` / `"none"` / `{type: "tool", toolName: "..."}`，验证请求体正确
5. **结构化 content 验证**——验证 `result.raw.content` 数组含多种 `GenerateContent` 变体
6. **cassette 回放**——用含 tools 的 cassette 测试，而非只匹配文本生成 cassette
7. **模态 e2e**——Embedding/Speech/Image/Transcription 在绑定层无任何测试（初稿误标为 ✅，见第 3 节勘误）

---

## 6. cassette 闲置

> **勘误（2026-07-29 验证）**：计数按文件名匹配，四类数字 269/124/175/36 准确；但"合计 604"有重复计算——`tool∩multi=11`、`tool∩reason=10`，去重后唯一文件 **583**。"0 选中/从未被选中"表述过激且与第 5 节"803 都回放"自相矛盾：`cassette_exhaustive_test.rs` 逐个挂载并回放每个 chat-completions cassette，583 个关键词 cassette 中 228 个是 chat-completions、被逐一回放（仅断言"不 panic/可解析"）。准确表述是**"场景从未被测试"**——没有任何测试传 tools/多轮消息，匹配器 [common/replay.rs:148-176](aimux-providers/tests/common/replay.rs#L148) 只对标量字段打分（model/stream 等），忽略 `tools`/`messages`/`response_format`，故无法区分场景 cassette。

| 场景 | 可用 cassette 数（文件名匹配） | 被真正测试 | 未被测试 |
|------|:----------------:|:------------:|:----:|
| 工具调用 (tool_call/tool_use/function_call) | 269 | 0 | 269 |
| 多轮对话 (chain/multi_turn/followup/multi_step) | 124 | 0 | 124 |
| 推理/思考 (reasoning/thinking) | 175 | 0 | 175 |
| 结构化输出 (structured/json_schema/response_format) | 36 | 0 | 36 |
| **合计（去重）** | **583** | **0** | **583** |

这些 cassette 录制了真实的工具调用、多轮对话、推理思考、结构化输出场景，但没有测试传 tools/多轮消息去匹配它们：mount-all 类测试用 `"Hello"` + 默认 options，按 model 匹配后回退到最简单的文本生成 cassette；exhaustive 测试虽逐个回放但只断言"可解析"。场景丰富的录像从未被真正验证。

---

## 7. 总结

**Rust 核心层功能完整**——8 个模态 trait + **17** 个 StreamPart 变体 + 5 个 GenerateContent 变体 + 4 个 ToolChoice + 4 个 Role + 9 个 ContentPart 变体，全部定义且有 serde。

**Rust provider 测试覆盖较好**——工具调用解析、ToolChoice 各值、多角色消息、结构化 content 都有测试，分布在 30+ 个测试文件中。**工具调用完整往返和多轮对话的 e2e 测试已于 2026-07-29 补上**（`e2e_test.rs`）。

**Node/Python 绑定测试严重不足**——全部测试只用纯文本字符串 prompt + 默认 options，**从未测试工具调用、多角色消息、ToolChoice、结构化 content、流式工具调用**；Embedding/Speech/Image/Transcription 在绑定层也无任何测试（初稿误标为 ✅）。绑定层 API 能力都已暴露（tools 在 options JSON 里可传入），但没有任何测试验证这些能力从 Node/Python 侧能正常工作。

**583 个场景丰富的 cassette 未被真正测试**（去重后；初稿误计 604）——269 个工具调用、124 个多轮对话、175 个推理思考、36 个结构化输出的真实录像虽被 exhaustive 测试逐个回放，但没有任何测试传 tools/多轮消息去匹配它们，只断言"可解析"。

**最关键的缺口**：~~工具调用完整往返（ToolCall → ToolResult → 继续生成）在所有层都没有端到端测试~~ → Rust 侧已于 2026-07-29 补上；Node/Python 绑定层仍未覆盖。

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，基于独立 agent 全面审计 |
| 2026-07-29 | v0.2 | 逐条验证修正：StreamPart 17 变体（初稿漏 `Raw`）；第 3 节绑定测试模态 ✅ 误标（实际无测试）；ToolChoice::None/Tool 断言描述偏差；第 10 条 `ToolResult` 断言位置错误（实为 `ToolCall`，真实 `ToolResult` 在 xai）；B 条"无多轮序列"偏差（转换层原有）。**已补** Rust e2e：`e2e_openai_tool_call_round_trip` + `e2e_openai_multi_turn_dialog`（均通过） |
| 2026-07-29 | v0.3 | cassette 闲置勘误：合计 604→583（去重，21 文件跨类）；"0 选中/从未被选中"过激——exhaustive 测试逐个回放 228 个 chat-completions 关键词 cassette（仅断言可解析），准确表述为"场景从未被测试"；C ABI 缺失确认属实（16 个函数中无 reranking/video/search） |
| 2026-07-29 | v0.4 | **修复缺口**：C ABI 补 6 个函数（`aimux_cohere_reranking_new`/`aimux_rerank`、`aimux_google_video_new`/`aimux_video_generate`、`aimux_tavily_search_new`/`aimux_search`），符号导出、编译通过；Node 绑定补 4 个结构化 e2e 测试（工具调用解析/多角色/ToolChoice/流式工具调用，全通过）；Python 绑定补 3 个结构化 e2e 测试（工具调用解析/多角色/ToolChoice，全通过） |
