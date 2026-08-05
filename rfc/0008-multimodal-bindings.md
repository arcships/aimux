# RFC-0008: Multimodal API Binding Design

> **Status**: IMPLEMENTED (2026-08-01 — full multimodal surface across all bindings; see [bindings/node/src/multimodal.rs](../bindings/node/src/multimodal.rs))
> **Date**: 2026-07-29
> **Related**: [RFC-0001](0001-multilang-bindings.md) Multilingual Bindings

---

## 1. Background

RFC-0001 completed multilingual bindings for text generation (`generate_text`/`stream_text`). However, aimux-core still has 7 modality traits not exposed to the binding layer:

| Trait | Method | Semantics | Bound |
|-------|--------|-----------|:------:|
| `LanguageModel` | `do_generate`, `do_stream` | Text generation | ✅ |
| `EmbeddingModel` | `do_embed` | Vector embedding | ❌ |
| `SpeechModel` | `do_generate` | Speech synthesis (TTS) | ❌ |
| `TranscriptionModel` | `do_generate`, `do_stream` | Speech-to-text (STT) | ❌ |
| `ImageModel` | `do_generate` | Image generation | ❌ |
| `RerankingModel` | `do_rerank` | Reranking | ❌ |
| `VideoModel` | `do_generate` | Video generation | ❌ |
| `SearchModel` | `do_search` | Search | ❌ |
| `Files` | `upload_file` | File upload | ❌ |

---

## 2. Data Transfer Mode Analysis

The input/output data characteristics of each modality differ greatly, and **not all of them are suitable for JSON string boundaries**:

### 2.1 Pure Text / JSON-friendly (can directly reuse existing JSON boundary)

| Modality | Input | Output | Analysis |
|----------|-------|--------|----------|
| **Embedding** | `Vec<String>` text | `Vec<Vec<f32>>` vectors | Input is text, output is an array of numbers. JSON serialization is naturally applicable. **But vector data volume is large** — 1000 1536-dimensional float32 = 6MB JSON, with performance overhead |
| **Reranking** | query + documents (text) | rank scores | Pure text + numbers, JSON fully applicable |
| **Search** | query string | search results | Pure text + JSON, fully applicable |

### 2.2 Binary Data (requires special handling)

| Modality | Input | Output | Analysis |
|----------|-------|--------|----------|
| **Speech (TTS)** | `text: String` | `AudioData`: base64 string or `Vec<u8>` raw bytes | Output is audio binary. If JSON is used, base64 encoding increases the size by 33%. But what the provider returns is usually already base64, so no additional encoding is needed |
| **Image** | `prompt: String` + optional image input | `ImageOutputs`: base64 string array or `Vec<Vec<u8>>` | Same as TTS, output is image binary. base64 works in JSON but is large |
| **Video** | `prompt: String` + optional image input | `VideoData`: URL / base64 / binary | **Video output is naturally URL-based** (code comment: "Most providers return URLs due to large file sizes"), no need to transfer binary |
| **Files** | `UploadFileData`: bytes or base64 | `provider_reference` (file ID) | **Output is a file ID, not content**. Input is generally KB-level documents |

### 2.3 Streaming Binary (most complex)

| Modality | Input | Output | Analysis |
|----------|-------|--------|----------|
| **Transcription (STT)** | `AudioInput`: bytes or base64 | `TranscriptionResult` (text + timestamps) | Input is audio binary, output is pure text |
| **Transcription Stream** | `Stream<AudioChunk>` audio stream | `Stream<TranscriptionStreamPart>` | Bidirectional stream: input is an audio chunk stream, output is a text chunk stream. **Cannot use JSON boundary** — requires a binary stream channel |

### 2.4 Summary: Three Transfer Modes

```
Mode A: JSON boundary (plain text/structured data)
  Embedding, Reranking, Search
  → Reuse the existing generate_text JSON string boundary

Mode B: JSON + base64 payload (small binary)
  Speech, Image, Transcription (non-streaming)
  → Input/output JSON contains base64 fields
  → Audio/image usually <2MB; base64's 33% overhead is acceptable

Mode B': JSON + URL (large binary, naturally URL)
  Video, Files
  → Video output is naturally URL-centric (provider returns URL, no binary transferred)
  → Files output is a provider file ID (no binary transferred)
  → No special handling needed; JSON boundary naturally applies

Mode C: Bidirectional streaming (binary chunk)
  Transcription Stream
  → Input: audio chunk stream (binary)
  → Output: text chunk stream
  → Requires an independent bidirectional stream channel; cannot reuse the existing JSON boundary
```

---

## 3. Design Plans for Each Path

### 3.1 Native Path (Node / Python)

