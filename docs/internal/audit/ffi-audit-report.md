# aimux-ffi C ABI 全量审计报告

> **审计日期**: 2026-07-31  
> **审计范围**: `aimux-ffi/src/lib.rs`, `aimux-ffi/aimux-ffi.h`, `aimux-ffi/Cargo.toml`  
> **对照基准**: Node 旗舰绑定 (`bindings/node/`)  
> **审计方法**: 逐文件实际读取, 每项结论给出文件:行号

---

## 1. 执行摘要

**aimux-ffi 的 C ABI 层已实现文本 + 全部 8 种模态的基础覆盖, 但存在 4 个构造器缺失、6 个头文件声明遗漏、全部非文本模态缺少 base_url 支持等共计 17 个缺口。** 其中 6 个为阻断级 (header 声明缺失可导致 C 编译器静默接受错误调用), 4 个为重要级 (缺失构造器 + 缺失 base_url), 7 个为建议级 (参数合同优化、provider 多样性等)。

---

## 2. 构造器覆盖矩阵

### 2.1 Node 工厂 → aimux-ffi 构造器对照

| Node 工厂函数 | 文件:行号 | aimux-ffi 对应构造器 | 文件:行号 | base_url 支持 | 缺失影响 |
|---|---|---|---|---|---|
| `openai(api_key, model_id, base_url?)` | `bindings/node/src/lib.rs:165` | `aimux_openai_new` + `_with_base` | `aimux-ffi/src/lib.rs:202,219` | ✅ 有 | — |
| `anthropic(api_key, model_id, base_url?)` | `bindings/node/src/lib.rs:188` | `aimux_anthropic_new` + `_with_base` | `aimux-ffi/src/lib.rs:245,262` | ✅ 有 | — |
| `deepseek(api_key, model_id, base_url?)` | `bindings/node/src/lib.rs:211` | ❌ 无 | — | 无 | 需用 `openai_new_with_base` 间接替代; Go binding 同样采用此 workaround (`bindings/go/multimodal.go:585`) |
| `openai_embedding(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:294` | `aimux_openai_embedding_new` | `aimux-ffi/src/lib.rs:448` | ❌ 无 `_with_base` 变体 | 无法通过 FFI 使用兼容 API (如 Azure OpenAI Embeddings) |
| `cohere_embedding(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:388` | ❌ 无 | — | 无 | Cohere embedding 不可用 |
| `google_embedding(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:426` | ❌ 无 | — | 无 | Google embedding 不可用 |
| `openai_speech(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:313` | `aimux_openai_speech_new` | `aimux-ffi/src/lib.rs:497` | ❌ 无 `_with_base` 变体 | 无法自定义 TTS endpoint |
| `openai_image(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:332` | `aimux_openai_image_new` | `aimux-ffi/src/lib.rs:533` | ❌ 无 `_with_base` 变体 | 无法使用兼容图像 API |
| `google_image(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:445` | ❌ 无 | — | 无 | Google Imagen 不可用 |
| `openai_transcription(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:351` | `aimux_openai_transcription_new` | `aimux-ffi/src/lib.rs:569` | ❌ 无 `_with_base` 变体 | 无法自定义 STT endpoint |
| `openai_files(api_key, base_url?)` | `bindings/node/src/multimodal.rs:370` | `aimux_openai_files_new` | `aimux-ffi/src/lib.rs:614` | ❌ 无 `_with_base` 变体 | 无法使用兼容文件 API |
| `cohere_reranking(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:407` | `aimux_cohere_reranking_new` | `aimux-ffi/src/lib.rs:662` | ❌ 无 `_with_base` 变体 | 无法自定义 reranking endpoint |
| `google_video(api_key, model_id, base_url?)` | `bindings/node/src/multimodal.rs:464` | `aimux_google_video_new` | `aimux-ffi/src/lib.rs:703` | ❌ 无 `_with_base` 变体 | 无法自定义视频 endpoint |
| `tavily_search` (Node 实际未暴露工厂) | — | `aimux_tavily_search_new` | `aimux-ffi/src/lib.rs:744` | ❌ 无 `_with_base` 变体 | aimux-ffi 比 Node 多暴露了 search (正向差异) |

