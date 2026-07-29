# RFC-0008：多模态 API 绑定设计

> **状态**：DRAFT（待评审）
> **日期**：2026-07-29
> **关联**：[RFC-0001](0001-multilang-bindings.md) 多语言绑定

---

## 1. 背景

RFC-0001 完成了文本生成（`generate_text`/`stream_text`）的多语言绑定。但 aimux-core 还有 7 个模态 trait 未暴露到绑定层：

| Trait | 方法 | 语义 | 已绑定 |
|-------|------|------|:------:|
| `LanguageModel` | `do_generate`, `do_stream` | 文本生成 | ✅ |
| `EmbeddingModel` | `do_embed` | 向量嵌入 | ❌ |
| `SpeechModel` | `do_generate` | 语音合成(TTS) | ❌ |
| `TranscriptionModel` | `do_generate`, `do_stream` | 语音转文字(STT) | ❌ |
| `ImageModel` | `do_generate` | 图像生成 | ❌ |
| `RerankingModel` | `do_rerank` | 重排序 | ❌ |
| `VideoModel` | `do_generate` | 视频生成 | ❌ |
| `SearchModel` | `do_search` | 搜索 | ❌ |
| `Files` | `upload_file` | 文件上传 | ❌ |

---

## 2. 数据传输方式分析

各模态的输入/输出数据特征差异很大，**不是所有都适合 JSON 字符串边界**：

### 2.1 纯文本 / JSON 友好（可直接复用现有 JSON 边界）

| 模态 | 输入 | 输出 | 分析 |
|------|------|------|------|
| **Embedding** | `Vec<String>` 文本 | `Vec<Vec<f32>>` 向量 | 输入是文本，输出是数字数组。JSON 序列化天然适用。**但向量数据量大**——1000 个 1536 维 float32 = 6MB JSON，有性能开销 |
| **Reranking** | query + documents(文本) | rank scores | 纯文本+数字，JSON 完全适用 |
| **Search** | query 字符串 | search results | 纯文本+JSON，完全适用 |

### 2.2 二进制数据（需要特殊处理）

| 模态 | 输入 | 输出 | 分析 |
|------|------|------|------|
| **Speech (TTS)** | `text: String` | `AudioData`：base64 字符串或 `Vec<u8>` 原始字节 | 输出是音频二进制。如果用 JSON，base64 编码会增加 33% 体积。但 provider 返回的通常已经是 base64，不需要额外编码 |
| **Image** | `prompt: String` + 可选图片输入 | `ImageOutputs`：base64 字符串数组或 `Vec<Vec<u8>>` | 同 TTS，输出是图片二进制。base64 在 JSON 里可行但大 |
| **Video** | `prompt: String` + 可选图片输入 | `VideoData`：URL / base64 / binary | **Video 输出天然以 URL 为主**（代码注释："Most providers return URLs due to large file sizes"），不需要传二进制 |
| **Files** | `UploadFileData`：bytes 或 base64 | `provider_reference`（文件 ID） | **输出是文件 ID 不是内容**。输入一般 KB 级文档 |

### 2.3 流式二进制（最复杂）

| 模态 | 输入 | 输出 | 分析 |
|------|------|------|------|
| **Transcription (STT)** | `AudioInput`：bytes 或 base64 | `TranscriptionResult`（文本+时间戳） | 输入是音频二进制，输出是纯文本 |
| **Transcription Stream** | `Stream<AudioChunk>` 音频流 | `Stream<TranscriptionStreamPart>` | 双向流：输入是音频 chunk 流，输出是文本 chunk 流。**无法用 JSON 边界**——需要二进制流通道 |

### 2.4 总结：三种传输模式

