# aimux · Java API

> Unified LLM service access layer — one API to access 290+ AI providers

The Java binding goes through the `aimux-ffi` C ABI via JNA — no native
toolchain is needed at build time, and the native library ships as per-platform
classifier JARs. Artifact: `io.aimux:aimux-java:0.2.0`, Java 8+ (compiled with
`--release 8`). See [RFC-0013](../../rfc/0013-java-bindings.md) for the design.

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

## Quick Start

```java
try (Model model = Model.openaiWithBase("sk-...", "gpt-4o", "http://localhost:3000")) {
    String result = model.generateText("\"What is Rust?\"");
}
```

## Built-in Providers (RFC-0017 phase 4)

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` holds the constants:

> **Scope:** `provider(name, ...)` covers exactly these 250 OpenAI-compatible
> providers. Native protocols (Anthropic, Google, Bedrock, …), multimodal
> providers (ElevenLabs, Deepgram, …) and local inference (Ollama, vLLM, …)
> are **not** name-addressable — use their dedicated factories (e.g.
> `Model.anthropic(apiKey, modelId)`). Custom OpenAI-compatible endpoints:
> registry name + base-URL variant, or `Model.openaiWithBase(apiKey, modelId, baseUrl)`.
> See [API.md §Scope](../API.md#built-in-providers-rfc-0017-phase-4).

```java
import io.aimux.Model;
import io.aimux.ProviderName;

// 推荐:ProviderName.GROQ 常量(类型检查 + 补全)
try (Model model = Model.providerFromEnv(ProviderName.GROQ, "llama-3.3-70b")) {
    String result = model.generateText("\"Hello\"");
}

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
try (Model model = Model.provider("groq", "sk-...", "llama-3.3-70b", null)) {
    String result = model.generateText("\"Hello\"");
}
```

`deepseek(apiKey, modelId)` remains as a shortcut (registry-backed).
Unknown names throw an error listing the available providers.

## Architecture

Two layers, mirroring the Kotlin binding:

- **Raw layer** (`Model`, `EmbeddingModel`, `SpeechModel`, …) — JNA → C ABI,
  JSON strings in and out. `Model.generateText` returns the raw JSON result
  (or an `{"error":"..."}` envelope, which the caller must check). The
  multimodal raw classes throw `AimuxException` on an error envelope.
- **Typed layer** (`TypedModel`, `Types`, `MultimodalTypes`) — Jackson POJOs
  with the same wire format as all other bindings. `TypedModel` decodes results
  into typed objects and surfaces engine errors as `AimuxException`.

Serialization uses `JsonInclude.Include.NON_NULL` (null fields omitted on
encode; zero values and empty collections are retained).

## Text Generation

```java
// raw layer — JSON in, JSON out
try (Model model = Model.openai("sk-...", "gpt-4o")) {
    String result = model.generateText("\"What is Rust?\"");
}

// typed layer — typed objects
try (TypedModel model = TypedModel.openai("sk-...", "gpt-4o")) {
    Types.GenerateTextResult result = model.generateText("What is Rust?");
    System.out.println(result.getText());
    System.out.println(result.getUsage().getInputTokens().getTotal());
}
```

`TypedModel.generateText` overloads: `(String prompt)`,
`(String prompt, GenerateTextOptions options)`,
`(List<ModelMessage> messages)`,
`(List<ModelMessage> messages, GenerateTextOptions options)`.

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

```java
// raw layer — callback-based (JSON parts), blocks the calling thread
try (Model model = Model.openai("sk-...", "gpt-4o")) {
    model.streamText("\"Write a haiku\"", null,
        part -> System.out.println(part),   // onPart (String JSON)
        () -> {},                           // onDone
        err -> System.err.println(err));    // onError
}

// raw layer — pull-based (lazy Stream<String>)
model.streamTextStream("\"Write a haiku\"").forEach(System.out::println);