### 2.2 构造器覆盖统计

| 类别 | 数量 |
|---|---|
| Node 工厂总数 | 13 |
| aimux-ffi 已覆盖 | 9 |
| aimux-ffi 缺失 | **4** (`deepseek`, `cohere_embedding`, `google_embedding`, `google_image`) |
| base_url 支持 (文本) | 2/2 (100%) |
| base_url 支持 (多模态) | **0/7 (0%)** |

---

## 3. 头文件声明差异

### 3.1 lib.rs 导出 vs aimux-ffi.h 声明

`aimux-ffi/src/lib.rs` 共导出 **24** 个 `pub extern "C"` 函数。`aimux-ffi/aimux-ffi.h` 仅声明了 **18** 个。**6 个函数已导出但未在头文件中声明:**

| # | 函数名 | lib.rs 行号 | aimux-ffi.h 声明 | 风险等级 |
|---|---|---|---|---|
| 1 | `aimux_cohere_reranking_new` | L:662 | ❌ 未声明 | **阻断** |
| 2 | `aimux_rerank` | L:676 | ❌ 未声明 | **阻断** |
| 3 | `aimux_google_video_new` | L:703 | ❌ 未声明 | **阻断** |
| 4 | `aimux_video_generate` | L:717 | ❌ 未声明 | **阻断** |
| 5 | `aimux_tavily_search_new` | L:744 | ❌ 未声明 | **阻断** |
| 6 | `aimux_search` | L:758 | ❌ 未声明 | **阻断** |

### 3.2 其他绑定如何处理此差异

| 绑定 | 处理方法 | 依赖的头文件 |
|---|---|---|
| **Kotlin** (`bindings/kotlin/.../Model.kt`) | 仅使用文本 API (openai/anthropic + generate/stream/drop/free), 不涉及这 6 个函数 | JNA 直接从 `.so` 动态加载, 不使用 `.h` |
| **Swift** (`bindings/swift/.../Aimux.swift`) | 仅使用文本 API, 同 Kotlin | 通过 `import CAimuxFFI` (module map 从 `.h` 生成), 不涉及 |
| **Flutter** (`bindings/flutter/lib/aimux.dart`) | 仅使用文本 API, 同 Kotlin | `dart:ffi` 通过 `DynamicLibrary.lookupFunction` 动态查找符号, 不使用 `.h` |
| **C** (`bindings/c/example.c`) | 仅使用文本 API, `#include "aimux-ffi.h"` | 直接 #include `.h`, 不涉及 |
| **Go** (`bindings/go/aimux.go`, `multimodal.go`) | ✅ **唯一全量使用者**: 在 CGo 注释块中手动重新声明了这 6 个函数 (`bindings/go/multimodal.go:20-25`) | 使用 `aimux-ffi.h` + 手动补充声明 |

### 3.3 差异根因

Go 绑定的 CGo 注释明确标注: `"Functions exported by libaimux_ffi.a but not yet declared in aimux-ffi.h."` (`bindings/go/multimodal.go:19`)。说明头文件更新滞后于实现。

---

## 4. Provider 覆盖缺口

### 4.1 按模态列出 Node vs aimux-ffi provider 对照

Rust 核心 (`aimux-providers`) 实际实现了远多于 FFI 暴露的 provider。下表仅对比 Node 已暴露的 provider 工厂。

| 模态 | Node 支持的 provider 工厂 | aimux-ffi 支持的 provider | 缺失 |
|---|---|---|---|
| **LanguageModel (文本)** | OpenAI, Anthropic, DeepSeek | OpenAI, Anthropic | DeepSeek |
| **EmbeddingModel** | OpenAI, Cohere, Google | OpenAI only | Cohere, Google |
| **SpeechModel (TTS)** | OpenAI only | OpenAI only | — |
| **ImageModel** | OpenAI, Google | OpenAI only | Google |
| **TranscriptionModel (STT)** | OpenAI only | OpenAI only | — |
| **Files** | OpenAI only | OpenAI only | — |
| **RerankingModel** | Cohere only | Cohere only | — |
| **VideoModel** | Google only | Google only | — |
| **SearchModel** | (Node 未暴露工厂) | Tavily | aimux-ffi 正向超出 Node |

