# aimux 测试覆盖审计

> 审计日期：2026-07-27（初版）
> 更新日期：2026-07-28 — trait 表、Responses API、各模型类型章节、测试基线已按当前实现状态更新
> 目标：系统性地对比 TS 源码测试与 Rust 移植测试，找出所有缺口

## 审计方法

1. 遍历 `reference/ai/packages/` 下所有 provider 包（排除非 provider 包：ai/ui框架/codemod/harness等）
2. 对每个 provider 包，逐个检查 `.test.ts` 文件对应的模型类型（chat/embedding/image/speech/transcription/video/files/realtime）
3. 对比 Rust 侧 `aimux-providers/tests/` 和 `aimux-providers/src/` 是否有对应实现和测试
4. 记录：TS 测试文件数/行数、Rust 测试文件数/行数、模型类型、缺口

## Rust 已有 trait 定义

> 更新（2026-07-28）：初版标注的"❌ 未定义"已全部过时——除 Realtime 外的所有 trait 均已定义并实现。

| Trait | 定义位置 | 状态 | 实现数 |
|-------|---------|------|------:|
| `LanguageModel` (chat) | aimux-core/src/language_model.rs | ✅ 已实现，do_generate + do_stream | 14 |
| `EmbeddingModel` | aimux-core/src/embedding_model.rs | ✅ 已定义 | 7 |
| `ImageModel` | aimux-core/src/image_model.rs | ✅ 已定义 | 9 |
| `SpeechModel` | aimux-core/src/speech_model.rs | ✅ 已定义 | 5 |
| `TranscriptionModel` | aimux-core/src/transcription_model.rs | ✅ 已定义 | 9 |
| `RerankingModel` | aimux-core/src/reranking_model.rs | ✅ 已定义 | 3 |
| `VideoModel` | aimux-core/src/video_model.rs | ✅ 已定义 | 6 |
| `Files` | aimux-core/src/files_model.rs | ✅ 已定义 | 3 |
| `RealtimeModel` | — | ❌ 未定义（WebSocket 双向音频/文本，尚未启动） | 0 |

## 逐 Provider 审计

### Chat Language Model（LanguageModel trait）

#### 已有 Rust 测试的 provider

| Provider | Rust 测试文件 | Rust 测试行数 | 缺口 |
|----------|-------------|-------------|------|
| anthropic | anthropic_model_test.rs, anthropic_convert_test.rs, anthropic_convert_full_test.rs, anthropic_prepare_tools_test.rs, anthropic_cache_control_test.rs, anthropic_provider_tools_test.rs | ~6000 | TS 有 17819 行，Rust ~6000 行。Anthropic reasoning/thinking/cache_control/provider tools 已覆盖。剩余 TS 测试涉及 context_management/mid-conversation tool changes/model capabilities 等 |
| anthropic-aws | anthropic_aws_model_test.rs | 543 | TS 1111 行。基本覆盖 |
| azure | azure_model_test.rs | 815 | TS 1857 行。缺 responses API / deepseek / completion 测试 |
| amazon-bedrock | bedrock_model_test.rs, bedrock_convert_test.rs, bedrock_reasoning_test.rs | ~3000 | TS 13575 行。reasoning 已覆盖。缺 legacy anthropic / ARN URL / mantle provider 测试 |
| cohere | cohere_model_test.rs | 1322 | TS 1912 行。基本覆盖 |
| deepseek | deepseek_reasoning_test.rs | 855 | TS 1384 行。reasoning_content 已覆盖 |
| google | google_model_test.rs, google_provider_tools_test.rs, google_embedding_test.rs, google_image_test.rs, google_video_test.rs, google_files_test.rs, google_remaining_test.rs | ~4000+ | TS 21946 行。provider tools/embedding/image/video/files 已覆盖 |
| google-vertex | vertex_model_test.rs | 509 | TS 5595 行。只有基本 model test，缺大量 |
| mistral | mistral_model_test.rs | 1420 | TS 2659 行。基本覆盖 |
| openai | openai_model_test.rs, openai_model_extended_test.rs, openai_convert_test.rs, openai_convert_extended_test.rs, openai_responses_test.rs, openai_embedding_test.rs, openai_image_test.rs, openai_speech_test.rs, openai_transcription_test.rs, openai_files_test.rs, openai_provider_test.rs | ~3100+ | TS 25985 行。Responses API/embedding/image/speech/transcription/files 已覆盖。剩余 TS 测试涉及 completion 等 |
| openai-compatible | openai_compatible_test.rs | 570 | TS 7228 行。只有 14 个薄封装的基本测试，缺 reasoning_content 等 |

