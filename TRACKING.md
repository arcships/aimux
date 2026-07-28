# aimux 测试与实现缺口追踪清单

> ⚠️ **此文档为 2026-07-27 的历史快照，部分内容已过时。** 当前 provider 数 221、测试数 94 文件/~57500 行。最新状态见 [HANDOFF_V2.md](HANDOFF_V2.md) 和 [TEST_AUDIT.md](TEST_AUDIT.md)。
> 生成日期：2026-07-27
> 最后更新：2026-07-27（全部完成）
> 范围：provider + 请求层统一（不含 agent loop / generate_object / MockLanguageModel）
> 原则：测试先行——已有实现的翻测试验证；未实现的先翻测试（基于 trait 接口）→ 红绿循环
>
> ## 完成状态：✅ 全部完成
>
> **初始基线**：837 tests, 0 failures
> **最终状态**：2080 tests, 0 failures, 53 ignored
> **新增测试**：1243 个
>
> | 区块 | 状态 | 新增测试 |
> |------|------|---------|
> | A1 薄封装 chat | ✅ | 237 |
> | A2 核心 chat 剩余 | ✅ | 632 |
> | B Responses API | ✅ | 232 |
> | C 非 chat 模型类型 | ✅ | 379 |
> | **合计** | | **1243** |

## 缺口分类说明

| 标记 | 含义 | 动作 |
|------|------|------|
| 🔵 测试补全 | Rust 有实现，缺测试 | 直接翻译 TS 测试 → 验证现有实现 |
| 🟡 实现+测试 | Rust 无实现，需新建 | 先翻测试（基于接口）→ 补实现 → 测试绿 |
| 🔴 trait+实现+测试 | Rust 无 trait，需从零开始 | 先定 trait → 翻测试 → 实现 → 测试绿 |

---

## A. Chat 测试补全（🔵 已有实现，纯补测试）

### A1. openai-compatible 薄封装 provider（14 个）

> 现状：13 个 provider 共用 `openai_compatible_test.rs` 的 2 个宏（`openai_compatible_tests!` 4 例 + `openai_compatible_tool_tests!` 2 例 = 每 provider 6 例），仅 groq 有 2 个专属 standalone 测试。`tests/` 下无任何 `<provider>_test.rs` 专属文件。provider 特有逻辑（env var、默认 base URL、区域端点、自定义 headers、reasoning_effort 映射、消息转换、usage 转换、prepare-tools）基本未测。

- [ ] **groq** — TS 115 例（chat）+ 0 responses。缺消息转换/usage 转换/prepare-tools/reasoning/snapshot 对比。Rust 仅 8 例通用 macro
- [ ] **alibaba** — TS 80 例。缺 Qwen 特有选项/消息转换/cache-control。Rust 仅 6 例
- [ ] **perplexity** — TS 48 例。缺消息转换/sonar 特性。Rust 仅 6 例
- [ ] **deepinfra** — TS 19 例。缺 provider 配置/headers 细节。Rust 仅 6 例
- [ ] **cerebras** — TS 22 例。缺 cerebras 特有配置。Rust 仅 6 例
- [ ] **moonshotai** — TS 23 例。缺 usage 转换/provider 配置。Rust 仅 6 例
- [ ] **xai** — TS 136 例（chat）。缺 reasoning effort/tools/消息转换/usage/error 全套。Rust 仅 6 例（Responses API 见 B2）
- [ ] **huggingface** — TS 10 例（chat）。缺 provider 配置细节。Rust 仅 6 例（Responses API 见 B3）
- [ ] **togetherai** — TS 19 例。缺 provider 配置细节。Rust 仅 6 例
- [ ] **fireworks** — TS 27 例。缺 fireworks 特有配置/headers。Rust 仅 6 例
- [ ] **baseten** — TS 35 例。缺 provider 配置/委派单测。Rust 仅 6 例
- [ ] **bytedance** — TS 无 chat 模型（仅 image/video）。Rust 自行加了 chat 封装（双 env var `ARK_API_KEY`/`BYTE_DANCE_API_KEY` + 中国端点）。缺双 env var 回退与端点常量测试。**⚠️ 需确认是否保留该 chat 封装**
- [ ] **vercel** — TS 6 例全是 provider 配置接线单测（mock）。Rust 6 例是真实 chat 行为（mock server）。两边焦点不重叠，缺 vercel 配置接线测试
- [ ] **open-responses** — TS 0 例 chat（仅有 responses）。Rust 无实现。**整 provider 不存在**（见 B4）