#### 3.1.1 Mode A: JSON Boundary (Embedding / Reranking / Search)

Directly reuse the `generate_text` pattern — JSON string in, JSON string out:

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

// Factory function
#[napi]
pub async fn openai_embedding(api_key: String, model_id: String, base_url: Option<String>) -> Result<EmbeddingModel> { ... }
```

```typescript
// Node user side
const embedder = await openaiEmbedding('sk-...', 'text-embedding-3-small', baseUrl);
const result = JSON.parse(await embedder.embed(JSON.stringify(['hello', 'world'])));
// result.embeddings = [[...], [...]]
```

#### 3.1.2 Mode B: JSON + base64 Payload (Speech / Image / Video / Files)

Also use the JSON boundary, but binary data goes through base64:

```rust
// Node (napi-rs)
#[napi]
pub struct SpeechModel { inner: Arc<dyn SpeechModelTrait> }

#[napi]
impl SpeechModel {
    #[napi]
    pub async fn generate(&self, opts_json: String) -> Result<String> {
        let opts: SpeechCallOptions = serde_json::from_str(&opts_json)?;
        // opts.text is the text input
        let result = self.inner.do_generate(&opts).await?;
        // result.audio is AudioData::Base64(String) or AudioData::Binary(Vec<u8>)
        // Uniformly serialized to JSON (base64 inside the JSON)
        Ok(serde_json::to_string(&result)?)  // SpeechResult JSON (contains base64 audio)
    }
}
```

```typescript
// Node user side
const speaker = await openaiSpeech('sk-...', 'tts-1', baseUrl);
const result = JSON.parse(await speaker.generate(JSON.stringify({
  text: 'Hello world',
  voice: 'alloy',
  output_format: 'mp3',
})));
// result.audio = { Base64: 'SUQzBAAAA...' } or { Binary: [...] }
// User side decodes base64 into a Buffer and writes the file
fs.writeFileSync('out.mp3', Buffer.from(result.audio.Base64, 'base64'));
```

**Image / Video work the same way**: return JSON containing base64 image/video data.

**Files**: both input and output contain binary:

```typescript
const files = await openaiFiles('sk-...', baseUrl);
const result = JSON.parse(await files.upload(JSON.stringify({
  data: { Data: { data: { Base64: '<base64-file-content>' } } },
  media_type: 'application/pdf',
})));
// result.file_id = 'file-xxx'
```

#### 3.1.3 Mode C: Transcription Stream (bidirectional stream)

Transcription's non-streaming `do_generate` follows Mode B (base64 audio input). Streaming `do_stream` requires a bidirectional binary stream — this is relatively complex in napi-rs and PyO3.

**Option 1 (recommended for PoC)**: Streaming transcription is not supported for now; only `do_generate` (non-streaming) is exposed.

**Option 2 (complete solution)**: Use napi-rs's `AsyncGenerator` + `Buffer` input. Input side: the JS side pushes audio chunks into Rust via a channel; output side: Rust pushes transcript chunks back to JS via a channel. This requires implementing a bidirectional channel on the napi-rs side — high complexity.

```typescript
// Option 2 ideal API (complete solution, not implemented for now)
const transcriber = await openaiTranscription('sk-...', 'whisper-1', baseUrl);
const input = new AudioChunkStream();  // JS side pushes audio chunks
for await (const part of transcriber.stream(input, opts)) {
  if (part.TranscriptDelta) console.log(part.TranscriptDelta.delta);
}
```

**Decision**: The PoC stage only implements `do_generate` (non-streaming transcription); streaming transcription is marked as TODO.

#### 3.1.4 Python Side

The Python side is isomorphic to Node — PyO3's `#[pyfunction]` / `#[pyclass]` + JSON string boundary. The plans for Modes A/B/C are exactly the same, only the syntax differs.

### 3.2 C ABI Path (aimux-ffi / Swift / Kotlin / Flutter)

aimux-ffi needs to add a constructor + operation function for each modality. Reuse the existing handle registry.

#### Mode A (JSON Boundary)

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

#### Mode B (JSON + base64)

```c
// Speech
uint64_t aimux_openai_speech_new(api_key, model_id, base_url);
char* aimux_speech_generate(handle, opts_json);  // → SpeechResult JSON (contains base64 audio)

// Image
uint64_t aimux_openai_image_new(api_key, model_id, base_url);
char* aimux_image_generate(handle, opts_json);  // → ImageResult JSON (contains base64 image)

// Video
uint64_t aimux_prodia_video_new(api_key, model_id, base_url);
char* aimux_video_generate(handle, opts_json);  // → VideoResult JSON

// Files
uint64_t aimux_openai_files_new(api_key, base_url);
char* aimux_file_upload(handle, data_json, media_type);  // → UploadFileResult JSON
```

