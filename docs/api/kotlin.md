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

Unknown names throw an error listing the available providers.

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

`TypedModel` is `Closeable` (use `use { }`); errors surface as `AimuxException`.

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