### 4.2 Provider 多样性缺口汇总

| 缺失 provider | 影响模态 | 影响范围 |
|---|---|---|
| DeepSeek (LanguageModel) | 文本 | 中国用户常用; Go binding 已通过 `OpenAIWithBase` workaround 覆盖 |
| Cohere Embedding | EmbeddingModel | Cohere embed-v3 系模型不可用 |
| Google Embedding | EmbeddingModel | Google text-embedding 不可用 |
| Google Image (Imagen) | ImageModel | Google Imagen 不可用 |

### 4.3 Provider 底层能力未被 FFI 暴露

aimux-providers 实际支持超过 170 个 provider, 其中实现非文本模态的包括但不限于:

- **SpeechModel**: ElevenLabs, LMNT, Cartesia, AWS Polly, Hume (共 6 个)
- **ImageModel**: Replicate, Recraft, Prodia, BlackForestLabs, Luma, Stability, Fal (共 7+ 个)
- **TranscriptionModel**: Gladia, ElevenLabs, Deepgram, RevAI, Cartesia, AssemblyAI, Fal (共 7+ 个)
- **RerankingModel**: Cohere, Jina AI, Voyage, Bedrock (共 4 个)
- **VideoModel**: Replicate, Prodia, RunwayML, KlingAI, Fal, Vertex (共 6+ 个)
- **SearchModel**: Tavily, You.com, Google PSE, Exa AI, DataForSEO, Firecrawl, TinyFish, Linkup, Parallel AI, SearXNG, Serper (共 11+ 个)

**aimux-ffi 每种模态仅暴露 1 个 provider 的构造器 (或 0), Node 同样保守 (每种模态 1-3 个)。** 这是有意设计而非遗漏 — 两者都采用相同策略: 暴露主力 provider, 其余通过配置切换。

---

## 5. 其他 C ABI 绑定的多模态覆盖

| 绑定 | 使用 FFI 函数 | 多模态覆盖 | 备注 |
|---|---|---|---|
| **Kotlin** (`bindings/kotlin/.../Model.kt`) | `aimux_openai_new`, `aimux_anthropic_new`, `_with_base` variants, `aimux_generate_text`, `aimux_stream_text`, `aimux_drop_handle`, `aimux_free_string` | ❌ 仅文本 | 实现于 `Model.kt:20-38`, 未使用任何多模态 FFI 函数 |
| **Swift** (`bindings/swift/.../Aimux.swift`) | 同上 Kotlin (`Aimux.swift:59-95`) | ❌ 仅文本 | 通过 `import CAimuxFFI` 调用, module map 从 `.h` 生成 |
| **Flutter** (`bindings/flutter/lib/aimux.dart`) | 同上 (`aimux.dart:50-86`) | ❌ 仅文本 | 通过 `dart:ffi` `DynamicLibrary.lookupFunction` 动态查找 |
| **C** (`bindings/c/example.c`) | `aimux_openai_new`, `aimux_generate_text`, `aimux_stream_text`, `aimux_drop_handle`, `aimux_free_string` | ❌ 仅文本 | 最简单的引用实现 |
| **Go** (`bindings/go/aimux.go` + `multimodal.go`) | **全部 24 个 FFI 函数** | ✅ 完整 8 模态 | **Go 是第一个全量接入多模态的 C ABI 绑定** |

### 5.1 Go 作为第一个全量 C ABI 多模态绑定的证据

- `bindings/go/multimodal.go:1-3` 明确注释: "Multimodal API for the Go binding — 8 modality models mirroring Node's multimodal.rs"
- 覆盖全部 8 种模态: EmbeddingModel, SpeechModel, ImageModel, TranscriptionModel, Files, RerankingModel, VideoModel, SearchModel
- 在 CGo 注释块中手动补充了 6 个未在 `aimux-ffi.h` 中声明的符号 (`bindings/go/multimodal.go:20-25`)
- 为每种模态提供 typed wrapper + JSON parse 函数 (对应 Node 的 `index.ts` 设计模式)

---

## 6. 操作函数覆盖