// typed layer — typed StreamPart objects
try (TypedModel model = TypedModel.openai("sk-...", "gpt-4o")) {
    model.streamTextStream("\"Write a haiku\"")
        .forEach(part -> System.out.println(part));  // Types.StreamPart
}
```

`TypedModel.streamText` (callback) and `streamTextStream` (pull) both have
`(String prompt)`, `(String prompt, options)`, `(List<ModelMessage> messages)`,
and `(List<ModelMessage> messages, options)` overloads.

> Stream part variants are documented in the
> [API overview](../API.md#streaming-generation).

## TypedModel

| API | Signature |
|------|------|
| `TypedModel.openai` / `TypedModel.anthropic` | `static TypedModel openai(String apiKey, String modelId)` (+ `openaiWithBase` / `anthropicWithBase` with `baseUrl`) |
| `TypedModel.of` | `static TypedModel of(Model model)` — wrap an existing raw `Model` (does not own the handle) |
| `generateText` | `GenerateTextResult generateText(...)` — 4 overloads (see above) |
| `streamText` | callback-based: `(prompt/options, Consumer<StreamPart> onPart, Runnable onDone, Consumer<String> onError)` |
| `streamTextStream` | `Stream<StreamPart> streamTextStream(...)` — 4 overloads, pull-based |

`TypedModel` is `Closeable` (owning factories must be closed with
try-with-resources); engine errors surface as `AimuxException`.

## Vector Embedding

```java
try (EmbeddingModel model = EmbeddingModel.openai("sk-...", "text-embedding-3-small")) {
    String result = model.embed("[\"hello\", \"world\"]");
}
// result: {"embeddings":[[0.1,0.2,...],[0.3,0.4,...]],"usage":{"tokens":5}, ...}
```

| Factory | Providers |
|------|------|
| `EmbeddingModel.openai` / `openaiWithBase` | OpenAI |
| `EmbeddingModel.cohere` / `cohereWithBase` | Cohere |
| `EmbeddingModel.google` / `googleWithBase` | Google |

`embed(String valuesJson)` / `embed(String valuesJson, String optsJson)`.

## Speech Synthesis (TTS)

```java
try (SpeechModel model = SpeechModel.openai("sk-...", "tts-1")) {
    String opts = new JSONObject()
        .put("text", "Hello")
        .put("voice", "alloy")
        .put("output_format", "mp3")
        .toString();
    String result = model.generate(opts);
}
```

Factories: `SpeechModel.openai` / `openaiWithBase`.

## Speech to Text (STT)

```java
try (TranscriptionModel model = TranscriptionModel.openai("sk-...", "whisper-1")) {
    String result = model.generate(base64Audio, "audio/wav");
}
```

Factories: `TranscriptionModel.openai` / `openaiWithBase`.
`generate(String audioBase64, String mediaType)` /
`generate(String audioBase64, String mediaType, String optsJson)`.

## Image Generation

```java
try (ImageModel model = ImageModel.openai("sk-...", "dall-e-3")) {
    String result = model.generate("{\"prompt\":\"an otter\",\"n\":1}");
}
```

Factories: `ImageModel.openai` / `openaiWithBase`, `ImageModel.google` /
`googleWithBase`.

## Video Generation

```java
try (VideoModel model = VideoModel.google("sk-...", "veo-3.0")) {
    String result = model.generate("{\"prompt\":\"a sunset\",\"n\":1}");
}
```

Factories: `VideoModel.google` / `googleWithBase`.

## Reranking

```java
try (RerankingModel model = RerankingModel.cohere("sk-...", "rerank-v3.0")) {
    String result = model.rerank("{\"query\":\"...\",\"documents\":{...},\"top_n\":2}");
}
```

Factories: `RerankingModel.cohere` / `cohereWithBase`.

## Search

```java
try (SearchModel model = SearchModel.tavily("sk-...")) {
    String result = model.search("{\"query\":\"What is Rust?\",\"max_results\":5}");
}
// result: {"results":[{"title":"Rust",...}],"answer":"Rust is a systems language."}
```

Factories: `SearchModel.tavily` / `tavilyWithBase`.

## File Upload

```java
try (Files files = Files.openai("sk-...")) {
    String result = files.uploadFile(base64Data, "application/pdf");
}
// result: {"provider_reference":{"openai":"file-abc"}, ...}
```

Factories: `Files.openai` / `openaiWithBase`.
`uploadFile(String dataBase64, String mediaType)` /
`uploadFile(String dataBase64, String mediaType, String optsJson)`.

## Types

`Types.java` declares the typed text/tool surface (all types nested in
`Types`): `TokenUsage`, `Usage`, `FinishReason`, `ResponseMetadata`, `ToolCall`,
`FunctionTool`, `ProviderTool`, `Tool` (sealed), `ToolChoice` (sealed, with
custom serializer for the scalar-or-object wire form), `ContentPart` (sealed),
`MessageContent` (sealed), `ModelMessage`, `GenerateTextOptions`,
`FileBytes` / `FileData`, `GenerateContent`, `GenerateResult`,
`GenerateTextResult`, `StreamPart` (sealed). All sealed hierarchies serialize
in the wrapper-object wire form (e.g. `{"TextDelta":{...}}`).

`MultimodalTypes.java` declares the typed multimodal surface:
`EmbeddingCallOptions` / `EmbeddingResult`, `SpeechCallOptions` / `SpeechResult`,
`ImageCallOptions` / `ImageResult`, `TranscriptionCallOptions` /
`TranscriptionResult`, `RerankingCallOptions` / `RerankingResult`,
`VideoCallOptions` / `VideoResult`, `SearchCallOptions` / `SearchResult`,
`UploadFileCallOptions` / `UploadFileResult`. Sealed unions (`AudioData`,
`ImageOutputs`, `VideoData`) use custom Jackson serializers with the
externally-tagged wire form. Response types (`EmbeddingResponse`, etc.) match
the `aimux-core` `.ts` wire definitions exactly (`{headers, body, ...}`).

## Error Handling

- **Raw text layer** (`Model.generateText`): returns the raw JSON string; an
  engine failure comes back as `{"error":"..."}` (caller must check).
- **Typed text layer** (`TypedModel`): `decodeResult` inspects the envelope and
  throws `AimuxException` with the error message.
- **Raw multimodal layer** (`EmbeddingModel`, `SpeechModel`, …): every call
  goes through `AimuxResult.extractString`, which throws `AimuxException` on an
  error envelope (parity with the Kotlin binding).

## Coverage

Full multimodal surface — text generation, streaming, embedding, TTS, STT,
image, video, reranking, search, and file upload. Verified by 58 tests across
7 suites (mock-server E2E + contract wire-format round-trips, no real network).
See the [coverage matrix](../API.md#feature-coverage).

## Build & Test

```bash
# First build the aimux-ffi .so
cargo build -p aimux-ffi --release

cd bindings/java
export JAVA_HOME=...   # JDK 9+ (bytecode targets Java 8 via --release 8)
export LD_LIBRARY_PATH="$(pwd)/../../target/release:${LD_LIBRARY_PATH}"
gradle test
```