```
模式 A: JSON 边界（纯文本/结构化数据）
  Embedding, Reranking, Search
  → 复用现有 generate_text 的 JSON 字符串边界

模式 B: JSON + base64 载荷（小体积二进制）
  Speech, Image, Transcription (非流式)
  → 输入/输出 JSON 含 base64 字段
  → 音频/图片通常 <2MB，base64 的 33% 开销可接受

模式 B': JSON + URL（大体积二进制，天然 URL）
  Video, Files
  → Video 输出天然以 URL 为主（provider 返回 URL，不传二进制）
  → Files 输出是 provider 文件 ID（不传二进制）
  → 无需特殊处理，JSON 边界天然适用

模式 C: 双向流式（二进制 chunk）
  Transcription Stream
  → 输入：音频 chunk 流（二进制）
  → 输出：文本 chunk 流
  → 需要独立的双向流通道，不能复用现有 JSON 边界
```

---

## 3. 各路径设计方案

### 3.1 原生路径（Node / Python）

#### 3.1.1 模式 A：JSON 边界（Embedding / Reranking / Search）

直接复用 `generate_text` 的模式——JSON 字符串进、JSON 字符串出：

```rust
// Node (napi-rs)
#[napi]
pub struct EmbeddingModel { inner: Arc<dyn EmbeddingModelTrait> }

#[napi]
impl EmbeddingModel {
    #[napi]
    pub async fn embed(&self, values_json: String, opts_json: Option<String>) -> Result<String> {
        let values: Vec<String> = serde_json::from_str(&values_json)?;
        let opts: EmbeddingCallOptions = parse_opts(opts_json)?;
        let result = self.inner.do_embed(&opts).await?;
        Ok(serde_json::to_string(&result)?)  // EmbeddingResult JSON
    }
}

// 工厂函数
#[napi]
pub async fn openai_embedding(api_key: String, model_id: String, base_url: Option<String>) -> Result<EmbeddingModel> { ... }
```

```typescript
// Node 用户侧
const embedder = await openaiEmbedding('sk-...', 'text-embedding-3-small', baseUrl);
const result = JSON.parse(await embedder.embed(JSON.stringify(['hello', 'world'])));
// result.embeddings = [[...], [...]]
```

#### 3.1.2 模式 B：JSON + base64 载荷（Speech / Image / Video / Files）

同样用 JSON 边界，但二进制数据走 base64：

```rust
// Node (napi-rs)
#[napi]
pub struct SpeechModel { inner: Arc<dyn SpeechModelTrait> }

#[napi]
impl SpeechModel {
    #[napi]
    pub async fn generate(&self, opts_json: String) -> Result<String> {
        let opts: SpeechCallOptions = serde_json::from_str(&opts_json)?;
        // opts.text 是文本输入
        let result = self.inner.do_generate(&opts).await?;
        // result.audio 是 AudioData::Base64(String) 或 AudioData::Binary(Vec<u8>)
        // 统一序列化为 JSON（base64 在 JSON 里）
        Ok(serde_json::to_string(&result)?)  // SpeechResult JSON（含 base64 音频）
    }
}
```

```typescript
// Node 用户侧
const speaker = await openaiSpeech('sk-...', 'tts-1', baseUrl);
const result = JSON.parse(await speaker.generate(JSON.stringify({
  text: 'Hello world',
  voice: 'alloy',
  output_format: 'mp3',
})));
// result.audio = { Base64: 'SUQzBAAAA...' } 或 { Binary: [...] }
// 用户侧把 base64 解码成 Buffer 写文件
fs.writeFileSync('out.mp3', Buffer.from(result.audio.Base64, 'base64'));
```

**Image / Video 同理**：返回 JSON 含 base64 图片/视频数据。

**Files**：输入和输出都含二进制：

```typescript
const files = await openaiFiles('sk-...', baseUrl);
const result = JSON.parse(await files.upload(JSON.stringify({
  data: { Data: { data: { Base64: '<base64-file-content>' } } },
  media_type: 'application/pdf',
})));
// result.file_id = 'file-xxx'
```

#### 3.1.3 模式 C：Transcription Stream（双向流）

Transcription 的非流式 `do_generate` 走模式 B（base64 音频输入）。流式 `do_stream` 需要双向二进制流——这在 napi-rs 和 PyO3 中比较复杂。

**方案 1（推荐 PoC）**：流式转录暂不支持，只暴露 `do_generate`（非流式）。

