# aimux · Kotlin API

> Unified LLM service access layer — one API to access 325 AI providers

Kotlin wraps the Rust core through the `aimux-ffi` C ABI (via JNA).

## Install

Maven Central (publishing):

```kotlin
implementation("ai.arcships:aimux-kotlin:0.3.0")
```

JNA loads `aimux_ffi` by name — provide the native library
(`libaimux_ffi.so` on Linux, `libaimux_ffi.dylib` on macOS, `aimux_ffi.dll` on
Windows) from [GitHub Releases](https://github.com/arcships/aimux/releases) on
the JNA search path: `java.library.path`, `LD_LIBRARY_PATH`, or next to the JAR.

## Quick Start

```kotlin
Model.openai("sk-...", "gpt-4o", "http://localhost:3000").use { model ->
    val result = model.generateText("\"What is Rust?\"")
}
```

## Providers

All 251 registry-backed OpenAI-compatible providers are reachable by name;
`ai.arcships.aimux.ProviderName` holds the constants:

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`Model.anthropic(apiKey, modelId)`); custom endpoints → base-URL variant.
> Full list: [providers.md](providers.md).

```kotlin
// 推荐:ProviderName.GROQ 常量(类型检查 + 补全)
Model.provider(name = ProviderName.GROQ, modelId = "llama-3.3-70b").use { model ->
    val result = model.generateText("\"Hello\"")
}

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
Model.provider(name = "groq", apiKey = "sk-...", modelId = "llama-3.3-70b").use { model ->
    val result = model.generateText("\"Hello\"")
}
```

Unknown names throw `NoSuchProviderError` naming the requested provider
(valid names come from the generated `ProviderName` constants).

## Errors

Two aimux exception types, each mirroring its own Rust type — **AiMux**
(`AimuxException`) and **recorder** (`RecordingException`). They share no base
beyond `RuntimeException`; catch each on its own. Every fallible C call returns
an `aimux_error_t *` (null = success, result in the out-parameter). The
binding reads one unified code: 1..13 restores an `AimuxException` subclass,
100..105 restores `RecordingException`, and 200..206 becomes
`IllegalStateException("aimux ffi: …")`. Payload getters are read only under
their owning AiMuxError code.
Each helper frees every string and frees the returned error once
(`aimux_error_free`) — **not** a JSON error envelope on the primary path,
and never a handle. Kotlin does not add a third aimux error type for C ABI
failures.

`AiMuxError` values throw the **`AimuxException` sealed hierarchy** (exhaustive
`when` in Kotlin; Java callers can still `catch (AimuxException e)`). It reads
`code`, `message`, `retryable` for every code and the payload getters only under
their owning code.

```text
RuntimeException
 └── AimuxException          // code, status, retryMs, retryable
      ├── JSONParseError / InvalidResponseDataError
      ├── ToolError
      ├── InvalidArgumentError / InvalidPromptError
      ├── TokenExpiredError          // 401, refresh and retry
      ├── UnsupportedFunctionalityError
      ├── NoSuchModelError / NoSuchProviderError   // modelId + modelType / providerId
      ├── APICallError               // every HTTP-shaped failure; classify on status
      │                              // + providerCode, providerMessage, requestId, responseBody (null when absent)
      ├── TimeoutError / RequestAbortedError
      └── OtherError
```

A code outside the enum is a header/library mismatch and fails with `IllegalStateException`, not an error type.

```kotlin
import ai.arcships.aimux.*

try {
    model.generateText("\"hi\"")
} catch (e: TokenExpiredError) {
    // 401 — refresh the token and retry
} catch (e: APICallError) {
    // Classify on status: 429 → rate limited (e.retryMs),
    // 401 → auth, 404 → model not found, -1 → no HTTP response observed
} catch (e: AimuxException) {
    // e.code (AIMUX_E_*), e.status, e.retryMs
}
```

| Field | Meaning |
|-------|---------|
| `code` | `AIMUX_E_*` matching C `aimux_error_code_t` (1..13; 1 is the catch-all `Other`) |
| `status` | HTTP status when known; otherwise `-1` |
| `retryMs` | Rate-limit hint in ms; `-1` if none; `0` = retry immediately |

Recording errors are a separate type, mirroring Rust's `recording::RecordingError`
(C codes 100..105): `initRecording()` and `recordingTryFlush()` throw
`RecordingException(code: RecordingErrorCode, message)` — a plain `RuntimeException`,
**not** an `AimuxException` — with `code` one of `INIT, OPEN_FILE, SPAWN,
WRITER_GONE, FLUSH_TIMEOUT, WRITE`. `initRecording()` reports `INIT` (dir could not
be created), `OPEN_FILE`, `SPAWN` and leaves any previous recorder in place; a flush
reports the last three. The legacy `recordingFlush()` stays and never reports.

**C ABI failures.** The binding validates what only the caller can get
wrong *before* the C call: malformed raw JSON text (`promptJson`, `optsJson`,
`configJson`, `valuesJson`; required arguments reject empty, optional empty =
default, JSONL by line) throws `IllegalArgumentException` naming the Kotlin
parameter; any method on a closed `Model` / `ProviderHandle` / multimodal model
/ `TranscriptionSession` throws `IllegalStateException("X is closed")`. Anything
the C layer itself reports as 200..206 (dead or type-mismatched handle,
re-entrant call, NULL / non-UTF-8 string, unserializable
result, panicking callback, internal) is a binding or library invariant and
surfaces as `IllegalStateException("aimux ffi: …")`. None of these are
`AimuxException`.