### 6.1 各模态操作对照

| 模态 | 操作 | Node 方法 (multimodal.rs) | aimux-ffi 函数 (lib.rs) | 缺失 |
|---|---|---|---|---|
| **LanguageModel** | generate (非流式) | `Model::generate_text` (L:42) | `aimux_generate_text` (L:298) | — |
| **LanguageModel** | stream (流式) | `Model::stream_text` (L:63) | `aimux_stream_text` (L:348) | — |
| **EmbeddingModel** | embed | `EmbeddingModel::embed` (L:36) | `aimux_embed` (L:461) | — |
| **SpeechModel** | generate | `SpeechModel::generate` (L:69) | `aimux_speech_generate` (L:508) | — |
| **ImageModel** | generate | `ImageModel::generate` (L:94) | `aimux_image_generate` (L:544) | — |
| **TranscriptionModel** | generate | `TranscriptionModel::generate` (L:120) | `aimux_transcription_generate` (L:580) | — |
| **Files** | upload_file | `Files::upload_file` (L:261) | `aimux_file_upload` (L:625) | — |
| **RerankingModel** | rerank | `RerankingModel::rerank` (L:162) | `aimux_rerank` (L:676) | — |
| **VideoModel** | generate | `VideoModel::generate` (L:202) | `aimux_video_generate` (L:717) | — |
| **SearchModel** | search | `SearchModel::search` (L:227) | `aimux_search` (L:758) | — |
| **Resource** | drop | `drop_handle` (via Drop trait) | `aimux_drop_handle` (L:420) | — |

### 6.2 操作函数完整性

✅ **全部 10 种操作均已实现**, 操作层面无缺失。但存在以下参数合同差异:

| 差异点 | Node | aimux-ffi | 影响 |
|---|---|---|---|
| search 参数风格 | `search(query: string, opts_json?: string)` (L:227) — query 和 opts 分离 | `aimux_search(handle, opts_json)` (L:758) — query 嵌入 opts_json | 参数合同不一致; C 调用者必须在 opts_json 中包含 query 字段 |
| rerank 参数风格 | `rerank(query, docs_json, opts_json?)` (L:162) — 参数分离 | `aimux_rerank(handle, opts_json)` (L:676) — 全部嵌入 opts_json | 同上 |
| transcription opts 合并 | 支持从 opts_json 合并 provider_options (L:131-136) | 不支持 opts_json 参数合并 (L:598-600) — 固定从 audio_base64/media_type 构建 | 无法通过 opts 传递额外 provider 配置 |

---

## 7. wire format 一致性

### 7.1 输入格式

| 层 | prompt 格式 | opts 格式 |
|---|---|---|
| Node `lib.rs` | `parse_prompt`: bare value 或 `{"prompt": ...}` (L:236-247) | `parse_opts`: empty/null → default, 否则反序列化 `GenerateTextOptions` (L:249-260) |
| aimux-ffi `lib.rs` | `parse_prompt`: 完全相同的逻辑 (L:150-159) | `parse_opts`: 完全相同的逻辑 (L:162-168) |

✅ **输入 wire format 完全一致。**

### 7.2 输出格式

| 层 | 成功返回 | 错误返回 |
|---|---|---|
| Node | `serde_json::to_string(&result)` — 直接序列化结果结构体 | `Error::from_reason(format!("{e}"))` — napi Error |
| aimux-ffi | `serde_json::to_string(&r).map(into_cstring_raw)` (L:324) — 直接序列化 | `error_json_raw(...)` → `{"error":"..."}` JSON 字符串 (L:179-181) |

⚠️ **错误处理不一致**: Node 抛出 napi Error (JS 异常), aimux-ffi 返回 `{"error":"..."}` JSON。C ABI 的设计是正确的 (C 没有异常机制), 但各绑定需要对错误 JSON 做二次解析 (Go 的 `extractError()` 函数 `aimux.go:278-289`; Swift 的 `Aimux.swift:120-124`; Flutter 的 `aimux.dart:220-222`)。

### 7.3 流式输出