**方案 2（完整方案）**：用 napi-rs 的 `AsyncGenerator` + `Buffer` 输入。输入侧：JS 端把音频 chunk 通过 channel 推入 Rust；输出侧：Rust 把 transcript chunk 通过 channel 推回 JS。这需要在 napi-rs 侧实现双向 channel——复杂度高。

```typescript
// 方案 2 的理想 API（完整方案，暂不实现）
const transcriber = await openaiTranscription('sk-...', 'whisper-1', baseUrl);
const input = new AudioChunkStream();  // JS 端 push 音频 chunk
for await (const part of transcriber.stream(input, opts)) {
  if (part.TranscriptDelta) console.log(part.TranscriptDelta.delta);
}
```

**决策**：PoC 阶段只做 `do_generate`（非流式转录），流式转录标记为 TODO。

#### 3.1.4 Python 侧

Python 侧与 Node 同构——PyO3 的 `#[pyfunction]` / `#[pyclass]` + JSON 字符串边界。模式 A/B/C 的方案完全相同，只是语法不同。

### 3.2 C ABI 路径（aimux-ffi / Swift / Kotlin / Flutter）

aimux-ffi 需要为每个模态加构造函数 + 操作函数。复用现有 handle 注册表。

#### 模式 A（JSON 边界）

```c
// Embedding
uint64_t aimux_openai_embedding_new(api_key, model_id, base_url);
char* aimux_embed(handle, values_json, opts_json);  // → EmbeddingResult JSON

// Reranking
uint64_t aimux_cohere_reranking_new(api_key, model_id, base_url);
char* aimux_rerank(handle, query, docs_json, opts_json);  // → RerankingResult JSON

// Search
uint64_t aimux_tavily_search_new(api_key, model_id, base_url);
char* aimux_search(handle, query, opts_json);  // → SearchResult JSON
```

#### 模式 B（JSON + base64）

```c
// Speech
uint64_t aimux_openai_speech_new(api_key, model_id, base_url);
char* aimux_speech_generate(handle, opts_json);  // → SpeechResult JSON（含 base64 音频）

// Image
uint64_t aimux_openai_image_new(api_key, model_id, base_url);
char* aimux_image_generate(handle, opts_json);  // → ImageResult JSON（含 base64 图片）

// Video
uint64_t aimux_prodia_video_new(api_key, model_id, base_url);
char* aimux_video_generate(handle, opts_json);  // → VideoResult JSON

// Files
uint64_t aimux_openai_files_new(api_key, base_url);
char* aimux_file_upload(handle, data_json, media_type);  // → UploadFileResult JSON
```

#### 模式 C（Transcription — 非流式 only）

```c
uint64_t aimux_openai_transcription_new(api_key, model_id, base_url);
// audio_base64: base64 编码的音频数据
// media_type: "audio/mp3" 等
char* aimux_transcription_generate(handle, audio_base64, media_type, opts_json);
```

所有 C ABI 函数返回 `*mut c_char`（JSON 字符串），调用方需 `aimux_free_string` 释放。与现有 `aimux_generate_text` 模式一致。

#### Swift / Kotlin / Flutter 侧

各语言 wrapper 跟随 aimux-ffi 的新 C 符号，模式相同：调 C 函数拿 JSON，parse 成 typed 对象。Swift/Kotlin 的二进制数据用各自语言的 `Data`/`ByteArray`。Flutter 的 `dart:ffi` 同理。

---

## 4. 工厂函数设计

### 问题：工厂函数爆炸

172 个 provider × 8 模态 = 理论上 1376 个工厂函数。但实际：
- 大部分 provider 只实现 1-2 个模态
- OpenAI 兼容 provider 共享同一套工厂函数

### 方案：按 provider 分组，按模态命名