#### Mode C (Transcription — non-streaming only)

```c
uint64_t aimux_openai_transcription_new(api_key, model_id, base_url);
// audio_base64: base64-encoded audio data
// media_type: "audio/mp3" etc.
char* aimux_transcription_generate(handle, audio_base64, media_type, opts_json);
```

All C ABI functions return `*mut c_char` (JSON string); the caller must release it with `aimux_free_string`. This is consistent with the existing `aimux_generate_text` pattern.

#### Swift / Kotlin / Flutter Side

Each language's wrapper follows the new C symbols of aimux-ffi, with the same pattern: call the C function to get JSON, then parse it into a typed object. Swift/Kotlin use their respective `Data`/`ByteArray` for binary data. Flutter's `dart:ffi` works the same way.

---

## 4. Factory Function Design

### Problem: Factory function explosion

172 providers × 8 modalities = 1376 factory functions in theory. But in practice:
- Most providers implement only 1-2 modalities
- OpenAI-compatible providers share the same set of factory functions

### Plan: Group by provider, name by modality

```typescript
// Node factory function naming convention
// {provider}_{modality}(apiKey, modelId, baseUrl?)

// Text (existing)
openai(apiKey, modelId, baseUrl?)         → Model
anthropic(apiKey, modelId, baseUrl?)      → Model
deepseek(apiKey, modelId, baseUrl?)       → Model

// Embedding
openaiEmbedding(apiKey, modelId, baseUrl?)    → EmbeddingModel
cohereEmbedding(apiKey, modelId, baseUrl?)   → EmbeddingModel
mistralEmbedding(apiKey, modelId, baseUrl?)   → EmbeddingModel

// Speech
openaiSpeech(apiKey, modelId, baseUrl?)       → SpeechModel
elevenlabsSpeech(apiKey, modelId, baseUrl?)   → SpeechModel
cartesiaSpeech(apiKey, modelId, baseUrl?)     → SpeechModel

// Image
openaiImage(apiKey, modelId, baseUrl?)        → ImageModel
// ... etc.
```

**Actual count**: about 50-60 factory functions (each provider only exposes factories for the modalities it supports). The first batch only implements OpenAI's full set of modalities + a few key providers.

### Alternative: Unified Provider Object

```typescript
const provider = createProvider('openai', apiKey, baseUrl);
provider.languageModel('gpt-4o');        // → Model
provider.embeddingModel('text-embedding-3-small');  // → EmbeddingModel
provider.speechModel('tts-1');           // → SpeechModel
```

**Pros**: Factory functions don't explode; a single `createProvider` handles it.
**Cons**: Requires abstracting an "AnyProvider" enum or trait object on the Rust side that can create a model of any modality on demand. The existing Provider trait only has `language_model()`, which needs to be extended.

**Decision**: The PoC uses the factory function plan (simple and direct). If the number of providers grows later and causes a maintenance burden, refactor into a Provider object.

---

## 5. Priority

| Priority | Modality | Reason | Transfer Mode |
|:------:|------|------|---------|
| P0 | Embedding | Almost all providers support it, high usage frequency | A (JSON) |
| P0 | Speech (TTS) | Common in voice scenarios, output is binary | B (JSON+base64) |
| P0 | Transcription (non-streaming) | Pairs with voice scenarios, input is binary | B (JSON+base64) |
| P1 | Image | Image generation is commonly used | B (JSON+base64) |
| P1 | Files | File upload is a prerequisite for other modalities | B (JSON+base64) |
| P2 | Reranking | Specific to search scenarios | A (JSON) |
| P2 | Search | Only 11 providers | A (JSON) |
| P3 | Video | Few providers | B (JSON+base64) |
| P3 | Transcription Stream | Bidirectional stream is complex | C (not implemented for now) |

---

## 6. Implementation Plan

### First Batch (P0: Embedding + Speech + Transcription)

#### Rust Side (aimux-ffi)

New C ABI functions:
```
aimux_openai_embedding_new / aimux_embed
aimux_cohere_embedding_new / aimux_embed  (reuse the same function, different construction)
aimux_mistral_embedding_new / aimux_embed

aimux_openai_speech_new / aimux_speech_generate
aimux_elevenlabs_speech_new / aimux_speech_generate
aimux_cartesia_speech_new / aimux_speech_generate

aimux_openai_transcription_new / aimux_transcription_generate
aimux_deepgram_transcription_new / aimux_transcription_generate
```

Each modality's handle is registered in the existing REGISTRY (needs to be extended to a `HashMap<u64, ModelHandle>` enum).

#### Node Side