#### 曾无 Rust 测试、现已补齐的 chat provider

> 更新（2026-07-28）：初版列在此表的 9 个 provider 中，8 个已补齐测试（部分还有 conformance 回放覆盖），仅 proda 仍缺。

| Provider | 现有 Rust 测试 | conformance | 说明 |
|----------|-------------|-----------|------|
| **xai** | xai_test.rs, xai_responses_test.rs | ✅ | Grok，含 Responses API |
| **groq** | groq_test.rs | ✅ | openai-compatible 封装 |
| **alibaba** | alibaba_test.rs | — | openai-compatible 封装 |
| **huggingface** | huggingface_responses_test.rs | ✅ | 含 Responses API |
| **perplexity** | perplexity_test.rs | ✅ | openai-compatible 封装 |
| **deepinfra** | deepinfra_test.rs | — | openai-compatible 封装 |
| **cerebras** | cerebras_test.rs | ✅ | openai-compatible 封装 |
| **moonshotai** | moonshotai_test.rs | — | openai-compatible 封装 |
| **open-responses** | open_responses_test.rs | — | 通用 Responses API 封装 |
| **proda** | ❌ 无 | — | 仍缺，Rust 侧尚无 chat provider 实现 |

### Responses API（LanguageModel trait，但不同请求格式）

Responses API 是 OpenAI 的新接口（`/v1/responses`），实现同一个 `LanguageModel` trait 但请求/响应格式完全不同。

| Provider | TS 源码文件数 | TS 源码行数 | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|------------|-----------|----------|----------|
| **openai** | 8 | 6302 | 6 | 17434 | ✅ `OpenAIResponsesModel`（src/openai/responses/mod.rs） | ✅ openai_responses_test.rs |
| **xai** | 7 | 2087 | 4 | 5404 | ✅ `XaiModel`（含 Responses 端点） | ✅ xai_responses_test.rs |
| **huggingface** | 7 | 1086 | 1 | 1579 | ✅ `HuggingFaceResponsesModel` | ✅ huggingface_responses_test.rs |
| **open-responses** | 3 | ~500 | 3 | 1880 | ✅ `OpenResponsesModel` | ✅ open_responses_test.rs |
| **azure** | （在 azure provider 内） | — | ~30 例 | — | ✅ `AzureResponsesModel` | ✅ azure_responses_test.rs |

### Embedding（EmbeddingModel trait）

> 更新（2026-07-28）：trait 已定义，7 个 provider 全部实现且有测试。初版的"阻塞"已解除。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| openai | 有 | ~? | ✅ `OpenAIEmbeddingModel` | ✅ openai_embedding_test.rs |
| google | 有 | ~? | ✅ `GoogleEmbeddingModel` | ✅ google_embedding_test.rs |
| google-vertex | 有 | ~? | ✅ `VertexEmbeddingModel` | ✅ vertex_embedding_test.rs |
| mistral | 有 | ~? | ✅ `MistralEmbeddingModel` | ✅ mistral_embedding_test.rs |
| cohere | 有 | ~? | ✅ `CohereEmbeddingModel` | ✅ cohere_embedding_test.rs |
| voyage | 有 | ~366 | ✅ `VoyageEmbeddingModel` | ✅ voyage_embedding_test.rs |
| amazon-bedrock | 有 | ~? | ✅ `BedrockEmbeddingModel` | ✅ bedrock_embedding_test.rs |

### Image（ImageModel trait）

> 更新（2026-07-28）：trait 已定义，9 个 provider 实现（含文档初版未列的 bedrock/vertex）且有测试。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| openai | 有 | ~? | ✅ `OpenAIImageModel` | ✅ openai_image_test.rs |
| google | 有 | ~? | ✅ `GoogleImageModel` | ✅ google_image_test.rs |
| google-vertex | 有 | ~? | ✅ `VertexImageModel` | ✅ google_vertex_image_test.rs |
| fal | 有 | ~2020 | ✅ `FalImageModel` | ✅ fal_image_test.rs |
| replicate | 有 | ~1624 | ✅ `ReplicateImageModel` | ✅ replicate_image_test.rs |
| black-forest-labs | 有 | ~809 | ✅ `BlackForestLabsImageModel` | ✅ black_forest_labs_image_test.rs |
| prodia | 有 | ~? | ✅ `ProdiaImageModel` | ✅ prodia_image_test.rs |
| luma | 有 | ~973 | ✅ `LumaImageModel` | ✅ luma_image_test.rs |
| amazon-bedrock | 有 | ~? | ✅ `BedrockImageModel` | ✅ amazon_bedrock_image_test.rs |