**A1 汇总**：TS chat 用例 ~520 例，Rust 现有 ~80 例，缺口 ~440 例

### A2. 已覆盖核心 provider 的剩余 chat 测试（10 个）

- [ ] **anthropic** — TS 468 例(12 文件) / Rust 206 例(6 文件)。缺：
  - [ ] context management API（`context-management-2025-06-27` beta，模型行为/请求体）
  - [ ] mid-conversation tool-change blocks（tool_addition/tool_removal）
  - [ ] Files API（`anthropic-files.test.ts`，12 例 → 见 C7）
  - [ ] provider 配置/headers/自定义 fetch（`anthropic-provider.test.ts`，14 例）
  - [ ] unknown-model max-output-tokens 能力（3 例）
  - [ ] sanitize-json-schema（5 例）
  - [ ] anthropic-language-model 277 例的细粒度场景（Rust 仅 45 例）
- [ ] **anthropic-aws** — TS 52 例(2 文件) / Rust 13 例(1 文件)。缺：
  - [ ] 独立 SigV4 fetch 签名细节（`anthropic-aws-fetch.test.ts`，25 例）
  - [ ] provider 配置层（`anthropic-aws-provider.test.ts`，27 例：baseURL/headers/自定义 fetch/model 创建）
- [ ] **azure** — TS 63 例(1 文件) / Rust 22 例(1 文件)。缺：
  - [ ] Responses API（~44 例 → 见 B5）
  - [ ] DeepSeek on Azure（`AzureDeepSeekLanguageModelOptions`）
  - [ ] completion API
- [ ] **amazon-bedrock** — TS 410 例(17 文件) / Rust 105 例(3 文件)。缺：
  - [ ] legacy anthropic 子 provider（`anthropic/amazon-bedrock-anthropic-*.test.ts`，38 例）
  - [ ] mantle provider（`mantle/bedrock-mantle-provider.test.ts`，14 例）
  - [ ] ARN URL / inference-profile（`arn:aws:bedrock:...:inference-profile/...`）
  - [ ] normalize-tool-call-id（10 例）
  - [ ] inject-fetch-headers（4 例）
  - [ ] sigv4-fetch 独立测试（27 例）
  - [ ] provider 配置（`amazon-bedrock-provider.test.ts`，22 例）
  - [ ] embedding/image/reranking 模型（→ 见 C1/C2/C6）
  - [ ] chat model 主测试 137 例 vs Rust 19 例的边缘场景
- [ ] **cohere** — TS 75 例(5 文件) / Rust 32 例(1 文件)。缺：
  - [ ] embedding/reranking 模型（→ 见 C1/C6）
  - [ ] 独立 prepare-tools 测试（`cohere-prepare-tools.test.ts`，7 例）
  - [ ] 独立 convert 测试（`convert-to-cohere-chat-prompt.test.ts`，13 例）
- [ ] **deepseek** — TS 41 例(3 文件) / Rust 21 例(1 文件，仅 reasoning)。缺：
  - [ ] **基础 chat model 行为**（`deepseek-chat-language-model.test.ts`，27 例：text/usage/tool/stream/errors/finish-reason）— Rust 只测 reasoning
  - [ ] 消息转换（`convert-to-deepseek-chat-messages.test.ts`，10 例）
  - [ ] prepare-tools（`deepseek-prepare-tools.test.ts`，4 例）