| 层 | 分块格式 | 错误传递 |
|---|---|---|
| Node | `serde_json::to_string(&part)` → AsyncGenerator yield (multimodal.rs:96-97) | `tx.send(Err(...))` → mpsc channel error |
| aimux-ffi | `serde_json::to_string(&part)` → `on_part(cstr.as_ptr())` (lib.rs:392-396) | `fire_error(on_error, ...)` → `on_error(cstr.as_ptr())` (lib.rs:187-192) |

✅ **流式输出 wire format 一致** (都序列化 StreamPart 为 JSON 字符串)。

---

## 8. 修复建议

### 8.1 阻断级 (必须立即修复)

| # | 问题 | 具体改动 | 影响范围 | 工作量 |
|---|---|---|---|---|
| **B1** | `aimux-ffi.h` 缺少 6 个函数声明 | 在 `aimux-ffi/aimux-ffi.h` 的 Reranking/Video/Search section 补充声明。Go 绑定已有完整声明模板 (`bindings/go/multimodal.go:20-25`)，可直接转录。 | 任何通过 `#include "aimux-ffi.h"` 编译的 C/C++ 消费者; 当前仅影响 C example 和潜在的 C++ 绑定 | 小 (10 分钟) |
| **B2** | header 缺少 Reranking/Video/Search section 注释 | 在 `aimux-ffi.h` 中为已实现但未声明的 section 添加文档注释块 (参照现有的 Embedding/Speech/Image/Transcription/Files 格式) | 文档完整性 | 小 (5 分钟) |

### 8.2 重要级 (应尽快修复)

| # | 问题 | 具体改动 | 影响范围 | 工作量 |
|---|---|---|---|---|
| **I1** | 缺失 4 个构造器: `deepseek`, `cohere_embedding`, `google_embedding`, `google_image` | 在 `aimux-ffi/src/lib.rs` 中新增 4 个构造器函数, 参照现有模式。`deepseek` 可参照 Node `lib.rs:211-230`; `cohere_embedding` 参照 `cohere_reranking_new` (L:662-669); `google_embedding`/`google_image` 参照 `google_video_new` (L:703-710)。需同步更新 `aimux-ffi.h`。 | Kotlin/Swift/Flutter 用户无法通过 FFI 使用 DeepSeek/Cohere Embedding/Google Embedding/Google Imagen | 中 (1-2 小时) |
| **I2** | 全部非文本模态缺少 `_with_base` 变体 (7 个) | 为以下构造器新增 `_with_base` 变体: `aimux_openai_embedding_new_with_base`, `aimux_openai_speech_new_with_base`, `aimux_openai_image_new_with_base`, `aimux_openai_transcription_new_with_base`, `aimux_openai_files_new_with_base`, `aimux_cohere_reranking_new_with_base`, `aimux_google_video_new_with_base`。模式参照 `aimux_openai_new_with_base` (L:219-239): 接收额外 `base_url` 参数, 调 `config.with_base_url()`。 | 测试/本地部署/代理场景下所有非文本模态不可用; 影响 Go binding 的多模态功能完整性 | 中 (1-2 小时) |
| **I3** | `aimux_transcription_generate` 不支持 opts 合并 | 参照 Node `multimodal.rs:126-136`, 在 `aimux-ffi/src/lib.rs:598-607` 增加 opts_json 解析和 provider_options 合并逻辑 | 无法通过 FFI 传递 transcription provider 配置 (如语言选择) | 小 (15 分钟) |
| **I4** | `aimux_file_upload` 不支持 opts 合并解析 | 参照 Node `multimodal.rs:272-278`, 在 `aimux-ffi/src/lib.rs:643-648` 增加 opts_json 对 filename/provier_options 的合并 | 无法通过 FFI 传递文件上传的 filename 等元数据 | 小 (15 分钟) |

### 8.3 建议级 (规划中改进)

