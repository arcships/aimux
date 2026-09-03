# aimux · Java API

> Unified LLM service access layer — one API to access 325 AI providers

The Java binding goes through the `aimux-ffi` C ABI via JNA — no native
toolchain is needed at build time. Artifact: `ai.arcships:aimux-java:0.3.0`,
Java 8+ (compiled with `--release 8`). See [RFC-0013](../../rfc/0013-java-bindings.md)
for the design.

## Install

Maven Central (publishing):

```groovy
implementation("ai.arcships:aimux-java:0.3.0")
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

All 251 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` holds the constants:

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
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

`AiMuxError` values throw an **`AimuxException` subclass hierarchy**
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
      ├── RetryError                 // reason, errors, lastError
      ├── TimeoutError / RequestAbortedError
      └── OtherError
```

Every instance has:

| Field | Meaning |
|-------|---------|
| `getMessage()` | human-readable text from C |
| `getCode()` | `aimux_error_code_t` value 1–14, where 14 = `Retry` (matches `aimux-error.h`) |
| `getStatusCode()` | HTTP status, or `-1` |
| `getRetryMs()` | rate-limit hint, or `-1` (`0` = retry now) |
| `isRetryable()` | the `AiMuxError` retry verdict (not derivable from status) |

A code outside the enum is a header/library mismatch and fails with
`IllegalStateException`, not an error type.

Four subclasses carry the C payload of their variant (`null` when
unavailable): `APICallError` — `getProviderCode()`, `getProviderMessage()`,
`getResponseBody()`, `getUrl()`, `getRequestBodyValues()`,
`getResponseHeaders()`, `getData()`; `RetryError` — `getReason()`
(`RetryErrorReason.MAX_RETRIES_EXCEEDED` — every permitted attempt failed
with a retryable error — or `ERROR_NOT_RETRYABLE` — a later attempt failed
non-retryably), `getErrors()` (the per-attempt history, oldest first, each
itself an `AimuxException` — typically `APICallError` with its full detail),
`getLastError()`; `NoSuchModelError` — `getModelId()`, `getModelType()`;
`NoSuchProviderError` — `getProviderId()`.

Recording failures are a **separate type**, mirroring the two unrelated
Rust error types: `Aimux.initRecording(dir)` and `Aimux.recordingTryFlush()`
throw `RecordingException` (a plain `RuntimeException`, *not* an
`AimuxException`) whose `getCode()` is a `RecordingErrorCode` — `INIT`,
`OPEN_FILE`, `SPAWN`, `WRITER_GONE`, `FLUSH_TIMEOUT`, `WRITE` (C
`aimux_error_code_t` 100–105). `initRecording` reports `INIT` / `OPEN_FILE` /
`SPAWN` (the previous recorder, if any, stays in place); a flush reports the
last three. The legacy `recordingFlush()` never reports.

**C ABI failures** are codes 200–206, not an aimux type. The binding catches what a
caller can trigger *before* the C call and throws the native Java exception;
anything in that C code range is a binding/library invariant:

| Situation | Java |
| --- | --- |
| `null` for a required `String` (`apiKey`, `modelId`, `promptJson`, `configJson`, `valuesJson`, `dir` …) | `NullPointerException("<param>")`, thrown before the C call |
| Malformed raw JSON (`promptJson`, `optsJson`, `configJson`, `valuesJson`, `recordingsJsonl` …), including trailing garbage or `""` for a required param | `IllegalArgumentException("<param>: invalid JSON: …")`, thrown before the C call |
| Use-after-close of any handle (`Model`, `EmbeddingModel`, `TranscriptionSession`, …) | `IllegalStateException("<Type> is closed")`, thrown before the C call |
| C code 200–206 (null pointer, invalid UTF-8, malformed wire JSON, dead handle, re-entrant call, result serialization, callback failure) | `IllegalStateException("aimux ffi: …")` — a binding/library invariant, never a caller error |

`TypedModel` failing to decode what the library returned is likewise a
binding/library invariant: `IllegalStateException("aimux: failed to decode
<Type>: …")`.

Known and accepted: the multimodal classes (`EmbeddingModel`, `SpeechModel`, …)
guard `close()` with an `AtomicLong` rather than a lock, so a `close()` racing an
in-flight call can surface as `IllegalStateException("aimux ffi: invalid or
expired … handle")` instead of `"<Type> is closed"` — still an
`IllegalStateException`.

`AimuxException` and `RecordingException` mirror the two unrelated Rust error
types and share no base beyond `RuntimeException`.

Transport: every fallible C call returns an opaque `aimux_error_t *`
(JNA `Pointer`) — `null` on success with the result in a trailing out-parameter
(`LongByReference` handle / `PointerByReference` JSON), non-null on failure.
`AimuxResult` reads one unified code: 1–14 restores the matching
`AimuxException` subclass, 100–105 restores `RecordingException`, and 200–206
becomes `IllegalStateException("aimux ffi: …")`. Payload getters are read only
under their owning AiMuxError code; a `RetryError`'s attempt errors are new
owned errors, decoded recursively and freed by the binding. Every returned
string is freed and the returned error is released with
`aimux_error_free` exactly once (errors are never handles). No JSON error
envelopes on the main path. Subclasses are nested under `AimuxException` (e.g.
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
    // any AiMuxError failure
}
```

Stream terminal failures also throw (there is no C `on_error` callback). The
raw `streamText` has no `onError` parameter; only `TypedModel.streamText` takes
one, and it reports local decode failures only, never C-level failures.

## Architecture

Two layers, mirroring the Kotlin binding:

- **Raw layer** (`Model`, `EmbeddingModel`, `SpeechModel`, …) — JNA → C ABI,
  JSON strings in and out. `AiMuxError` values throw typed `AimuxException`
  subclasses, decoded from the `aimux_error_t *` a failed call returns;
  C ABI misuse throws plain `IllegalArgumentException` /
  `IllegalStateException` (see Errors).
- **Typed layer** (`TypedModel`, `Types`, `MultimodalTypes`) — Jackson POJOs
  with the same wire format as all other bindings. `TypedModel` decodes results
  into typed objects; `AiMuxError` values propagate as `AimuxException`.

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
    // No onError: stream failures throw AimuxException from streamText.
    model.streamText("\"Write a haiku\"", null,
        part -> System.out.println(part),   // onPart (String JSON)
        () -> {});                          // onDone
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
try-with-resources); `AiMuxError` values surface as `AimuxException`.

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
`VideoCallOptions` / `VideoPollOptions` / `VideoResult`, `SearchCallOptions` / `SearchResult`,
`UploadFileCallOptions` / `UploadFileResult`. Sealed unions (`AudioData`,
`ImageOutputs`, `VideoData`) use custom Jackson serializers with the
externally-tagged wire form. Response types (`EmbeddingResponse`, etc.) match
the `aimux-core` `.ts` wire definitions exactly (`{headers, body, ...}`).

## Error Handling

- **All fallible raw APIs** (`Model.generateText`, multimodal, …): a failed C
  call returns an `aimux_error_t *`; Java decodes it and throws the typed
  `AimuxException` subclass (see [Errors](#errors) above). Success payloads
  are plain result JSON strings, not error envelopes.
- **Typed text layer** (`TypedModel`): AiMuxError failures throw the same hierarchy;
  failing to decode the library's own output is a binding invariant and throws
  `IllegalStateException("aimux: failed to decode <Type>: …")`.

## Coverage

Full multimodal surface — text generation, streaming, embedding, TTS, STT,
image, video, reranking, search, and file upload. Verified by the JUnit suite
(mock-server E2E + contract wire-format round-trips, no real network).
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
