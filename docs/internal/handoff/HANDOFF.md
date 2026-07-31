> **注：aimux-tools 和 aimux-macros 已于 2026-07-31 删除。文中相关内容已过时，仅保留历史记录。**

﻿# Handoff: aimux 后续工作

> ⚠️ **此文档已被 [HANDOFF_V2.md](HANDOFF_V2.md) 取代。** 以下数据为 2026-07-26 的快照，仅供参考。
> 交接日期：2026-07-26
> 当前状态：837 tests, 0 failures, 23 LLM providers, V4-aligned architecture
> 最新更新：Anthropic provider 能力增强完成（thinking/reasoning, cache_control, provider-defined tools）

## 项目现状

### 已完成

| 维度 | 状态 |
|------|------|
| 架构 | V4 对齐：用户面 `generate_text`/`stream_text` ↔ provider 面 `do_generate`/`do_stream` 分离 |
| LLM Provider | 23 个（OpenAI/Anthropic/Google/Azure/Bedrock/Vertex/Anthropic-AWS/Mistral/Cohere + 14 个 openai-compatible） |
| 测试 | 837 个（737 providers + core/stream/utils），0 失败，11 ignored（需 providerMetadata/fixture 的深层功能） |
| 文档 | 14 份研究文档（docs/01-14）覆盖 AI SDK 全貌 |
| 参考源码 | `reference/ai/`（vercel/ai 浅克隆） |
| Anthropic 能力增强 | ✅ thinking/reasoning + cache_control + provider-defined tools 已完成 |

### Workspace 结构

```
aimux-core          — LanguageModel trait, 用户面 API, V4 类型
aimux-provider-utils — API key, HTTP, headers, URL, retry, error parsing
aimux-providers     — 23 个 provider 实现
aimux-stream        — SSE 解析, NDJSON, StreamingToolCallTracker, extract_lines
```

---

## 需要补充的功能

### P0：核心能力缺口

#### 1. Agent 层（ToolLoopAgent）

**现状**：`generate_text` / `stream_text` 是纯单次调用。用户需要自己写循环：调 `generate_text` → 检查 `tool_calls` → 执行工具 → 把 `ToolResult` 加入 messages → 再调 `generate_text`。

**设计决策**：不在 `generate_text` 内置循环。Agent loop 作为独立层，`generate_text`/`stream_text` 保持纯单次调用职责清晰。AI SDK 5.x 把 `stopWhen` 循环内置进 `generateText` 导致 API 语义混淆（既是单次又是多步），Rust 版避免这个问题。

**需要做**：
- 在 `aimux-core` 或新 `aimux-agent` crate 中实现 `ToolLoopAgent`

- `stream_text` 版本用 stitchable stream（顺序合并多个 step 流）
- 参考：`docs/04-core-mechanisms.md` 第 1 节 + 第 4 节，TS 源 `agent/tool-loop-agent.ts`

#### 2. 结构化对象生成（generate_object / stream_object）

**现状**：`CallOptions` 有 `ResponseFormat::Json` 但没有用户面的 `generate_object` 函数。

**需要做**：
- 在 `aimux-core/src/generate.rs` 中加 `generate_object<T: DeserializeOwned>` 函数
- 通过 `response_format: Json { schema }` 传 JSON Schema 给 provider
- 解析返回的 JSON 文本 → `serde_json::from_str` → `T`
- 实现部分 JSON 流式解析（`parse_partial_json` 已在 `aimux-core/src/util.rs` 中）
- 实现 `repair_text` 回调修复损坏的 JSON（`fix_json` 已在 `aimux-core/src/util.rs` 中）
- 参考：`docs/04-core-mechanisms.md` 第 2 节

#### 3. MockLanguageModel（测试基础设施）

**现状**：没有 mock 模型，无法测试 `generate_text`/`stream_text` 的编排逻辑（多步循环、工具执行等）。

**需要做**：
- 在 `aimux-core/src/test.rs` 或独立 crate 中实现 `MockLanguageModel`
- 支持：单值返回 / 数组按步返回 / 闭包动态返回
- 记录所有调用参数（`do_generate_calls: Mutex<Vec<CallOptions>>`）
- 参考：`docs/11-test-architecture.md` Mock 模型体系部分

### P1：Provider 功能补全

#### 4. Anthropic reasoning/thinking 支持 ✅ 已完成

**现状**：已实现。`build_request_body` 处理 `providerOptions.anthropic.thinking` 和顶层 `reasoning` 映射（含模型能力检测、budget 计算、warnings）。响应解析 `thinking` content block 为 `GenerateContent::Reasoning`。流式 emit `StreamPart::Reasoning*`。

**测试**：30 个 reasoning 测试全部通过（含 1 个 un-ignored 的响应解析测试）。

#### 5. Anthropic provider-defined tools ✅ 已完成