| # | 问题 | 具体改动 | 影响范围 | 工作量 |
|---|---|---|---|---|
| **S1** | search/rerank 参数合同与 Node 不一致 | 为 `aimux_search` 增加 `query` 参数, `aimux_rerank` 增加 `query`/`docs_json` 参数, 使签名更接近 Node 风格。或保持现状 (opts_json 包含一切), 在文档中明确标注。 | 跨绑定 API 一致性; 当前 Go binding 已适配 opts_json 风格 (multimodal.go:543-556) | 小-中 |
| **S2** | Provider 多样性: 考虑为热门模态暴露更多 provider | 例如新增 `aimux_elevenlabs_speech_new`, `aimux_stability_image_new`, `aimux_deepgram_transcription_new` 等。参照现有构造器模式。 | 用户选择范围; 需评估维护成本 | 大 |
| **S3** | 缺少 `aimux_cohere_embedding_new_with_base` 和 `aimux_google_embedding_new_with_base` | 在 I1/I2 修复后, 也应为 Cohere/Google 的 embedding 提供 `_with_base` 变体 | 测试/代理场景 | 小 |
| **S4** | 头文件中未声明 Stream callback typedef | 建议在 `aimux-ffi.h` 中添加 `typedef void (*aimux_on_part_fn)(const char*);` 等类型别名, 提升 C API 可用性 | C 绑定开发者体验 | 小 |
| **S5** | 考虑为 `aimux_tavily_search_new` 增加 `_with_base` | Tavily 也应有 base_url 支持 | 测试场景 | 小 |
| **S6** | 补充 `aimux_anthropic_files_new` 构造器 | Anthropic provider 已实现 `files()` 方法 (`aimux-providers/src/anthropic/mod.rs:212`), 但 FFI 未暴露 | 可通过 FFI 使用 Anthropic Files API | 小 |
| **S7** | 补充 `aimux_google_files_new` 构造器 | Google provider 已实现 `files()` 方法 (`aimux-providers/src/google/mod.rs:81`) | 可通过 FFI 使用 Google Files API | 小 |

---

## 9. 总体评价

### 9.1 成熟度评分

| 维度 | 评分 | 说明 |
|---|---|---|
| **文本模态** | ★★★★★ (5/5) | 完整: OpenAI + Anthropic + base_url + 流式 + 非流式 |
| **多模态基础操作** | ★★★★☆ (4/5) | 全部 8 种模态的操作函数均已实现, 但缺失 4 个构造器 |
| **base_url 支持** | ★★☆☆☆ (2/5) | 仅文本模态支持 base_url; 7 个多模态构造器均不支持 |
| **头文件完整性** | ★★★☆☆ (3/5) | 6 个函数 (25%) 未声明, 但功能可正常使用 (linker 可见) |
| **参数合同一致性** | ★★★☆☆ (3/5) | 文本层一致; search/rerank 参数风格有差异; transcription/files 缺少 opts 合并 |
| **错误处理一致性** | ★★★★☆ (4/5) | 错误 JSON envelope 设计合理, 但需各绑定自行解析 |

### 9.2 是否能支撑所有 C ABI 绑定达到 Node 旗舰水准?

**当前状态: 部分可以, 但不完全。**

- **Go binding** 已通过自行补充 6 个头文件声明 + DeepSeek workaround, 实现了完整的多模态覆盖, 最接近 Node 旗舰水准。
- **Kotlin / Swift / Flutter** 目前仅使用文本 API, 它们的多模态需求被未暴露的构造器和缺失的 base_url 支持所阻断。
- **C example** 是最基本的引用实现, 受限于头文件缺失, 无法使用 Reranking/Video/Search。

**核心瓶颈在于 3 点:**

1. **头文件滞后**: 6 个函数未声明 (阻断级, 修复成本极低)
2. **构造器缺失**: 4 个 Node 已有的 factory 在 FFI 无对应 (重要级)
3. **base_url 支持**: 多模态层完全缺失, 阻碍所有测试/本地部署/代理场景 (重要级)

**修复 B1+B2+I1+I2 后, aimux-ffi 即可达到 Node 旗舰水准的功能对等。** 其余建议级改进可逐步推进。

### 9.3 Go binding 的先行意义

Go 是所有 C ABI 绑定中第一个全量接入多模态的, 其在 CGo 注释块中手动补充 6 个缺失声明 (multimodal.go:20-25) 的做法, 实际上已经"标出了"头文件需要补全的所有符号。修复 B1 时可直接参照 Go 的这些声明。

---

*报告结束。审计过程中未修改任何源文件。*