| Failure | Kotlin / Java |
|---------|---------------|
| bad raw JSON argument | `IllegalArgumentException("promptJson: …")` (before C) |
| use-after-close | `IllegalStateException("Model is closed")` |
| C code 200..206 | `IllegalStateException("aimux ffi: …")` (binding/library invariant) |

Local decode failures in `TypedModel` throw `InvalidArgumentError`. Stream
setup and terminal failures throw the typed hierarchy; the raw `Model.streamText`
has no `onError` parameter (the C ABI has no `on_error` callback), and
`TypedModel.streamText`'s `onError` reports local decode failures only.

## Text Generation

```kotlin
Model.openai("sk-...", "gpt-4o").use { model ->
    val result = model.generateText("\"What is Rust?\"")
}
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

```kotlin
// streaming
Model.openai("sk-...", "gpt-4o").use { model ->
    // Raw Model has no onError: stream failures throw AimuxException from streamText.
    model.streamText("\"Write a haiku\"", onPart = { println(it) }, onDone = {})
}
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

## TypedModel

The raw `Model` speaks JSON strings. `TypedModel` wraps it with typed objects:

```kotlin
val model = TypedModel.openai("sk-...", "gpt-4o")
val result = model.generateText("What is Rust?")
println(result.text)          // typed GenerateTextResult
println(result.usage?.inputTokens?.total)
```

| API | Signature |
|------|------|
| `TypedModel.openai` / `TypedModel.anthropic` | `fun openai(apiKey: String, modelId: String): TypedModel` (+ `baseUrl` overload) |
| `TypedModel.of` | `fun of(model: Model): TypedModel` — wrap an existing raw `Model` |
| `generateText` | `fun generateText(prompt: String, options: GenerateTextOptions? = null): GenerateTextResult` |
| `generateText` | `fun generateText(messages: List<ModelMessage>, options: GenerateTextOptions? = null): GenerateTextResult` |
| `streamText` | callback-based streaming (`onPart: (StreamPart) -> Unit`, `onDone`, `onError`) |
| `streamTextSequence` | `fun streamTextSequence(...): Sequence<StreamPart>` — pull-based streaming |

`TypedModel` is `Closeable` (use `use { }`); `AiMuxError` values surface as
typed `AimuxException` subclasses (see [Errors](#errors)).

## Streaming Transcription (STT)

Realtime transcription models (e.g. OpenAI `gpt-realtime-whisper`) support
streaming sessions (RFC-0028): push audio chunks, then pull transcription
parts. `TranscriptionModel.startStream` returns a `TranscriptionSession`
(`Closeable`):

```kotlin
TranscriptionModel.openai("sk-...", "gpt-realtime-whisper").use { model ->
    model.startStream().use { session ->
        session.pushAudio(chunk)          // blocking (backpressure)
        session.inputDone()               // end-of-audio (idempotent)
        while (true) {
            try {
                val part = session.nextPart(timeoutMs = 500)
                println(part)             // JSON TranscriptionStreamPart
            } catch (e: TranscriptionSession.AimuxTranscriptionEndedException) {
                break                     // stream finished normally
            } catch (e: TranscriptionSession.AimuxTranscriptionTimeoutException) {
                // No part within timeoutMs — retryable: the session stays
                // live, just call nextPart again.
            }
        }
    }
}
```

`nextPart(timeoutMs)`: `timeoutMs > 0` waits at most that long; `0` polls
immediately; `< 0` waits indefinitely. Outcomes:

| Exception | Meaning |
|-----------|---------|
| returns a `String` | the next part (JSON `TranscriptionStreamPart`) |
| `AimuxTranscriptionEndedException` | the stream finished normally |
| `AimuxTranscriptionTimeoutException` | no part in time — **retryable**, the session stays live |
| `AimuxException` subclasses | the stream failed (typed hierarchy) |

The timeout sentinel is deliberately **not** an `AimuxException` / `TimeoutError`
— a timeout is not a stream failure, so catch it explicitly (same shape as the
Go / Java / Swift / Flutter bindings). `close()` aborts and releases the
session (idempotent).

## Types

`bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Types.kt` declares the
typed model surface: `Role`, `FinishReasonUnified`, `ReasoningEffort`,
`TokenUsage`, `Usage`, `FinishReason`, `ResponseMetadata`, `ToolCall`,
`FunctionTool`, `ProviderTool`, `Tool` (sealed), `ToolChoice` (sealed), `ContentPart`
(sealed), `MessageContent` (sealed), `ModelMessage`, `GenerateTextOptions`,
`FileBytes` / `FileData` (sealed), `GenerateContent` (sealed),
`GenerateResult`, `GenerateTextResult`, `StreamPart` (sealed).

## Coverage

Full multimodal surface — text generation, streaming, embedding, TTS, STT
(incl. streaming `TranscriptionSession`), image, video, rerank, search, and
file upload (`Multimodal.kt` + `MultimodalTypes.kt`), verified by
mock-server end-to-end tests (no real network). See the
[coverage matrix](../API.md#feature-coverage).
