# Binding API Gaps(绑定 API 差距清单)

> 状态跟踪文档:记录每个语言绑定相对完整能力(见
> [Feature Coverage](../API.md#feature-coverage))缺失的 API,以及每个缺口
> 对应的 C ABI 函数和参考实现。
>
> 判定标准与 [Feature Coverage](../API.md#feature-coverage) 一致:
> ✅ = 源码中有公开构造 + 可调用方法;⚠️ = 类存在但无工厂函数,无法实例化;
> ❌ = 完全没有暴露。

## 差距总览

| 绑定 | 缺失项 | 状态 |
|------|------|------|
| **Swift** | ~~9 个多模态功能全部缺失~~ | ✅ 已完成 |
| **Kotlin** | ~~9 个多模态功能全部缺失~~ | ✅ 已完成 |
| **Flutter** | ~~9 个多模态功能全部缺失~~ | ✅ 已完成 |
| **Go** | ~~多模态构造缺 `base_url` 变体 + embedding/image 仅 OpenAI~~ | ✅ 已完成 |
| **Node.js** | ~~Search 无工厂~~ | ✅ 已完成 |
| **Python** | ~~Search 无工厂~~ | ✅ 已完成 |
| **C ABI** | ~~`cohere_embedding` / `google_embedding` / `google_image` 无 `_with_base`~~ | ✅ 已完成 |
| **Java** | 无（RFC-0013 新绑定，自始实现完整多模态面） | ✅ 已完成 |

> **所有差距已修复。** Feature Coverage 矩阵全绿(见
> [API.md](../API.md#feature-coverage))。

> 注:核心多模态能力本身是完整的——C ABI 已导出全部 36 个函数,Rust core
> 有 10 个 trait。缺口全部在包装层。

---

## 1. Swift(`bindings/swift/Sources/Aimux/`)

现状:[Aimux.swift](../../bindings/swift/Sources/Aimux/Aimux.swift) 只包装语言模型
(4 个构造 + `generateText` / `streamText` / `streamTextAsync` / `generate`),
FFI 调用仅 8 个符号。[Types.swift](../../bindings/swift/Sources/Aimux/Types.swift)
只有文本侧 26 个 Codable 类型。

### 缺失的类(每个 = 构造 + 调用方法 + Codable 类型)

| 缺失类 | 需要的 C ABI 函数 | 参考实现 |
|------|------|------|
| `EmbeddingModel` | `aimux_openai_embedding_new(_with_base)`, `aimux_cohere_embedding_new`, `aimux_google_embedding_new`, `aimux_embed` | Go [multimodal.go](../../bindings/go/multimodal.go#L129) |
| `SpeechModel` | `aimux_openai_speech_new(_with_base)`, `aimux_speech_generate` | Go `multimodal.go#L211` |
| `TranscriptionModel` | `aimux_openai_transcription_new(_with_base)`, `aimux_transcription_generate` | Go `multimodal.go#L306` |
| `ImageModel` | `aimux_openai_image_new(_with_base)`, `aimux_google_image_new`, `aimux_image_generate` | Go `multimodal.go#L259` |
| `VideoModel` | `aimux_google_video_new(_with_base)`, `aimux_video_generate` | Go `multimodal.go#L490` |
| `RerankingModel` | `aimux_cohere_reranking_new(_with_base)`, `aimux_rerank` | Go `multimodal.go#L442` |
| `SearchModel` | `aimux_tavily_search_new(_with_base)`, `aimux_search` | Go `multimodal.go#L537` |
| `Files` | `aimux_openai_files_new(_with_base)`, `aimux_file_upload` | Go `multimodal.go#L374` |

### 缺失的 Codable 类型(Types.swift)

`EmbeddingCallOptions/Result/Usage/Response`, `AudioData`,
`SpeechCallOptions/Request/Response/Result`, `ImageCallOptions/Outputs/Usage/Result`,
`TranscriptionCallOptions/Segment/Result`, `VideoCallOptions/Data/Result`,
`RerankingCallOptions/Rank/Result`, `SearchCallOptions/ResultItem/Result`,
`UploadFileCallOptions/Result` — 字段以
[aimux-core/bindings](../../aimux-core/bindings) 的 ts-rs 声明和
[Go multimodal_types.go](../../bindings/go/multimodal_types.go) 为准。

### 实现提示

- 回调式流媒体已有 trampoline 模式(`StreamContext`),`streamText` 可复用
- 句柄生命周期复用现有 `deinit { aimux_drop_handle }` 模式

---

## 2. Kotlin(`bindings/kotlin/src/main/kotlin/aimux/`)

现状:[Model.kt](../../bindings/kotlin/src/main/kotlin/aimux/Model.kt) 的 JNA 接口
`AimuxFFI` 只声明 8 个 ABI 函数;[TypedModel.kt](../../bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt)
只做文本 typed 包装;[Types.kt](../../bindings/kotlin/src/main/kotlin/aimux/Types.kt)
已有文本侧类型 + `FileBytes`/`FileData`。

### 缺失的工作项

1. **JNA 接口扩展**(Model.kt L20-38):新增 28 个函数声明
   (`aimux_openai_embedding_new` … `aimux_search`),多模态 stream 无回调(都是
   一次性返回),直接映射 `Pointer?` 返回即可
2. **8 个模型类**(同 Swift 清单,构造 + 方法),实现模式参考
   [Go multimodal.go](../../bindings/go/multimodal.go)(同为 C ABI 客户端)
3. **Typed 包装**:多模态 `*CallOptions` / `*Result` 类型 + kotlinx.serialization
   序列化器(参考 TypedModel.kt 的 `AimuxJson` 模式)
4. `AimuxException` 已有,错误路径沿用

---

## 3. Flutter(`bindings/flutter/lib/`)

现状:[aimux.dart](../../bindings/flutter/lib/aimux.dart) 的 dart:ffi `lookupFunction`
只有 8 个 ABI 符号(L76-85);[typed_model.dart](../../bindings/flutter/lib/typed_model.dart)
纯文本;[types.dart](../../bindings/flutter/lib/types.dart) 已有文本侧类型 +
`FileBytes`/`FileData`/`ContentPart`。

### 缺失的工作项

1. **typedef + lookup**(aimux.dart L19-85):新增 28 个 ABI 函数的
   `_XxxC` / `_XxxDart` typedef 与 `lookupFunction`(参考现有 8 个的模式)
2. **8 个模型类**:构造 + 同步方法(多模态调用是阻塞式 C 调用,与现有
   `generateText` 相同,无需 `Stream` 管道)
3. **Typed 包装**(typed_model.dart):多模态 `*CallOptions.toJson` / `*Result.fromJson`
4. 测试:参考 `bindings/flutter/test/`

---

## 4. Go(`bindings/go/`)

现状:[multimodal.go](../../bindings/go/multimodal.go) 已有 8 个多模态类,但:

| 缺口 | 说明 |
|------|------|
| embedding 仅 OpenAI | `NewCohereEmbedding` / `NewGoogleEmbedding` 不存在(FFI 有 `aimux_cohere_embedding_new` / `aimux_google_embedding_new`) |
| image 仅 OpenAI | `NewGoogleImage` 不存在(FFI 有 `aimux_google_image_new`) |
| 多模态无 base_url 变体 | 所有 `NewXxx` 都固定官方 URL;文本模型已有 `OpenAIWithBase` 模式可照搬 |

修法:在 multimodal.go 追加 3 个构造 + 为已有多模态构造加 `WithBase` 变体
(注意 `aimux_cohere_embedding_new` / `aimux_google_embedding_new` /
`aimux_google_image_new` 本身没有 C ABI 的 `_with_base`,需要先补 C ABI,见 §7)。

---

## 5. Node.js(`bindings/node/`)

现状:[index.d.ts](../../bindings/node/index.d.ts) 有 `SearchModel.search`(L65)但
导出列表(L116-152)没有 search 工厂。

修法:napi 层参照
[src/index.ts 的其它工厂](../../bindings/node/src/index.ts) 加
`tavilySearch(apiKey, baseUrl?)` 导出即可(底层 Rust core 已有 `TavilyProvider`)。

## 6. Python(`bindings/python/`)

现状:同 Node——[lib.rs](../../bindings/python/src/lib.rs#L203-L231) 注册了
`SearchModel` 类但 `add_function` 列表里没有 search 工厂;
[multimodal.rs](../../bindings/python/src/multimodal.rs) 是工厂的参考位置。

修法:在 multimodal.rs 加 `tavily_search` 工厂,在 lib.rs 注册。

## 7. C ABI(`aimux-ffi/`)

现状:36 个导出函数,其中 3 个构造没有 `_with_base` 变体:

| 函数 | 有 `_with_base`? |
|------|------|
| `aimux_cohere_embedding_new` | ❌ |
| `aimux_google_embedding_new` | ❌ |
| `aimux_google_image_new` | ❌ |
| `aimux_deepseek_new` | 无(设计如此——DeepSeek 固定官方 URL) |

修法:在 [lib.rs](../../aimux-ffi/src/lib.rs) 补 3 个 `_with_base` 变体(照
`aimux_openai_embedding_new_with_base` 的模式),并在
[c.md](c.md#function-list) 补行。

---

## 建议实施顺序

1. **C ABI + Go + Node/Python 小缺口**(半天):不涉及新架构,纯追加
2. **Swift**(1-2 天):已有 trampoline 和 Codable 模式,单语言最容易闭环
3. **Kotlin**(1-2 天):JNA 直接映射,模式最清晰
4. **Flutter**(1-2 天):typedef 样板多,但阻塞式调用无异步复杂度

每个绑定完成后,更新本文档对应小节和
[Feature Coverage](../API.md#feature-coverage) 矩阵。
