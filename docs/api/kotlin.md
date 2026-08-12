# aimux · Kotlin API

> Unified LLM service access layer — one API to access 325 AI providers

Kotlin wraps the Rust core through the `aimux-ffi` C ABI (via JNA).

## Install

Maven Central (publishing):

```kotlin
implementation("ai.arcships:aimux-kotlin:0.2.1")
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

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`aimux.ProviderName` holds the constants:

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
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

Engine and binding failures throw an **`AimuxException` sealed hierarchy**
(exhaustive `when` in Kotlin; Java callers can still `catch (AimuxException e)`).
Transport is Rust → C `AimuxError` → `AimuxException.fromC` — **not** a JSON
error envelope on the primary path.

```text
RuntimeException
 └── AimuxException          // code, status, retryMs, errorValue
      ├── JSONParseError / InvalidResponseDataError
      ├── ToolError
      ├── InvalidArgumentError / InvalidPromptError
      ├── TokenExpiredError          // 401, refresh and retry
      ├── UnsupportedFunctionalityError
      ├── NoSuchModelError / NoSuchProviderError
      ├── APICallError               // every HTTP-shaped failure; classify on status
      ├── TimeoutError / RequestAbortedError
      ├── OtherError
      └── UnknownAimuxError          // unrecognized / future code, raw code preserved
```

```kotlin
import ai.arcships.aimux.*

try {
    model.generateText("\"hi\"")
} catch (e: TokenExpiredError) {
    // 401 — refresh the token and retry
} catch (e: APICallError) {
    // Classify on status: 429 → rate limited (e.retryMs),
    // 401 → auth, 404 → model not found, -1 → transport failure
} catch (e: AimuxException) {
    // e.code (AIMUX_E_*), e.status, e.retryMs
}
```

| Field | Meaning |
|-------|---------|
| `code` | `AIMUX_E_*` matching C `AimuxErrorCode` (1..14; core variants are 2..14) |
| `status` | HTTP status when known; otherwise `-1` |
| `retryMs` | Rate-limit hint in ms; `-1` if none; `0` = retry immediately |

Local decode failures in `TypedModel` throw `InvalidArgumentError`. Stream
setup / terminal failures throw the typed hierarchy after optional legacy
`onError` notification (native C ABI has no `on_error` callback).

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
    model.streamText("\"Write a haiku\"", onPart = { println(it) }, onDone = {}, onError = {})
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

`TypedModel` is `Closeable` (use `use { }`); engine errors surface as
typed `AimuxException` subclasses (see [Errors](#errors)).

## Types

`bindings/kotlin/src/main/kotlin/aimux/Types.kt` declares the typed model
surface: `Role`, `FinishReasonUnified`, `ReasoningEffort`, `TokenUsage`,
`Usage`, `FinishReason`, `ResponseMetadata`, `ToolCall`, `FunctionTool`,
`ProviderTool`, `Tool` (sealed), `ToolChoice` (sealed), `ContentPart`
(sealed), `MessageContent` (sealed), `ModelMessage`, `GenerateTextOptions`,
`FileBytes` / `FileData` (sealed), `GenerateContent` (sealed),
`GenerateResult`, `GenerateTextResult`, `StreamPart` (sealed).

## Coverage

Full multimodal surface — text generation, streaming, embedding, TTS, STT,
image, video, rerank, search, and file upload (`Multimodal.kt` +
`MultimodalTypes.kt`), verified by mock-server end-to-end tests (no real
network). See the [coverage matrix](../API.md#feature-coverage).