- [ ] **google** — TS 698 例(25 文件) / Rust 125 例(2 文件)。缺：
  - [ ] interactions API（整个子目录 9 文件，182 例）
  - [ ] realtime API（2 文件，51 例）
  - [ ] embedding/image/speech/video/files 模型（→ 见 C1/C2/C3/C5/C7）
  - [ ] json-accumulator（31 例）
  - [ ] model-capabilities（0 it，describe 风格）
  - [ ] supported-file-url（4 例）
  - [ ] convert-json-schema-to-openapi-schema（19 例）
  - [ ] get-model-path（3 例）
  - [ ] provider 配置（`google-provider.test.ts`，22 例）
  - [ ] 独立 prepare-tools（36 例）/ convert（58 例）覆盖不足
- [ ] **google-vertex** — TS 219 例(18 文件) / Rust 14 例(1 文件，仅基础 chat)。缺：
  - [ ] embedding/image/transcription/video 模型（→ 见 C1/C2/C4/C5）
  - [ ] anthropic-on-vertex 子 provider（3 文件，32 例）
  - [ ] maas（Model Garden）子 provider（3 文件，30 例）
  - [ ] xai-on-vertex 子 provider（3 文件，23 例）
  - [ ] edge runtime（auth + provider，3 文件，21 例）
  - [ ] provider-base 认证/google-auth-library（31 例）
  - [ ] provider 配置（7 例）
- [ ] **mistral** — TS 91 例(7 文件) / Rust 38 例(1 文件)。缺：
  - [ ] embedding/speech 模型（→ 见 C1/C3）
  - [ ] 独立 convert 测试（14 例）
  - [ ] 独立 usage 测试（6 例）
  - [ ] 独立 prepare-tools 测试（4 例）
- [ ] **openai** — TS 608 例(22 文件) / Rust 151 例(5 文件)。缺：
  - [ ] Responses API（6 文件，357 例 → 见 B1）
  - [ ] completion 模型（18 例 → 见 C 后续）
  - [ ] embedding/image/files/realtime/speech/transcription/skills 模型（→ 见 C1/C2/C7/C3/C4）
  - [ ] forward-compatible-defaults（3 例）
  - [ ] language-model-capabilities（describe 风格）
  - [ ] provider 配置（`openai-provider.test.ts`，7 例）
  - [ ] 独立 error 测试（`openai-error.test.ts`，1 例）

**A2 汇总**：TS ~2595 例，Rust ~606 例，缺口 ~1990 例（含部分转入 B/C 的项目）

---

## B. Responses API（🟡 需先实现，再测试）

> 现状：Rust 端 Responses API **实现为零、测试为零**。所有 provider 仅实现 `/chat/completions`，没有任何 `/v1/responses` 端点、Responses model、流式事件解析。trait 已有（`LanguageModel`），但请求/响应格式完全不同，需新建转换层。
> 落地顺序：openai 先行（范本）→ open-responses（通用封装）→ xai（独立事件超集）→ huggingface（最轻量）→ azure（工厂层，依赖 openai）

### B1. openai Responses（最大缺口）

- [ ] **实现**：新建 `aimux-providers/src/openai/responses/` 模块（8 个源码文件，6302 行 TS 参考）
  - [ ] `openai_responses_language_model.rs`（请求构建 + do_generate + do_stream）
  - [ ] `openai_responses_api.rs`（HTTP 调用层）
  - [ ] `convert_to_openai_responses_input.rs`（消息→input 数组转换）
  - [ ] `openai_responses_prepare_tools.rs`（工具准备）
  - [ ] `openai_responses_language_model_options.rs`
  - [ ] `openai_responses_provider_metadata.rs`
  - [ ] `convert_openai_responses_usage.rs`
  - [ ] `map_openai_responses_finish_reason.rs`
