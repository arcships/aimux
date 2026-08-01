# aimux · Kotlin API

> Unified LLM service access layer — one API to access 172+ AI providers

Kotlin wraps the Rust core through the `aimux-ffi` C ABI (via JNA).

## Quick Start

```kotlin
Model.openai("sk-...", "gpt-4o", "http://localhost:3000").use { model ->
    val result = model.generateText("\"What is Rust?\"")
}
```

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