```typescript
// Node 侧工厂函数命名规范
// {provider}_{modality}(apiKey, modelId, baseUrl?)

// 文本（已有）
openai(apiKey, modelId, baseUrl?)         → Model
anthropic(apiKey, modelId, baseUrl?)      → Model
deepseek(apiKey, modelId, baseUrl?)       → Model

// 嵌入
openaiEmbedding(apiKey, modelId, baseUrl?)    → EmbeddingModel
cohereEmbedding(apiKey, modelId, baseUrl?)   → EmbeddingModel
mistralEmbedding(apiKey, modelId, baseUrl?)   → EmbeddingModel

// 语音
openaiSpeech(apiKey, modelId, baseUrl?)       → SpeechModel
elevenlabsSpeech(apiKey, modelId, baseUrl?)   → SpeechModel
cartesiaSpeech(apiKey, modelId, baseUrl?)     → SpeechModel

// 图像
openaiImage(apiKey, modelId, baseUrl?)        → ImageModel
// ... 等等
```

**实际数量**：约 50-60 个工厂函数（每个 provider 只为其支持的模态暴露工厂）。第一批只做 OpenAI 的全套模态 + 少数关键 provider。

### 替代方案：统一 Provider 对象

```typescript
const provider = createProvider('openai', apiKey, baseUrl);
provider.languageModel('gpt-4o');        // → Model
provider.embeddingModel('text-embedding-3-small');  // → EmbeddingModel
provider.speechModel('tts-1');           // → SpeechModel
```

**优点**：工厂函数不爆炸，一个 `createProvider` 搞定。
**缺点**：需要在 Rust 侧抽象一个 "AnyProvider" enum 或 trait object，能按需创建任何模态的 model。现有 Provider trait 只有 `language_model()`，需要扩展。

**决策**：PoC 用工厂函数方案（简单直接）。如果后续 provider 数量增长导致维护负担，再重构为 Provider 对象。

---

## 5. 优先级

| 优先级 | 模态 | 原因 | 传输模式 |
|:------:|------|------|---------|
| P0 | Embedding | 几乎所有 provider 都支持，使用频率高 | A（JSON） |
| P0 | Speech (TTS) | 语音场景常见，输出是二进制 | B（JSON+base64） |
| P0 | Transcription (非流式) | 语音场景配套，输入是二进制 | B（JSON+base64） |
| P1 | Image | 图像生成常用 | B（JSON+base64） |
| P1 | Files | 文件上传是其他模态的前置 | B（JSON+base64） |
| P2 | Reranking | 搜索场景专用 | A（JSON） |
| P2 | Search | 只有 11 个 provider | A（JSON） |
| P3 | Video | 少数 provider | B（JSON+base64） |
| P3 | Transcription Stream | 双向流复杂 | C（暂不实现） |

---

## 6. 实现计划

### 第一批（P0：Embedding + Speech + Transcription）

#### Rust 侧（aimux-ffi）

新增 C ABI 函数：
```
aimux_openai_embedding_new / aimux_embed
aimux_cohere_embedding_new / aimux_embed  (复用同一函数，不同构造)
aimux_mistral_embedding_new / aimux_embed

aimux_openai_speech_new / aimux_speech_generate
aimux_elevenlabs_speech_new / aimux_speech_generate
aimux_cartesia_speech_new / aimux_speech_generate

aimux_openai_transcription_new / aimux_transcription_generate
aimux_deepgram_transcription_new / aimux_transcription_generate
```

每个模态的 handle 注册到现有 REGISTRY（需扩展为 `HashMap<u64, ModelHandle>` enum）。

#### Node 侧

新增 napi class：
```
EmbeddingModel { embed(values_json, opts?) → Promise<string> }
SpeechModel { generate(opts_json) → Promise<string> }
TranscriptionModel { generate(audio_base64, media_type, opts?) → Promise<string> }
```

新增工厂函数：`openaiEmbedding`, `openaiSpeech`, `openaiTranscription` 等。

#### Python 侧

同构：`#[pyclass]` + `#[pyfunction]`。

### 第二批（P1：Image + Files）

同模式 B，结构相同。

### 第三批（P2-P3：Reranking + Search + Video）

同模式 A/B，结构相同。

---

## 7. 二进制传输的性能考量

### 7.1 base64 开销

