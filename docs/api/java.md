# aimux · Java API

> Unified LLM service access layer — one API to access 325 AI providers

The Java binding goes through the `aimux-ffi` C ABI via JNA — no native
toolchain is needed at build time. Artifact: `ai.arcships:aimux-java:0.2.1`,
Java 8+ (compiled with `--release 8`). See [RFC-0013](../../rfc/0013-java-bindings.md)
for the design.

## Install

Maven Central (publishing):

```groovy
implementation("ai.arcships:aimux-java:0.2.1")
```

JNA loads `aimux_ffi` by name — provide the native library
(`libaimux_ffi.so` / `libaimux_ffi.dylib` / `aimux_ffi.dll`) from
[GitHub Releases](https://github.com/arcships/aimux/releases) on the JNA search
path (`-Djava.library.path=...` or `LD_LIBRARY_PATH`).

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

## Quick Start

```java
try (Model model = Model.openaiWithBase("sk-...", "gpt-4o", "http://localhost:3000")) {
    String result = model.generateText("\"What is Rust?\"");
}
```

## Providers

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` holds the constants:

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`Model.anthropic(apiKey, modelId)`); custom endpoints → base-URL variant.
> Full list: [providers.md](providers.md).

```java
import ai.arcships.aimux.Model;
import ai.arcships.aimux.ProviderName;

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
Unknown names throw `NoSuchProviderError` naming the requested provider
(valid names come from the generated `ProviderName` constants).

## Errors

Engine and binding failures throw an **`AimuxException` subclass hierarchy**
(OpenAI Java / Vercel AI SDK style — `instanceof`, not stringly `code` checks):

```text
RuntimeException
 └── AimuxException
      ├── JSONParseError / InvalidResponseDataError
      ├── ToolError
      ├── InvalidArgumentError / InvalidPromptError
      ├── TokenExpiredError          // 401, refresh and retry
      ├── UnsupportedFunctionalityError
      ├── NoSuchModelError / NoSuchProviderError
      ├── APICallError               // every HTTP-shaped failure; classify on getStatusCode()
      ├── TimeoutError / RequestAbortedError
      └── OtherError
```

Every instance has:

| Field | Meaning |
|-------|---------|
| `getMessage()` | human-readable text from C |
| `getCode()` | `AimuxErrorCode` value 0–14 (matches `aimux-error.h`) |
| `getStatusCode()` | HTTP status, or `-1` |
| `getRetryMs()` | rate-limit hint, or `-1` (`0` = retry now) |

Transport: fallible C calls take a trailing `AimuxError *err` and return
`0` / `NULL` on failure. The Java binding maps that into the hierarchy via
`AimuxException.fromC(AimuxCError)` — **not** JSON error envelopes on the
main path. Subclasses are nested under `AimuxException` (e.g.
`AimuxException.APICallError`).

```java
import ai.arcships.aimux.AimuxException;
import ai.arcships.aimux.AimuxException.APICallError;
import ai.arcships.aimux.AimuxException.TokenExpiredError;
import ai.arcships.aimux.Model;

try (Model model = Model.openai("sk-...", "gpt-4o")) {
    model.generateText("\"hi\"");
} catch (TokenExpiredError e) {
    // 401 — refresh the token and retry
} catch (APICallError e) {
    // Classify on status: 429 → rate limited (e.getRetryMs()),
    // 401 → auth, 404 → model; -1 → no HTTP response observed
} catch (AimuxException e) {
    // any engine / binding failure
}
```

Stream terminal failures also throw (there is no C `on_error` callback). The
raw `streamText` `onError` parameter is retained for API compatibility (e.g.
typed decode issues) but is not used for C-level failures.

## Architecture

Two layers, mirroring the Kotlin binding:

- **Raw layer** (`Model`, `EmbeddingModel`, `SpeechModel`, …) — JNA → C ABI,
  JSON strings in and out. Failures throw typed `AimuxException` subclasses
  from the C `AimuxError` out-param (handle `0` / pointer `NULL`).
- **Typed layer** (`TypedModel`, `Types`, `MultimodalTypes`) — Jackson POJOs
  with the same wire format as all other bindings. `TypedModel` decodes results
  into typed objects; engine errors propagate as `AimuxException`.

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

- **All fallible raw APIs** (`Model.generateText`, multimodal, …): C sentinel
  failure fills `AimuxError *`; Java throws `AimuxException.fromC` (typed
  subclasses — see [Errors](#errors) above). Success payloads are plain result
  JSON strings, not error envelopes.
- **Typed text layer** (`TypedModel`): engine failures throw the same hierarchy;
  local decode failures throw `AimuxException` / `InvalidArgumentError`.

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