**现状**：已实现。`build_request_body` 调用 `prepare_tools_with_provider`，支持 `Tool::Provider` 传入。响应解析 `server_tool_use` 为 `GenerateContent::ToolCall`。`mcpServers` 通过 `provider_options` 传递。beta headers 自动生成。

**测试**：41 个 provider tools 测试通过，11 个 ignored（需 providerMetadata/fixture 的深层功能）。

#### 6. Anthropic cache_control ✅ 已完成

**现状**：已实现。`convert_part_to_anthropic` 读取 `provider_options.anthropic.cacheControl` 并写入 `cache_control`。`LanguageModelPromptMessage` 加了 `provider_options` 字段支持 message-level cache_control。`ContentPart::ToolCall`/`ToolResult` 加了 `provider_options`。

**测试**：16 个 cache_control 测试全部通过。

#### 7. Google provider-defined tools

**现状**：未实现。Google 的 `google_search` / `code_execution` / `url_context` 等 provider-defined tools 未支持。

**需要做**：
- 在 `google/convert.rs` 的 `prepare_tools` 中支持 provider tool 类型
- 解析响应中的 `groundingMetadata` / `webSearchQueries`
- 参考：`docs/03-provider-implementations.md` Google 实现要点

#### 8. Bedrock reasoning 支持

**现状**：未实现。

**需要做**：
- 在 `bedrock/convert.rs` 中把 `reasoning` 映射为 `inferenceConfig.thinking`
- 解析响应中的 reasoning content

#### 9. DeepSeek reasoning_content ✅ 已完成

**已实现**：DeepSeek 已有独立 `DeepSeekModel`（`aimux-providers/src/deepseek/model.rs`），
不再是 openai-compatible 薄封装。`reasoning_content` 字段被解析为
`GenerateContent::Reasoning` / `StreamPart::Reasoning*`，提取为 `ContentPart::Reasoning`。

### P2：架构完善

#### 10. Middleware 系统

**现状**：未实现。

**需要做**：
- 实现 `LanguageModelMiddleware` trait（`transform_params` / `wrap_generate` / `wrap_stream`）
- 实现 `wrap_language_model(model, middleware)` 函数（reverse-reduce 包装）
- 内置中间件：`default_settings`、`extract_json`、`extract_reasoning`
- 参考：`docs/04-core-mechanisms.md` 第 3 节

#### 11. Provider Registry + 字符串模型解析

**现状**：用户必须手动创建 provider 和 model 实例。不支持 `generate_text("openai/gpt-4o", ...)` 字符串解析。

**需要做**：
- 实现 `ProviderRegistry`（`provider:model` 复合 id 查找）
- 实现 `customProvider`（聚合多 provider）
- 全局默认 provider（`AI_SDK_DEFAULT_PROVIDER`）
- 参考：`docs/07-kernel-infrastructure.md` 第 2 节

#### 12. Telemetry / 内核钩子

**现状**：未实现。

**需要做**：
- 实现 `Telemetry` trait（`on_start` / `on_step_start` / `on_end` 等回调）
- `create_telemetry_dispatcher` 派发器
- 与 OpenTelemetry 集成（`aimux-otel` crate）
- 参考：`docs/07-kernel-infrastructure.md` 第 3 节

#### 13. Prompt 标准化管道

**现状**：`convert_to_language_model_prompt` 只做了最基本的 string→TextPart 转换。

**需要做**：
- 实现完整的 `standardize_prompt`（互斥校验、instructions 处理）
- 实现 `prepare_tools` 排序
- 实现 `prepare_tool_choice` 映射
- 实现 `prepare_call_options` 字段验证
- 实现资源下载（URL→bytes）
- 实现 tool-call/tool-result 配对完整性检查
- 参考：`docs/07-kernel-infrastructure.md` 第 1 节

### P3：扩展能力

#### 14. 其他模型类型

**现状**：只实现 `LanguageModelV4`（chat）。

**需要做**（按 AI SDK V4 规范）：
| 模型类型 | 方法 | 优先级 |
|----------|------|--------|
| `EmbeddingModelV4` | `do_embed` | 高（简单，单次调用） |
| `TranscriptionModelV4` | `do_generate`/`do_stream` | 中 |
| `ImageModelV4` | `do_generate` | 中 |
| `SpeechModelV4` | `do_generate` | 低 |
| `RerankingModelV4` | `do_rank` | 低 |
| `VideoModelV4` | `do_generate` | 低 |
| `RealtimeModelV4` | WebSocket | 低 |
| `FilesV4` | `upload_file` | 低 |
| `SkillsV4` | `upload_skill` | 低 |

参考：`docs/10-standards-and-reference-design.md` V4 规范清单

#### 15. Conformance Test Harness

**现状**：没有跨 provider 统一测试。每个 provider 各写各的。