### Speech / TTS（SpeechModel trait）

> 更新（2026-07-28）：trait 已定义，5 个 provider 实现（openai/elevenlabs/cartesia/hume/lmnt）且有测试。gladia 实为 transcription，初版误归此类。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| openai | 有 | ~? | ✅ `OpenAISpeechModel` | ✅ openai_speech_test.rs |
| elevenlabs | 有 | ~349 | ✅ `ElevenLabsSpeechModel` | ✅ elevenlabs_speech_test.rs |
| cartesia | 有 | ~962 | ✅ `CartesiaSpeechModel` | ✅ cartesia_speech_test.rs |
| hume | 有 | ~213 | ✅ `HumeSpeechModel` | ✅ hume_speech_test.rs |
| lmnt | 有 | ~196 | ✅ `LMNTSpeechModel` | ✅ lmnt_speech_test.rs |

### Transcription / STT（TranscriptionModel trait）

> 更新（2026-07-28）：trait 已定义，9 个 provider 实现（含文档初版未列的 cartesia/elevenlabs/fal/gladia/vertex）且有测试。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| openai | 有 | ~? | ✅ `OpenAITranscriptionModel` | ✅ openai_transcription_test.rs |
| assemblyai | 有 | ~687 | ✅ `AssemblyAITranscriptionModel` | ✅ assemblyai_transcription_test.rs |
| deepgram | 有 | ~517 | ✅ `DeepgramTranscriptionModel` | ✅ deepgram_transcription_test.rs |
| revai | 有 | ~166 | ✅ `RevaiTranscriptionModel` | ✅ revai_transcription_test.rs |
| cartesia | 有 | ~? | ✅ `CartesiaTranscriptionModel` | ✅ cartesia_transcription_test.rs |
| elevenlabs | 有 | ~? | ✅ `ElevenLabsTranscriptionModel` | ✅ elevenlabs_transcription_test.rs |
| fal | 有 | ~? | ✅ `FalTranscriptionModel` | ✅ fal_transcription_test.rs |
| gladia | 有 | ~212 | ✅ `GladiaTranscriptionModel` | ✅ gladia_transcription_test.rs |
| google-vertex | 有 | ~? | ✅ `VertexTranscriptionModel` | ✅ vertex_transcription_test.rs |

### Video（VideoModel trait）

> 更新（2026-07-28）：trait 已定义，6 个 provider 实现（含文档初版未列的 google/prodia/replicate/vertex）且有测试。luma 仅有 ImageModel，无 VideoModel。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| klingai | 有 | ~1779 | ✅ `KlingAIVideoModel` | ✅ klingai_video_test.rs |
| fal | 有 | ~? | ✅ `FalVideoModel` | ✅ fal_video_test.rs |
| google | 有 | ~? | ✅ `GoogleVideoModel` | ✅ google_video_test.rs |
| prodia | 有 | ~? | ✅ `ProdiaVideoModel` | ✅ prodia_video_test.rs |
| replicate | 有 | ~? | ✅ `ReplicateVideoModel` | ✅ replicate_video_test.rs |
| google-vertex | 有 | ~? | ✅ `VertexVideoModel` | ✅ vertex_video_test.rs |

### Files（Files trait）

> 更新（2026-07-28）：trait 已定义，3 个 provider 实现（含文档初版未列的 google）且有测试。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| openai | 有 | ~? | ✅ `OpenAIFiles` | ✅ openai_files_test.rs |
| anthropic | 有 | ~188 | ✅ `AnthropicFiles` | ✅ anthropic_files_test.rs |
| google | 有 | ~? | ✅ `GoogleFiles` | ✅ google_files_test.rs |

### Reranking（RerankingModel trait）

> 更新（2026-07-28）：trait 已定义，3 个 provider 实现（含文档初版未列的 bedrock）且有测试。

| Provider | TS 测试文件数 | TS 测试行数 | Rust 实现 | Rust 测试 |
|----------|------------|-----------|----------|----------|
| cohere | 有 | ~? | ✅ `CohereRerankingModel` | ✅ cohere_reranking_test.rs |
| voyage | 有 | ~? | ✅ `VoyageRerankingModel` | ✅ voyage_reranking_test.rs |
| amazon-bedrock | 有 | ~? | ✅ `BedrockRerankingModel` | ✅ bedrock_reranking_test.rs |

## 缺口汇总

> 更新（2026-07-28）：初版列出的"第二优先：新模型类型 trait"已全部完成——8 个 trait 均已定义，56 个 provider 实现落盘并配有测试。以下仅保留尚未完成的部分。

### 已完成（初版缺口，现已关闭）