- [ ] **测试**：翻译 6 个测试文件 / 17434 行 / 357 例
  - [ ] `openai-responses-language-model.test.ts`（9374 行，175 例）
  - [ ] `convert-to-openai-responses-input.test.ts`（5474 行，120 例）
  - [ ] `openai-responses-prepare-tools.test.ts`（2026 行，52 例）
  - [ ] `openai-responses-computer.test.ts`（411 行，3 例）
  - [ ] `openai-responses-api.test.ts`（88 行，6 例）
  - [ ] `convert-to-openai-responses-input-tool-search.test.ts`（61 行，1 例）
- [ ] **关键能力验收点**：
  - [ ] 请求构建：input 数组 / instructions / previous_response_id / store / metadata / include / textVerbosity / truncation / logprobs / serviceTier / parallelToolCalls / allowedTools
  - [ ] 流式解析：response.created → output_item.added → output_text.delta → output_text.done → output_item.done → response.completed/incomplete/failed 主干
  - [ ] 14 种工具：function / web_search / file_search / code_interpreter / image_generation / computer / local_shell / shell / apply_patch / mcp / custom / tool_search / programmatic_tool_calling
  - [ ] reasoning：effort(low/medium/high/xhigh/none) / summary / encrypted_content / 多 reasoning 块
  - [ ] structured output：json_schema / json_object
  - [ ] annotations：url_citation / file_citation / container_file_citation / file_path
  - [ ] 多模态输入：image URL/binary/file_id/provider reference / PDF
  - [ ] compaction / phase

### B2. xai Responses

- [ ] **实现**：在 `aimux-providers/src/xai.rs` 新增 responses 路径（7 个源码文件，2087 行 TS 参考）
- [ ] **测试**：翻译 4 个测试文件 / 5404 行 / 140 例
  - [ ] `xai-responses-language-model.test.ts`（3713 行，79 例）
  - [ ] `xai-responses-prepare-tools.test.ts`（832 行，33 例）
  - [ ] `convert-to-xai-responses-input.test.ts`（727 行，22 例）
  - [ ] `convert-xai-responses-usage.test.ts`（132 行，6 例）
- [ ] **关键能力验收点**：
  - [ ] 6 个 xAI 专属工具：web_search / x_search / code_execution / view_image / view_x_video / file_search / mcp
  - [ ] ~65 流式事件类型（xai 专属超集）
  - [ ] reasoning：effort(none/low/medium/high) / summary(auto/concise/detailed) / encrypted_content
  - [ ] cost（`cost_in_usd_ticks` in providerMetadata）
  - [ ] citations（annotations → sources）

### B3. huggingface Responses

- [ ] **实现**：在 `aimux-providers/src/huggingface.rs` 新增 responses 路径（7 个源码文件，1086 行 TS 参考）
- [ ] **测试**：翻译 1 个测试文件 / 1579 行 / 31 例
  - [ ] `huggingface-responses-language-model.test.ts`（1579 行，31 例）
- [ ] **关键能力验收点**：
  - [ ] 轻量：仅 function tool（无内置工具）
  - [ ] mime 检测（top-level-only media type resolution）
  - [ ] 消息转换（images / file parts with provider references / assistant / tool）
  - [ ] reasoning content（generate + stream）

### B4. open-responses（通用 Responses API 封装）

- [ ] **实现**：**新建 provider** `aimux-providers/src/open_responses.rs`（6 个源码文件，1891 行 TS 参考）。当前 `lib.rs` 无此模块
- [ ] **测试**：翻译 3 个测试文件 / 1880 行 / 72 例
  - [ ] `convert-to-open-responses-input.test.ts`（1017 行，31 例）
  - [ ] `open-responses-language-model.test.ts`（795 行，33 例）
  - [ ] `map-open-responses-finish-reason.test.ts`（68 行，8 例）