| 场景 | 原始大小 | base64 大小 | 开销 | 可接受？ |
|------|---------|------------|------|---------|
| 3 秒 MP3 语音 (TTS 输出) | ~48KB | ~64KB | +33% | ✅ 可接受 |
| 1024×1024 PNG 图片 (Image 输出) | ~1.5MB | ~2MB | +33% | ✅ 可接受 |
| 30 分钟音频 (Transcription 输入) | ~5MB | ~6.7MB | +33% | ⚠️ 边界 |
| 视频 (Video 输出) | — | — | — | ❌ 不需要 |

**Video 不需要传二进制**：`VideoData` 已经是三选一（`Url` / `Base64` / `Binary`），代码注释写明 "Most providers return URLs due to large file sizes"。绝大多数 provider 返回 URL，JSON 边界天然适用。

**Files 不需要传二进制**：`UploadFileResult` 返回的是 `provider_reference`（provider 文件 ID，如 `{"openai": "file-xxx"}`），不是文件内容。上传输入虽然含 `FileBytes`，但文件上传场景一般是文档/PDF（KB 级），不是视频。

**Speech / Image 的二进制输出**：音频/图片通常 <2MB，base64 的 33% 开销可接受。且 provider 通常已返回 base64，无需额外编码。

**Transcription 的二进制输入**：如果用户转录长音频（几十 MB），base64 开销较大。但 PoC 阶段可接受，后续优化为二进制传输。

### 7.2 优化方向（未来）

如果 base64 开销不可接受：
1. **napi-rs `Buffer` 类型**：napi-rs 支持 `Uint8Array` 直接传二进制，不需要 base64。可以返回 `Buffer` 而非 JSON 字符串。
2. **C ABI 二进制传递**：C ABI 加 `aimux_speech_generate_binary(handle, opts_json, *mut *mut u8, *mut usize)` 返回原始字节 + 长度。
3. **分块流式**：对于大文件，用 chunked 流而非一次性传输。

**PoC 决策**：第一版全部用 base64 JSON。这是最简单的方案，与现有 JSON 边界一致。后续按需优化为二进制传输。

---

## 8. 类型生成

所有模态的 options/result 类型已经在 aimux-core 里派生了 `Serialize/Deserialize/TS`（RFC-0001 阶段 0 完成）。ts-rs 会自动生成对应的 `.ts` 文件：

- `EmbeddingCallOptions.ts`, `EmbeddingResult.ts`
- `SpeechCallOptions.ts`, `SpeechResult.ts`, `AudioData.ts`
- `TranscriptionCallOptions.ts`, `TranscriptionResult.ts`
- `ImageCallOptions.ts`, `ImageResult.ts`
- 等等

Node 绑定的 `types/` 目录已包含这些文件。Python 不需要类型文件（动态类型）。

---

## 9. 测试策略

### 9.1 Cassette 回放

现有 cassette 目录有非 chat 端点的录像（1847 个被跳过的 cassette），包括：
- `embeddings/` 目录（embedding cassette）
- `images/` 目录（image cassette）
- `audio/transcriptions/` 目录（transcription cassette）

对这些 cassette 做逐个回放测试，与文本生成的 exhaustive test 相同模式。

### 9.2 E2E 测试

Node/Python 侧用本地 mock server 回放真实 cassette 响应，验证各模态的完整链路。

---

## 10. 待决策问题

1. **工厂函数 vs Provider 对象**：PoC 先用工厂函数。如果维护负担大再重构为 Provider 对象？
2. **Transcription 流式**：PoC 是否只做非流式？流式作为 TODO 标记？
3. **base64 性能**：第一版全用 base64 JSON。是否需要立即为 Node 侧实现 `Uint8Array` 二进制传输？
4. **aimux-ffi handle 注册表**：当前 REGISTRY 是 `HashMap<u64, Arc<dyn LanguageModel>>`。扩展为 enum（`LanguageModelHandle` / `EmbeddingModelHandle` / ...）还是用多个独立 REGISTRY？

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | DRAFT v0.1 | 初稿，分析各模态数据传输方式，设计三路径方案 |