New napi classes:
```
EmbeddingModel { embed(values_json, opts?) → Promise<string> }
SpeechModel { generate(opts_json) → Promise<string> }
TranscriptionModel { generate(audio_base64, media_type, opts?) → Promise<string> }
```

New factory functions: `openaiEmbedding`, `openaiSpeech`, `openaiTranscription`, etc.

#### Python Side

Isomorphic: `#[pyclass]` + `#[pyfunction]`.

### Second Batch (P1: Image + Files)

Same as Mode B, identical structure.

### Third Batch (P2-P3: Reranking + Search + Video)

Same as Modes A/B, identical structure.

---

## 7. Performance Considerations for Binary Transfer

### 7.1 base64 Overhead

| Scenario | Original size | base64 size | Overhead | Acceptable? |
|------|---------|------------|------|---------|
| 3-second MP3 speech (TTS output) | ~48KB | ~64KB | +33% | ✅ Acceptable |
| 1024×1024 PNG image (Image output) | ~1.5MB | ~2MB | +33% | ✅ Acceptable |
| 30-minute audio (Transcription input) | ~5MB | ~6.7MB | +33% | ⚠️ Borderline |
| Video (Video output) | — | — | — | ❌ Not needed |

**Video does not need to transfer binary**: `VideoData` is already a three-way choice (`Url` / `Base64` / `Binary`), and the code comment states "Most providers return URLs due to large file sizes". The vast majority of providers return a URL, so the JSON boundary is naturally applicable.

**Files does not need to transfer binary**: `UploadFileResult` returns `provider_reference` (a provider file ID, e.g. `{"openai": "file-xxx"}`), not file content. Although the upload input contains `FileBytes`, the file upload scenario is generally documents/PDFs (KB-level), not videos.

**Binary output of Speech / Image**: Audio/images are usually <2MB, so the 33% overhead of base64 is acceptable. Moreover, providers usually already return base64, so no additional encoding is needed.

**Binary input of Transcription**: If the user transcribes long audio (tens of MB), the base64 overhead is significant. But this is acceptable for the PoC stage; optimize to binary transfer later.

### 7.2 Optimization Directions (future)

If the base64 overhead is unacceptable:
1. **napi-rs `Buffer` type**: napi-rs supports `Uint8Array` for passing binary directly, without base64. It can return a `Buffer` instead of a JSON string.
2. **C ABI binary transfer**: Add `aimux_speech_generate_binary(handle, opts_json, *mut *mut u8, *mut usize)` to the C ABI to return raw bytes + length.
3. **Chunked streaming**: For large files, use a chunked stream instead of one-shot transfer.

**PoC decision**: The first version uses base64 JSON throughout. This is the simplest plan, consistent with the existing JSON boundary. Optimize to binary transfer later as needed.

---

## 8. Type Generation

The options/result types for all modalities have already derived `Serialize/Deserialize/TS` in aimux-core (completed in RFC-0001 stage 0). ts-rs will automatically generate the corresponding `.ts` files:

- `EmbeddingCallOptions.ts`, `EmbeddingResult.ts`
- `SpeechCallOptions.ts`, `SpeechResult.ts`, `AudioData.ts`
- `TranscriptionCallOptions.ts`, `TranscriptionResult.ts`
- `ImageCallOptions.ts`, `ImageResult.ts`
- etc.

The Node binding's `types/` directory already contains these files. Python does not need type files (dynamically typed).

---

## 9. Testing Strategy

### 9.1 Cassette Replay

The existing cassette directory has recordings of non-chat endpoints (1847 skipped cassettes), including:
- `embeddings/` directory (embedding cassettes)
- `images/` directory (image cassettes)
- `audio/transcriptions/` directory (transcription cassettes)

Perform per-cassette replay testing on these cassettes, using the same pattern as the exhaustive test for text generation.

### 9.2 E2E Testing

The Node/Python sides use a local mock server to replay real cassette responses, validating the complete pipeline of each modality.

---

## 10. Open Questions

1. **Factory functions vs Provider object**: The PoC uses factory functions first. Refactor into a Provider object later if the maintenance burden grows?
2. **Transcription streaming**: Should the PoC only do non-streaming? Mark streaming as TODO?
3. **base64 performance**: The first version uses base64 JSON throughout. Is it necessary to immediately implement `Uint8Array` binary transfer for the Node side?
4. **aimux-ffi handle registry**: The current REGISTRY is `HashMap<u64, Arc<dyn LanguageModel>>`. Extend it to an enum (`LanguageModelHandle` / `EmbeddingModelHandle` / ...) or use multiple independent REGISTRYs?

---

## Revision History

| Date | Version | Description |
|------|------|------|
| 2026-07-29 | DRAFT v0.1 | Initial draft; analyzes the data transfer mode of each modality and designs a three-path plan |