- [ ] **关键能力验收点**：
  - [ ] 通用封装：`OpenResponsesConfig` = { provider, providerOptionsName, url, headers, fetch, generateId }
  - [ ] 请求构建：instructions / response_format / tool_choice / tools(function) / multi-turn
  - [ ] reasoning：effort 映射 / reasoningSummary
  - [ ] 流式事件（OpenAI 子集）
  - [ ] input 类型全覆盖：input_text/image/file/video / output_text / refusal / url_citation / summary_text / reasoning / message / function_call / function_call_output
  - [ ] PDF input file / 安全（防 Object.prototype 污染）

### B5. azure Responses

- [ ] **实现**：在 `aimux-providers/src/azure/` 新增 `responses()` 工厂，复用 openai Responses 模型（TS 复用 openai，301 行工厂代码）
- [ ] **测试**：翻译 `azure-openai-provider.test.ts` 中 ~44 例 Responses 专属用例
- [ ] **关键能力验收点**（Azure 专属，超出 openai 部分）：
  - [ ] Microsoft Entra ID tokenProvider（每请求调用，与 apiKey 互斥）
  - [ ] api-version query 参数（默认 + 修改）
  - [ ] Azure 文件 ID `assistant-` 前缀透传 / 非 assistant 回退 base64
  - [ ] provider metadata `azure` namespace
  - [ ] include 透传（file_search_call.results 等）

**B 汇总**：5 provider，TS ~28154 行测试 / ~644 例，Rust 0

---

## C. 非 chat 模型类型（🔴 需先定 trait，再实现，再测试）

> 现状：Rust 侧 7 类非 chat 模型**全部零实现**——trait 未定义、无 provider 实现、无测试。`grep` `EmbeddingModel|ImageModel|SpeechModel|TranscriptionModel|VideoModel|RerankingModel|FilesV4` 在整个 Rust 仓 0 命中。
> 落地顺序：每类先定 trait（`aimux-core/src/`）→ 翻测试 → 实现 provider。从最简 provider 打通闭环（如 Files·anthropic 95 行、Embedding·voyage 164 行）。

### C1. Embedding（`EmbeddingModelV4`）

- [ ] **定义 trait** `aimux-core/src/embedding_model.rs`
  - 方法：`do_embed(options: &EmbeddingCallOptions) -> Result<EmbeddingResult, AiMuxError>`
  - 只读字段：`provider()` / `model_id()` / `max_embeddings_per_call` / `supports_parallel_calls`
  - 输入：`values: Vec<String>` / `abort_signal` / `provider_options` / `headers`
  - 输出：`embeddings: Vec<Embedding>` / `usage?: {tokens: u32}` / `provider_metadata`
- [ ] **provider 实现 + 测试**（7 provider / 8 测试文件 / 2233 行 / 73 例）
  - [ ] openai（1 文件，164 行，7 例）
  - [ ] google（1 文件，550 行，16 例）
  - [ ] google-vertex（1 文件，460 行，15 例）
  - [ ] mistral（2 文件，164 行，8 例）
  - [ ] cohere（1 文件，185 行，7 例）
  - [ ] voyage（1 文件，174 行，7 例）
  - [ ] amazon-bedrock（1 文件，536 行，13 例）

### C2. Image（`ImageModelV4`）

- [ ] **定义 trait** `aimux-core/src/image_model.rs`
  - 方法：`do_generate(options: &ImageCallOptions) -> Result<ImageResult, AiMuxError>`
  - 只读字段：`provider()` / `model_id()` / `max_images_per_call`
  - 输入：`prompt` / `n` / `size(WxH)` / `aspect_ratio(W:H)` / `seed` / `files` / `mask` / `provider_options`
  - 输出：`images: Vec<String> | Vec<Vec<u8>>` / `warnings` / `provider_metadata` / `response`
