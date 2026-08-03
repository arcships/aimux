# aimux · Flutter/Dart API

> Unified LLM service access layer — one API to access 290+ AI providers

Flutter/Dart wraps the Rust core through the `aimux-ffi` C ABI (via `dart:ffi`).

## Quick Start

```dart
final model = Model.openai('sk-...', 'gpt-4o', baseUrl: 'http://localhost:3000');
final result = model.generateText('What is Rust?');
model.close();
```

## Built-in Providers (RFC-0017 phase 4)

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` holds the constants:

> **Scope:** `provider(name, ...)` covers exactly these 250 OpenAI-compatible
> providers. Native protocols (Anthropic, Google, Bedrock, …), multimodal
> providers (ElevenLabs, Deepgram, …) and local inference (Ollama, vLLM, …)
> are **not** name-addressable — use their dedicated factories (e.g.
> `Model.anthropic(apiKey, modelId)`). Custom OpenAI-compatible endpoints:
> registry name + base-URL variant, or `Model.openai(apiKey, modelId, baseUrl:)`.
> See [API.md §Scope](../API.md#built-in-providers-rfc-0017-phase-4).

```dart
// 推荐:ProviderName.groq 常量(补全 + 防拼写错误)
final model = Model.provider(ProviderName.groq, 'llama-3.3-70b');
final result = model.generateText('Hello');
model.close();

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
final model2 = Model.provider('groq', 'llama-3.3-70b', apiKey: 'sk-...');
model2.close();
```

Unknown names throw an error listing the available providers.

## Text Generation

```dart
final model = Model.openai('sk-...', 'gpt-4o');
final result = model.generateText('What is Rust?');
model.close();
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

```dart
// streaming
final model = Model.openai('sk-...', 'gpt-4o');
final stream = model.streamText('Write a haiku');
await for (final part in stream) {
  if (part.containsKey('TextDelta')) print(part['TextDelta']['delta']);
}
model.close();
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

## TypedModel

The raw `Model` speaks JSON maps. `TypedModel` wraps it with typed objects:

```dart
final model = TypedModel(Model.openai('sk-...', 'gpt-4o'));
final result = model.generateText('What is Rust?');
print(result.text);  // typed GenerateTextResult

final stream = model.streamText('Write a haiku');
await for (final part in stream) {
  if (part is StreamPartTextDelta) print(part.delta);  // typed StreamPart variants
}
model.close();
```

| API | Signature | Description |
|------|------|------|
| `TypedModel` | `TypedModel(Model raw)` — wrap a raw `Model` | |
| `generateText` | `GenerateTextResult generateText(String prompt, [GenerateTextOptions? options])` | String prompt |
| `generateTextMessages` | `GenerateTextResult generateTextMessages(List<ModelMessage> messages, [GenerateTextOptions? options])` | Multi-turn typed messages |
| `streamText` | `Stream<StreamPart> streamText(Object prompt, [GenerateTextOptions? options])` | Yields typed `StreamPart`s |
| `close` | `void close()` | Release the native handle |

## Types

`bindings/flutter/lib/types.dart` declares the typed model surface (with
`toJson` / `fromJson` on each): `Role`, `FinishReasonUnified`,
`ReasoningEffort`, `TokenUsage`, `Usage`, `FinishReason`, `ToolCall`,
`FunctionTool`, `Tool`, `ToolChoice`, `ResponseMetadata`, `GenerateContent`
(sealed), `GenerateResult`, `GenerateTextResult`, `GenerateTextOptions`,
`ModelMessage`, `StreamPart` (sealed), `FileBytes`, `FileData`, `ContentPart`.

## Coverage

Text generation and streaming are supported. Multimodal features (embedding,
TTS, STT, image, video, rerank, search, files) are reachable only through the
raw [C ABI](c.md) until the wrappers are extended — see the
[coverage matrix](../API.md#feature-coverage).