- ✅ **全部 8 个 trait 已定义**：LanguageModel / EmbeddingModel / ImageModel / SpeechModel / TranscriptionModel / VideoModel / RerankingModel / Files（仅 Realtime 未启动）
- ✅ **Responses API** — openai/xai/huggingface/open-responses/azure 全部实现 + 测试
- ✅ **Embedding** — 7 个 provider 实现 + 测试
- ✅ **Image** — 9 个 provider 实现 + 测试
- ✅ **Speech** — 5 个 provider 实现 + 测试
- ✅ **Transcription** — 9 个 provider 实现 + 测试
- ✅ **Video** — 6 个 provider 实现 + 测试
- ✅ **Reranking** — 3 个 provider 实现 + 测试
- ✅ **Files** — 3 个 provider 实现 + 测试
- ✅ **Conformance Test Harness** — 17 家 provider 录像回放真命中，2626 个 cassette

### 仍待完成

#### 第一优先：Chat provider 测试补全

1. **proda** — TS 有 1 个 chat test file，Rust 侧尚无对应 chat provider 实现（注：prodia 已有 image/video 实现，proda 是不同 provider）
2. **openai-compatible 薄封装的 unit test 补全** — groq/alibaba/perplexity/deepinfra/cerebras/moonshotai 等已有 conformance 回放覆盖，但细粒度 unit test（reasoning_content 等）仍少于 TS
3. **已覆盖 provider 的剩余测试** — anthropic context_management、azure completion、bedrock legacy/ARN、google-vertex 补全、openai completion

#### 第二优先：录像回放覆盖扩充（见 [HANDOFF_V2.md](HANDOFF_V2.md)）

4. **chatgpt / ollama / zai** — 有录像但无 conformance 挂载（OpenAI 兼容端点，改动小）
5. **有实现但无录像的 provider（~10 家）** — alibaba/baseten/bytedance/deepinfra/fireworks/moonshotai/togetherai/vercel/azure/vertex，需用 llmtape 自补录
6. **pydantic-ai 的 xai 录像（18 个）** — protobuf 格式未转换（rig 已有 62 个覆盖，优先级低）

#### 第三优先：基础设施

7. **Provider Registry** — 字符串模型解析。
8. **Prompt 标准化管道** — 输入校验/资源下载/tool 配对检查。
9. **HTTP 框架集成** — axum/actix SSE 响应封装。
10. **Realtime** — WebSocket 双向音频/文本（trait 未定义）。

### 数量统计（更新后）

| 类别 | 状态 | 备注 |
|------|------|------|
| Chat provider 补全 | 部分完成 | conformance 17 家真命中；proda 仍缺；薄封装 unit test 可继续补 |
| Responses API | ✅ 完成 | 5 个 provider 实现 + 测试 |
| Embedding | ✅ 完成 | 7 个 provider 实现 + 测试 |
| Image | ✅ 完成 | 9 个 provider 实现 + 测试 |
| Speech | ✅ 完成 | 5 个 provider 实现 + 测试 |
| Transcription | ✅ 完成 | 9 个 provider 实现 + 测试 |
| Video | ✅ 完成 | 6 个 provider 实现 + 测试 |
| Reranking | ✅ 完成 | 3 个 provider 实现 + 测试 |
| Files | ✅ 完成 | 3 个 provider 实现 + 测试 |
| Realtime | ❌ 未启动 | trait 未定义 |

## 当前 Rust 测试基线

- Rust 测试文件：94 个
- Rust 测试行数：~57500 行
- `cargo test --workspace`：EXITCODE=0，0 failed
- provider 实现模块：161 个（aimux-providers/src/）
- LanguageModel 实现：14 个类型（含 5 个 Responses API 变体）
- conformance 录像回放真命中：28 家（anthropic/alibaba/baseten/bedrock/bytedance/cerebras/chatgpt/cohere/copilot/deepinfra/deepseek/doubleword/fireworks/gemini/groq/huggingface/llamafile/mistral/mistralrs/moonshotai/ollama/openai/openrouter/perplexity/togetherai/vercel/xai/zai）
- cassette 录像：2626 个（rig 884 + pydantic-ai 1742）

## 需求定义

用户需求：**统一大部分 provider**。这意味着：

1. 每个 TS 里有 chat model 的 provider，Rust 侧都应有对应的实现 + 测试
2. 每个 TS 里有 responses API 的 provider，Rust 侧都应有对应实现 + 测试
3. 非 chat 模型类型（embedding/image/speech/...）也应覆盖，但优先级低于 chat
4. 测试应忠实翻译 TS 源码，确保行为一致性
5. 所有测试必须 `cargo test` 通过