**需要做**：
- 定义 `ConformanceTest` trait
- 用 `rstest` 参数化跑统一 doGenerate/doStream 矩阵
- 所有 provider 必须通过的基线测试集
- 这是 Rust 版独有优势（TS 版没有）
- 参考：`docs/12-test-reusability.md` Conformance test 部分

#### 16. 文件上传（uploadFile / uploadSkill）

**现状**：未实现。

**需要做**：
- 实现 `uploadFile` / `uploadSkill` AI 函数
- 实现 `FilesV4` / `SkillsV4` provider 接口
- 实现 `SharedV4ProviderReference` 在消息中的引用
- 参考：`docs/09-skills-files-codemod.md`

#### 17. HTTP 框架集成

**现状**：无。用户不能直接在 axum/actix 中使用。

**需要做**：
- `to_text_stream_response` / `pipe_text_stream_to_response`（SSE 响应封装）
- `to_ui_message_stream_response`
- axum feature flag
- 参考：`docs/07-kernel-infrastructure.md` text-stream 部分

#### 18. Realtime 会话

**现状**：未实现。

**需要做**：
- `AbstractRealtimeSession`（WebSocket 双向音频/文本）
- `RealtimeEventReducer`（状态机）
- `BrowserRealtimeTransport`（WS 封装）
- 参考：`docs/07-kernel-infrastructure.md` realtime 部分

---

## 需要增加的测试

### 已实现功能但缺测试的

| 功能 | 缺口 | 优先级 |
|------|------|--------|
| Anthropic reasoning/thinking | ~200 个 TS 用例 | P1（需先实现功能） |
| Anthropic provider-defined tools | ~100 个 TS 用例 | P1 |
| Anthropic cache_control | ~4 个 TS 用例 | P1 |
| Google provider-defined tools | ~100 个 TS 用例 | P1 |
| Bedrock reasoning | ~50 个 TS 用例 | P1 |
| generate_text 多步循环 | ~260 个 TS 用例 | P0（需先实现功能） |
| generate_object | ~100 个 TS 用例 | P0（需先实现功能） |
| middleware | ~100 个 TS 用例 | P2 |

### 未实现功能的测试（功能实现后再翻译）

这些 TS 测试对应 Rust SDK 尚未实现的功能，当功能实现后需要翻译：

- Anthropic: web_search/web_fetch/code_execution/mcp/advisor/memory/context_management（~200 例）
- Google: search tool selection/urlContextMetadata/thinkingLevel（~100 例）
- Azure: responses API tools/deepseek/completion（~30 例）
- Bedrock: legacy anthropic/ARN URL（~50 例）
- 各 provider 的 embedding/image/transcription/speech 测试（功能未实现）

---

## 已知技术债务

1. ~~**`anthropic/convert.rs` vs `convert_full.rs` 双模块**~~：✅ 已解决。`convert_full.rs` 已合并到 `convert.rs`，`model.rs` 使用合并后的模块。

2. **`ContentPart` 字段膨胀**：为翻译测试而添加的 `provider_options`/`filename` 字段散布在多个变体上，部分 provider 的 match 分支是 stub（未真正处理）。

3. ~~**f32 精度**~~：✅ 已解决。`temperature`/`top_p`/`top_k`/`presence_penalty`/`frequency_penalty` 已改为 f64。

4. ~~**`CallOptions.reasoning` 类型**~~：✅ 已解决。已改为强类型枚举 `ReasoningEffort { ProviderDefault, None, Minimal, Low, Medium, High, Xhigh }`。

5. **Windows 应用控制策略**：偶尔阻止测试 .exe 执行（os error 4551），删除 `target/debug/deps/*.exe` 重建即可。非代码问题。



7. **Anthropic provider tools 剩余 11 个 ignored 测试**：需要 `providerMetadata` on `GenerateResult`、container.skills 处理、fixture 数据。属深层功能，不影响核心 provider tools 请求/响应。

8. **`CallOptions.tools` 类型变更**：已从 `Option<Vec<FunctionTool>>` 改为 `Option<Vec<Tool>>`（`Tool = Function(FunctionTool) | Provider(ProviderTool)`）。所有 provider 已适配（Provider 变体暂时过滤丢弃，仅 Anthropic 接通 `prepare_tools_with_provider`）。

---

## 参考资源

| 文档 | 内容 |
|------|------|
| `docs/01-09` | AI SDK 架构研究（已复核） |
| `docs/10` | V4 规范清单（trait 设计蓝本） |
| `docs/11` | 测试体系结构 |
| `docs/12` | 测试用例可复用性分析 |
| `docs/13` | 测试统计审计（7153 用例 / 381 文件） |
| `docs/14` | 测试翻译完整度审计 |
| `reference/ai/` | Vercel AI SDK TS 源码（浅克隆） |