- [ ] **provider 实现 + 测试**（9 provider / 9 测试文件 / 8129 行 / 241 例）
  - [ ] openai（918 行，28 例）
  - [ ] google（914 行，31 例）
  - [ ] google-vertex（1244 行，37 例）— 补充发现
  - [ ] fal（930 行，25 例）
  - [ ] replicate（752 行，23 例）
  - [ ] black-forest-labs（762 行，23 例）
  - [ ] prodia（623 行，19 例）
  - [ ] luma（1001 行，27 例）
  - [ ] amazon-bedrock（985 行，28 例）— 补充发现

### C3. Speech / TTS（`SpeechModelV4`）

- [ ] **定义 trait** `aimux-core/src/speech_model.rs`
  - 方法：`do_generate(options: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError>`
  - 只读字段：`provider()` / `model_id()`
  - 输入：`text` / `voice` / `output_format` / `instructions` / `speed` / `language` / `provider_options`
  - 输出：`audio: String | Vec<u8>` / `warnings` / `response`
- [ ] **provider 实现 + 测试**（9 provider / 9 测试文件 / 2326 行 / 98 例）
  - [ ] openai（202 行，8 例）
  - [ ] elevenlabs（179 行，7 例）
  - [ ] cartesia（349 行，15 例）
  - [ ] hume（214 行，8 例）
  - [ ] lmnt（197 行，8 例）
  - [ ] google（356 行，16 例）— 补充发现
  - [ ] mistral（346 行，16 例）— 补充发现
  - [ ] fal（128 行，5 例）— 补充发现
  - [ ] deepgram（355 行，15 例）— 补充发现
  - [ ] google-vertex — 仅类型别名复用 google，无独立 impl/test

### C4. Transcription / STT（`TranscriptionModelV4`）

- [ ] **定义 trait** `aimux-core/src/transcription_model.rs`
  - 方法：`do_generate(options: &TranscriptionCallOptions) -> Result<TranscriptionResult, AiMuxError>` + 可选 `do_stream`
  - 只读字段：`provider()` / `model_id()`
  - 输入：`audio: Vec<u8> | String(base64)` / `media_type` / `provider_options`
  - 输出：`text` / `segments: Vec<{text, start_second, end_second}>` / `language` / `duration_in_seconds`
  - 流式：唯一带流式的非 chat 模型，`do_stream` 用默认方法返回 Unsupported，provider 按需覆写
- [ ] **provider 实现 + 测试**（9 provider / 9 测试文件 / 3435 行 / 108 例）
  - [ ] openai（997 行，27 例）
  - [ ] assemblyai（729 行，20 例）
  - [ ] deepgram（218 行，9 例）
  - [ ] revai（156 行，5 例）
  - [ ] google-vertex（356 行，17 例）— 补充发现
  - [ ] fal（142 行，5 例）— 补充发现
  - [ ] elevenlabs（194 行，7 例）— 补充发现
  - [ ] cartesia（435 行，11 例）— 补充发现
  - [ ] gladia（208 行，7 例）— 补充发现

### C5. Video（`VideoModelV4`）

- [ ] **定义 trait** `aimux-core/src/video_model.rs`
  - 方法：`do_generate(options: &VideoCallOptions) -> Result<VideoResult, AiMuxError>`
  - 只读字段：`provider()` / `model_id()` / `max_videos_per_call`
  - 输入：`prompt` / `n` / `aspect_ratio` / `resolution` / `duration` / `fps` / `seed` / `image` / `frame_images` / `input_references` / `generate_audio`
  - 输出：`videos: Vec<VideoData>`（tagged union：url / base64 / binary）
- [ ] **provider 实现 + 测试**（6 provider / 6 测试文件 / 6519 行 / 246 例）
  - [ ] klingai（1869 行，96 例）
  - [ ] fal（909 行，31 例）
  - [ ] google（1026 行，32 例）— 补充发现
  - [ ] google-vertex（1209 行，37 例）— 补充发现
  - [ ] replicate（987 行，35 例）— 补充发现
  - [ ] prodia（519 行，15 例）— 补充发现

### C6. Reranking（`RerankingModelV4`）

