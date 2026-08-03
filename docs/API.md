# aimux API Documentation

> Unified LLM service access layer — one API to access 325 AI providers

## Table of Contents

- [Language Guides](#language-guides)
- [Quick Start](#quick-start)
- [Features](#features)
  - [Text Generation](#text-generation)
  - [Streaming Generation](#streaming-generation)
  - [Tool Calling](#tool-calling)
  - [Multi-Role Messages](#multi-role-messages)
  - [Vector Embedding](#vector-embedding)
  - [Speech Synthesis (TTS)](#speech-synthesis-tts)
  - [Speech to Text (STT)](#speech-to-text-stt)
  - [Image Generation](#image-generation)
  - [Video Generation](#video-generation)
  - [Reranking](#reranking)
  - [Search](#search)
  - [File Upload](#file-upload)
- [Provider Factory Functions](#provider-factory-functions)
- [Feature Coverage](#feature-coverage)
- [Construction and base_url Support](#construction-and-base_url-support)
- [Design Documents](#design-documents)
- [License](#license)

## Language Guides

Each language has its own guide with that language's examples:

| Language | Guide | Coverage |
|------|------|------|
| Node.js | [api/node.md](api/node.md) | Full multimodal surface (native path) |
| Python | [api/python.md](api/python.md) | Full multimodal surface (native path) |
| Rust | [api/rust.md](api/rust.md) | Full multimodal surface (core) |
| Go | [api/go.md](api/go.md) | Full multimodal surface (C ABI path, typed wrappers) |
| C/C++ | [api/c.md](api/c.md) | Full multimodal surface (C ABI) |
| Swift | [api/swift.md](api/swift.md) | Full multimodal surface (C ABI path) |
| Kotlin | [api/kotlin.md](api/kotlin.md) | Full multimodal surface (C ABI path) |
| Flutter/Dart | [api/flutter.md](api/flutter.md) | Full multimodal surface (C ABI path) |
| Java | [api/java.md](api/java.md) | Full multimodal surface (C ABI path) |

## Quick Start

All bindings share the same API shape — only the syntax differs. Pick your
language guide and follow its Quick Start:

[Node.js](api/node.md#quick-start) · [Python](api/python.md#quick-start) ·
[Rust](api/rust.md#quick-start) · [Go](api/go.md#quick-start) ·
[C/C++](api/c.md#quick-start) · [Swift](api/swift.md#quick-start) ·
[Kotlin](api/kotlin.md#quick-start) · [Flutter/Dart](api/flutter.md#quick-start)

## Providers

### Native protocols

OpenAI, Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI,
Anthropic-AWS — one constructor per provider: `openai(apiKey, model, baseUrl?)`
/ `anthropic(apiKey, model, baseUrl?)` (Node, Python), `NewOpenAI(apiKey,
model)` (Go), `Model.openai(apiKey, modelId)` (Java, Kotlin, Flutter),
`Aimux.openai(apiKey:modelId:)` (Swift), `OpenAIProvider::new(..)` (Rust).
Multimodal, local-inference and search providers have their own constructors
too — full list: [reference.md](api/reference.md).

### OpenAI-compatible (250)

One function in every binding:

```text
provider(name, api_key?, model_id, config?)   // all languages
  name     — provider name; 推荐使用类型化 ProviderName（见下），字符串同样可用
  api_key  — optional; omitted/None reads the provider's env var
  config   — optional overrides (base_url / headers / maxRetries / body_overrides)
```

- **ProviderName** (Rust enum, TS const object, Go/Java/Kotlin consts, Swift
  enum, Dart consts) — IDE-completable and typo-proof:

  ```text
  Rust:   provider(ProviderName::Groq, ...)        TS:     provider(ProviderName.groq, ...)
  Go:     Provider(ProviderName.Groq, ...)         Java:   Model.provider(ProviderName.GROQ, ...)
  Swift:  Aimux.provider(name: ProviderName.groq.rawValue, ...)
  Dart:   Model.provider(ProviderName.groq, ...)
  ```

  字符串形式（`provider("groq", ...)`）在全部语言中同样可用——两种写法等价。
- Full list (250, name / env var / base URL): [providers.md](api/providers.md)
- Custom endpoint: registry name + `base_url` override, or the OpenAI
  constructor with a base URL

## Features

### Text Generation

Non-streaming text generation; returns the complete result.

Examples: [Node.js](api/node.md#text-generation) · [Python](api/python.md#text-generation) · [Rust](api/rust.md#text-generation) · [Go](api/go.md#text-generation) · [Swift](api/swift.md#text-generation) · [Kotlin](api/kotlin.md#text-generation) · [Flutter](api/flutter.md#text-generation) · [C ABI](api/c.md#language-model)

#### Parameters

| Parameter | Type | Description |
|------|------|------|
| `prompt` | `string` / `Message[]` | Prompt or message array |
| `max_output_tokens` | `number?` | Maximum number of generated tokens |
| `temperature` | `number?` | Sampling temperature |
| `top_p` | `number?` | Nucleus sampling |
| `stop_sequences` | `string[]?` | Stop sequences |
| `tools` | `Tool[]?` | List of available tools |
| `tool_choice` | `ToolChoice?` | Tool selection strategy |
| `instructions` | `string?` | System instructions |
| `reasoning` | `ReasoningEffort?` | Reasoning effort |
| `max_retries` | `number?` | Per-call retry override; `0` disables retries (`None` = provider default, 2) |
| `timeout` | `TimeoutConfiguration?` | Per-call timeouts (total / first-chunk / chunk idle) — see [Timeouts](#timeouts) |
| `body_overrides` | `object?` | Per-call request-body overrides, deep-merged; `null` values delete keys |
| `headers` | `object?` | Extra HTTP headers |

Node.js additionally accepts an `AbortSignal` as the 4th argument of
`generateText` / `streamText` (see [Request Cancellation](#request-cancellation)).

#### Return Value

The result is a structured object with these fields (the exact type
declaration differs per language — see each [language guide](#language-guides)
for its own declaration):

| Field | Description |
|------|------|
| `text` | Generated text (all Text variants concatenated) |
| `tool_calls` | Tool call list (extracted from content) |
| `finish_reason` | Finish reason |
| `usage` | Token usage |
| `warnings` | Warnings |
| `raw` | Raw provider result (includes full content) |

> **Note**: `text` and `tool_calls` are convenience fields extracted from `raw.content`.
> The `Source`, `Reasoning`, and `ToolResult` variants do not appear in the convenience fields — access them via `raw.content`.

> **Type declarations are per-language.** Every binding declares these types in
> its own syntax: TypeScript `interface`/`type` in [node.md](api/node.md#types),
> Python pydantic models in [python.md](api/python.md#types), Rust `struct`/`enum`
> in [rust.md](api/rust.md#types), Kotlin `data class` in
> [kotlin.md](api/kotlin.md#types), Dart classes in [flutter.md](api/flutter.md#types),
> Swift `struct` in [swift.md](api/swift.md#types), Go `struct` in
> [go.md](api/go.md#types). The tables below describe the shared **JSON shape**
> that crosses the binding boundary — the field names and variant tags are
> identical in every language.

#### Structured content (`raw.content`)

`raw.content` is a `GenerateContent` array containing 6 variants:

| Variant | Fields | Description |
|------|------|------|
| `Text` | `text` | Generated text |
| `ToolCall` | `tool_call_id`, `tool_name`, `input`, `provider_executed?`, `dynamic?`, `provider_metadata?` | Tool call requested by the model |
| `Source` | `id`, `source_type`, `url?`, `title?` | Reference/source |
| `Reasoning` | `text`, `provider_metadata?` | Reasoning/thinking segment |
| `File` | `data: FileData`, `media_type`, `filename?`, `provider_metadata?` | File generated by the model |
| `ToolResult` | `tool_call_id`, `tool_name`, `result`, `is_error?`, `preliminary?`, `dynamic?`, `provider_metadata?` | Tool result executed by the provider |

### Streaming Generation

Returns generated content as a stream, output chunk by chunk.

Examples: [Node.js](api/node.md#streaming-generation) · [Python](api/python.md#streaming-generation) · [Rust](api/rust.md#streaming-generation) · [Go](api/go.md#streaming-generation) · [Swift](api/swift.md#streaming-generation) · [Kotlin](api/kotlin.md#streaming-generation) · [Flutter](api/flutter.md#streaming-generation) · [C ABI](api/c.md#language-model)

#### StreamPart Types

| Variant | Description |
|------|------|
| `StreamStart` | Stream start (carries warnings) |
| `TextStart` / `TextDelta` / `TextEnd` | Text segment lifecycle |
| `ToolInputStart` / `ToolInputDelta` / `ToolInputEnd` | Tool calling input stream |
| `ToolCall` | Complete tool call |
| `ToolResult` | Tool result executed by the provider |
| `ReasoningStart` / `ReasoningDelta` / `ReasoningEnd` | Reasoning segment lifecycle |
| `ResponseMetadata` | Response metadata (id, timestamp, model_id) |
| `Source` | Reference/source |
| `Finish` | Stream end (carries usage + finish_reason) |
| `Error` | Stream error |
| `Raw` | Provider raw chunk (for debugging, when `include_raw_chunks` is set) |

### Request Cancellation (abort)

Calls can be cancelled mid-flight. Cancellation covers the whole request
lifecycle — connect, response headers, non-streaming body reads, and the
streaming body (including while waiting between chunks and during retry
backoff). An abort is reported as `AiMuxError::Aborted` (not retryable; a
pre-aborted signal fails fast without sending).

- **Node.js** — pass an `AbortSignal` as the last argument; the typed wrapper
  bridges it internally:

  ```ts
  import { openai, generateText, streamText } from 'aimux'

  const controller = new AbortController()
  const model = await openai('sk-...', 'gpt-4o')

  const result = await generateText(model, 'Explain Rust.', {}, controller.signal)

  const gen = streamText(model, 'Write a haiku.', {}, controller.signal)
  controller.abort() // cancels the stream promptly
  ```

  Under the hood the wrapper constructs an `AbortBridge` (also exported, for
  the raw napi surface and multimodal calls). `AbortBridge` is one-shot: it
  shares the signal's cancellation state, so aborting once aborts every call
  that uses the same bridge, and reusing an aborted bridge fails fast.

- **Rust** — set `abort_signal` directly on the options (a runtime handle, it
  never crosses the JSON boundary):

  ```rust
  let signal = aimux_core::shared::AbortSignal::new();
  let opts = GenerateTextOptions { abort_signal: Some(signal.clone()), ..Default::default() };
  let task = tokio::spawn(generate_text(&model, "Explain Rust.", opts));
  signal.abort(); // cancels the call
  ```

- **FFI / Python / Go / C ABI** — the JSON boundary cannot carry runtime
  handles; cancellation is not yet exposed there (tracked in RFC-0016 §7.3).

### Timeouts

Per-call timeout limits, JSON-serializable in every binding:

| Field | Type | Description |
|------|------|------|
| `total_ms` | `number?` | Overall deadline for the whole call — includes retries and, for streaming, the entire stream. `0` fails immediately |
| `first_chunk_ms` | `number?` | Streaming only: time allowed from request start until the first chunk |
| `chunk_ms` | `number?` | Streaming only: max idle time **between** chunks (sliding window, reset on every chunk) |

`None`/absent disables the corresponding limit. On expiry the call fails with
`AiMuxError::Timeout` (not retryable); streaming timeouts surface as a
`StreamPart::Error` item (`"first chunk timeout"` / `"chunk idle timeout"` /
`"total timeout"`). Unrepresentable values (e.g. `u64::MAX` ms on narrower
platforms) are rejected with `AiMuxError::InvalidArgument` instead of
panicking. When both abort and a deadline are in play, abort wins.

```ts
// Node.js — timeouts ride inside the options object
await generateText(model, 'Explain Rust.', {
  timeout: { total_ms: 30_000, first_chunk_ms: 5_000, chunk_ms: 2_000 },
})
```

```rust
// Rust
let opts = GenerateTextOptions {
    timeout: Some(aimux_core::options::TimeoutConfiguration {
        total_ms: Some(30_000),
        first_chunk_ms: Some(5_000),
        chunk_ms: Some(2_000),
    }),
    ..Default::default()
};
```

> Vercel's `stepMs`/`toolMs` are intentionally absent — they serve the
> multi-step tool loop (H4), which aimux does not implement (RFC-0016 §7.5).

### Tool Calling

Tool definitions are language-agnostic data descriptions (JSON Schema) that require no macros.

Examples: [Node.js](api/node.md#tool-calling) · [Python](api/python.md#tool-calling) · [Rust](api/rust.md#tool-calling)

### Multi-Role Messages

`prompt` accepts a message array to implement multi-turn conversation; roles support `system` / `user` / `assistant` / `tool`.

Examples: [Node.js](api/node.md#multi-role-messages) · [Python](api/python.md#multi-role-messages) · [Rust](api/rust.md#multi-role-messages)

### Vector Embedding

Converts text into a vector representation.

Examples: [Node.js](api/node.md#vector-embedding) · [Python](api/python.md#vector-embedding) · [Rust](api/rust.md#vector-embedding) · [Go](api/go.md#vector-embedding) · [C ABI](api/c.md#vector-embedding)

#### Supported Providers

| Factory function | Provider | Representative model |
|---------|---------|---------|
| `openaiEmbedding` | OpenAI | text-embedding-3-small/large |
| `cohereEmbedding` | Cohere | embed-english-v3.0 |
| `googleEmbedding` | Google | gemini-embedding-001 |

### Speech Synthesis (TTS)

Converts text into speech audio.

Examples: [Node.js](api/node.md#speech-synthesis-tts) · [Python](api/python.md#speech-synthesis-tts) · [Rust](api/rust.md#speech-synthesis-tts) · [Go](api/go.md#speech-synthesis-tts) · [C ABI](api/c.md#speech)

#### Supported Providers

| Factory function | Provider | Representative model |
|---------|---------|---------|
| `openaiSpeech` | OpenAI | tts-1, tts-1-hd |

### Speech to Text (STT)

Converts audio into text (non-streaming).

Examples: [Node.js](api/node.md#speech-to-text-stt) · [Python](api/python.md#speech-to-text-stt) · [Rust](api/rust.md#speech-to-text-stt) · [Go](api/go.md#speech-to-text-stt) · [C ABI](api/c.md#speech)

### Image Generation

Examples: [Node.js](api/node.md#image-generation) · [Python](api/python.md#image-generation) · [Rust](api/rust.md#image-generation) · [Go](api/go.md#image-generation) · [C ABI](api/c.md#image)

#### Supported Providers

| Factory function | Provider | Representative model |
|---------|---------|---------|
| `openaiImage` | OpenAI | dall-e-3 |
| `googleImage` | Google | gemini-2.5-flash-image |

### Video Generation

Video generation typically returns a URL (not binary).

Examples: [Node.js](api/node.md#video-generation) · [Python](api/python.md#video-generation) · [Rust](api/rust.md#video-generation) · [Go](api/go.md#video-generation) · [C ABI](api/c.md#video-generation)

### Reranking

Reorders a document list by relevance.

Examples: [Node.js](api/node.md#reranking) · [Python](api/python.md#reranking) · [Rust](api/rust.md#reranking) · [Go](api/go.md#reranking) · [C ABI](api/c.md#reranking)

### Search

Calls a search provider to obtain results.

Examples: [Node.js](api/node.md#search) · [Python](api/python.md#search) · [Rust](api/rust.md#search) · [Go](api/go.md#search) · [C ABI](api/c.md#search)

> ⚠️ The `SearchModel` class is exported in Node.js / Python but there is **no factory function** in those bindings yet — use Rust, Go, or the C ABI.

### File Upload

Uploads a file to the provider and returns a file ID.

Examples: [Node.js](api/node.md#file-upload) · [Python](api/python.md#file-upload) · [Rust](api/rust.md#file-upload) · [Go](api/go.md#file-upload) · [C ABI](api/c.md#file)

## Provider Factory Functions

### Text Generation

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `openai(apiKey, modelId, baseUrl?)` | OpenAI | gpt-4o |
| `anthropic(apiKey, modelId, baseUrl?)` | Anthropic | claude-3-5-sonnet-20241022 |
| `deepseek(apiKey, modelId, baseUrl?)` | DeepSeek | deepseek-chat |

### Vector Embedding

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `openaiEmbedding(apiKey, modelId, baseUrl?)` | OpenAI | text-embedding-3-small |
| `cohereEmbedding(apiKey, modelId, baseUrl?)` | Cohere | embed-english-v3.0 |
| `googleEmbedding(apiKey, modelId, baseUrl?)` | Google | gemini-embedding-001 |

### Speech Synthesis

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `openaiSpeech(apiKey, modelId, baseUrl?)` | OpenAI | tts-1 |

### Speech to Text

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `openaiTranscription(apiKey, modelId, baseUrl?)` | OpenAI | whisper-1 |

### Image Generation

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `openaiImage(apiKey, modelId, baseUrl?)` | OpenAI | dall-e-3 |
| `googleImage(apiKey, modelId, baseUrl?)` | Google | gemini-2.5-flash-image |

### Video Generation

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `googleVideo(apiKey, modelId, baseUrl?)` | Google | veo-3.0 |

### Reranking

| Function | Provider | Example modelId |
|---------|---------|-------------|
| `cohereReranking(apiKey, modelId, baseUrl?)` | Cohere | rerank-v3.0 |

### File Upload

| Function | Provider |
|---------|---------|
| `openaiFiles(apiKey, baseUrl?)` | OpenAI |

> The `baseUrl?` parameter of all factory functions is optional; by default each provider's official API address is used. When testing, pass a local mock server URL.

> **Per-language naming.** The tables above use the Node.js (camelCase) names. Each binding has its own naming convention for the same factories: Python uses snake_case (`openai_embedding`, `google_video`), Go uses `NewXxx` constructors returning `(T, error)` (`NewOpenAIEmbedding`, `NewGoogleVideo`), and the C ABI uses `aimux_<provider>_<feature>_new` (`aimux_openai_embedding_new`). Swift/Kotlin/Flutter currently expose only the language-model constructors (`Model.openai` / `Model.anthropic`). See [Feature Coverage](#feature-coverage) for the full matrix and each [language guide](#language-guides) for examples.

---

## Feature Coverage

Coverage verified against the current binding implementations (2026-08-01):

| Feature | Rust (core) | Node.js | Python | Swift | Kotlin | Flutter | Go | C/C++ | Java |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| Text generation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Streaming generation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Vector embedding | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Speech synthesis (TTS) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Speech to text (STT) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Image generation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Video generation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Reranking | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Search | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| File upload | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

- ✅ — available. All bindings now expose the full multimodal surface.
- **Node.js / Python** (native path): full multimodal surface — every factory in the [Provider Factory Functions](#provider-factory-functions) section.
- **Go** (C ABI path): full multimodal surface with typed wrappers — `NewOpenAIEmbedding` / `NewCohereEmbedding` / `NewGoogleEmbedding`, `NewOpenAISpeech`, `NewOpenAITranscription`, `NewOpenAIImage` / `NewGoogleImage`, `NewOpenAIFiles`, `NewCohereReranking`, `NewGoogleVideo`, `NewTavilySearch`, plus `DeepSeek`/`NewDeepSeek` and the typed `Generate`/`Stream` API. All multimodal constructors support `WithBase` variants.
- **C/C++** (C ABI path): full multimodal surface via the [C ABI function list](api/c.md#function-list).
- **Swift / Kotlin / Flutter / Java** (C ABI path): each now wraps all 8 multimodal model types alongside text generation and streaming. See each [language guide](#language-guides) for the API surface.

**How this table was derived** — every cell was checked against the binding's
own source (not inferred from another language). A feature counts as ✅ only
when the binding exposes a public factory **and** an invocable method for it;
⚠️ means the class exists but no factory function was found, so it cannot be
instantiated. Evidence per binding:

| Binding | Evidence (source of truth) |
|------|------|
| Rust | `aimux-core/src/` — one trait per feature: `language_model.rs`, `embedding_model.rs`, `speech_model.rs`, `transcription_model.rs`, `image_model.rs`, `video_model.rs`, `reranking_model.rs`, `search_model.rs`, `files_model.rs`; plus `generate.rs` (`generate_text` / `stream_text`) |
| Node.js | `bindings/node/index.d.ts` — 9 model classes + 16 factory functions (L12-152); `SearchModel.search` exists (L65) but no search factory in the export list |
| Python | `bindings/python/src/lib.rs` L203-231 — 8 multimodal classes registered via `add_class`, 10 multimodal factories via `add_function`; no search factory |
| Swift | `bindings/swift/Sources/Aimux/Aimux.swift` — `Model` (4 constructors) + `generateText` / `streamText` / `streamTextAsync` / `generate`; `Multimodal.swift` — 8 multimodal classes (EmbeddingModel, SpeechModel, TranscriptionModel, ImageModel, VideoModel, RerankingModel, SearchModel, Files) with factory constructors + methods + 19 Codable types |
| Kotlin | `bindings/kotlin/src/main/kotlin/aimux/Model.kt` — JNA interface declares all 38 ABI functions; `Multimodal.kt` — 8 multimodal Closeable classes with factory methods; `MultimodalTypes.kt` — serializable data classes for all result/option types |
| Flutter | `bindings/flutter/lib/aimux.dart` — `Model` (text); `multimodal.dart` — 8 multimodal classes with dart:ffi lookups for all 32 C ABI multimodal symbols |
| Go | `bindings/go/multimodal.go` — `NewOpenAIEmbedding` / `NewOpenAISpeech` / `NewOpenAITranscription` / `NewOpenAIImage` / `NewGoogleVideo` / `NewCohereReranking` / `NewTavilySearch` / `NewOpenAIFiles` + matching `ParseXxxResult`; embedding & image are OpenAI-only (no Cohere/Google constructors) |
| C/C++ | `aimux-ffi/src/lib.rs` — 36 exported `extern "C"` functions; full mapping in [c.md](api/c.md#function-list) |

> The ❌ / ⚠️ cells are tracked as actionable work items in
> [Binding API Gaps](api/gaps.md) — each gap lists the required C ABI
> functions and a reference implementation.

## Construction and base_url Support

| Binding | FFI method | base_url support | Construction example |
|------|---------|:---:|---------|
| **Node.js** | napi-rs (calls Rust directly) | ✅ 3rd parameter | `await openai(key, model, 'http://localhost:3000')` |
| **Python** | PyO3 (calls Rust directly) | ✅ 3rd parameter | `openai(key, model, "http://localhost:3000")` |
| **Swift** | C ABI (CAimuxFFI) | ✅ `baseUrl:` parameter | `try Model.openai(apiKey: key, modelId: model, baseUrl: url)` |
| **Kotlin** | C ABI (JNA) | ✅ 3rd parameter | `Model.openai(key, model, baseUrl)` |
| **Flutter/Dart** | C ABI (dart:ffi) | ✅ `baseUrl:` named parameter | `Model.openai(key, model, baseUrl: url)` |
| **Go** | C ABI (cgo static linking) | ✅ `OpenAIWithBase` | `aimux.OpenAIWithBase(key, model, url)` |
| **C/C++** | C ABI (direct linking) | ✅ `_with_base` function | `aimux_openai_new_with_base(key, model, url)` |

> The Node/Python bindings bypass the C ABI and call `aimux-providers` directly; Swift/Kotlin/Flutter/Go/C go through the `aimux-ffi` C ABI. Go uses cgo to statically link `libaimux_ffi.a`, producing a single binary (see [RFC-0011](../rfc/0011-golang-bindings.md) for details).

---

## Design Documents

| Document | Content |
|------|------|
| [RFC-0001](../rfc/0001-multilang-bindings.md) | Multi-language binding design |
| [RFC-0003](../rfc/0003-test-cassette.md) | Cassette testing design |
| [RFC-0008](../rfc/0008-multimodal-bindings.md) | Multimodal binding design |

---

## License

MIT