- [ ] **定义 trait** `aimux-core/src/reranking_model.rs`
  - 方法：`do_rerank(options: &RerankingCallOptions) -> Result<RerankingResult, AiMuxError>`
  - 只读字段：`provider()` / `model_id()`
  - 输入：`documents: RerankingDocuments`（enum：Text / Object）/ `query` / `top_n`
  - 输出：`ranking: Vec<{index: u32, relevance_score: f64}>`（按相关度降序）
- [ ] **provider 实现 + 测试**（3 provider / 3 测试文件 / 766 行 / 35 例）
  - [ ] cohere（243 行，12 例）
  - [ ] voyage（211 行，10 例）
  - [ ] amazon-bedrock（312 行，13 例）— 补充发现

### C7. Files（`FilesV4`）

- [ ] **定义 trait** `aimux-core/src/files.rs`
  - 方法：`upload_file(options: &UploadFileCallOptions) -> Result<UploadFileResult, AiMuxError>`（注意：非 `do_` 前缀）
  - 只读字段：`provider()`
  - 输入：`data: FileData`（enum：Data / Text）/ `media_type` / `filename` / `provider_options`
  - 输出：`provider_reference: SharedProviderReference` / `media_type` / `filename`
- [ ] **provider 实现 + 测试**（3 provider / 3 测试文件 / 985 行 / 41 例）
  - [ ] openai（188 行，8 例）
  - [ ] anthropic（224 行，12 例）
  - [ ] google（573 行，21 例）— 补充发现

**C 汇总**：7 trait 全未定义，46 provider / 47 测试文件 / 24393 行 / 842 例

---

## D. 清单勘误

| 项 | 原任务清单 | 仓库实际 | 处理 |
|----|----------|---------|------|
| Embedding · amelan | 列出（如有） | provider 包不存在 | 移除 |
| Speech · gladia | 列出 | 无 speech（仅 transcription） | 移出 Speech，归入 Transcription |
| Video · luma | 列出 | 无 video（仅 image） | 移出 Video，仅保留 Image |
| bytedance · chat | — | TS 无 chat（仅 image/video），Rust 自行加了 chat 封装 | ⚠️ 需确认是否保留 |

---

## E. 总量统计

| 类别 | 标记 | TS 测试行数 | TS 用例数 | Rust 现有用例 | 缺口用例 |
|------|------|-----------|---------|------------|---------|
| A1. 薄封装 chat 测试 | 🔵 | — | ~520 | ~80 | ~440 |
| A2. 核心 chat 剩余 | 🔵 | — | ~2595 | ~606 | ~1990 |
| B. Responses API | 🟡 | ~28154 | ~644 | 0 | ~644 |
| C. 非 chat 模型类型 | 🔴 | ~24393 | ~842 | 0 | ~842 |
| **合计** | | **~90000+** | **~4600** | **~686** | **~3916** |

---

## F. 建议落地顺序

1. **A1 薄封装 chat 测试补全** — 风险最低，Rust 有实现，翻完立刻跑，第一批绿
2. **A2 核心 chat 剩余测试** — 同上，优先 deepseek（只测了 reasoning）和 google-vertex（仅 14 例）
3. **B1 openai Responses** — 最大单项缺口，是 B5 azure 的基础
4. **B4 open-responses** — 通用封装，可复用 openai 事件 schema
5. **B2 xai Responses** — 独立事件超集
6. **B3 huggingface Responses** — 最轻量
7. **B5 azure Responses** — 工厂层，依赖 B1
8. **C7 Files** — 最简 trait（仅 upload_file），3 provider
9. **C1 Embedding** — 简单（单次调用，无流式），7 provider
10. **C6 Reranking** — 简单（单次调用），3 provider
11. **C3 Speech** — 简单（仅 do_generate），9 provider
12. **C4 Transcription** — 中等（可选流式），9 provider
13. **C2 Image** — 中等，9 provider
14. **C5 Video** — 中等，6 provider
