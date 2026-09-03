# aimux · Flutter/Dart API

> Unified LLM service access layer — one API to access 325 AI providers

Flutter/Dart wraps the Rust core through the `aimux-ffi` C ABI (via `dart:ffi`).

## Install

On [pub.dev](https://pub.dev/packages/aimux) (publisher: [`arcships.ai`](https://pub.dev/publishers/arcships.ai)):

```bash
flutter pub add aimux
```

The package is a Flutter plugin: the Rust core ships inside it —
`libaimux_ffi.so` per ABI (Android) and `aimux_ffi.xcframework` (iOS) are
embedded at publish time, so no extra download or build step is needed.
iOS integrates via CocoaPods (`ios/aimux.podspec`) — Flutter's SwiftPM
integration does not link plugin binary targets as of Flutter 3.44, so the
podspec is the integration path; the vendored `aimux_ffi.xcframework` slice is
force-loaded into the app binary and resolved with `DynamicLibrary.process()`.

While developing against the repo, depend on the path:

```yaml
dependencies:
  aimux:
    path: bindings/flutter
```

Desktop (Linux/macOS/Windows) is supported for development and tests: the
library is resolved from the platform library path — build it once with
`cargo build -p aimux-ffi --release` and point `LD_LIBRARY_PATH`
(`DYLD_LIBRARY_PATH` on macOS) at `target/release`.

The binding loads the platform library at runtime
(`libaimux_ffi.so` / `libaimux_ffi.dylib` / `aimux_ffi.dll`) — ship it with
your app or place it where the loader can find it.

## Quick Start

```dart
final model = Model.openai('sk-...', 'gpt-4o', baseUrl: 'http://localhost:3000');
final result = model.generateText('What is Rust?');
model.close();
```

## Providers

All 251 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` holds the constants:

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`Model.anthropic(apiKey, modelId)`); custom endpoints → base-URL variant.
> Full list: [providers.md](providers.md).

```dart
// 推荐:ProviderName.groq 常量(补全 + 防拼写错误)
final model = Model.provider(ProviderName.groq, 'llama-3.3-70b');
final result = model.generateText('Hello');
model.close();

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
final model2 = Model.provider('groq', 'llama-3.3-70b', apiKey: 'sk-...');
model2.close();
```

Unknown names throw `NoSuchProviderError` (payload: the provider id); valid
names come from the generated `ProviderName` constants.

## Errors

Two independent aimux exception types, mirroring two Rust types — neither
shares a base with the other (both just `implements Exception`):

| Source | Rust | Dart | C code |
|---|---|---|---|
| AiMux | `AiMuxError` | `AimuxException` hierarchy | 1..14 |
| recorder | `RecordingError` | `RecordingException` | 100..105 |

Every fallible C call returns an opaque `aimux_error_t *` (`NULL` =
success, result in the trailing out-parameter). The binding has one decoder
(`errors.dart`): `expectAimuxError(e, context)` for model calls,
`expectRecordingError(e, context)` for `initRecording` / `recordingTryFlush`,
`expectFfiError(e, context)` for utilities that can only fail in the C ABI.
One unified code selects 1..14, 100..105, or 200..206; each decoder copies the
relevant fields, releases the error with `aimux_error_free` exactly once, and
throws the matching `AimuxException` subclass / `RecordingException`. Codes
200..206 throw the native
`StateError('aimux ffi: …')` — see *C ABI failures* below.

`AiMuxError` values throw an **`AimuxException` subclass hierarchy**
(idiomatic Dart — `is` / `on` type checks, not stringly `code` switches):

```text
Exception (implements)
 └── AimuxException
      ├── JSONParseError / InvalidResponseDataError
      ├── ToolError
      ├── InvalidArgumentError / InvalidPromptError
      ├── TokenExpiredError         // status 401
      ├── UnsupportedFunctionalityError
      ├── NoSuchModelError / NoSuchProviderError
      ├── APICallError              // every HTTP-shaped failure; branch on status
      ├── RetryError                // reason, errors, lastError
      ├── AimuxTimeoutError
      ├── RequestAbortedError
      └── OtherError
```

Every instance has `message`, `code` (`AimuxErrorCode` constants matching C
`aimux_error_code_t`), `status` (HTTP or `-1`), `retryMs` (hint or `-1`;
`0` = retry now) and `retryable`. Per-code payload lives on the carrying
subclass only: `APICallError.providerCode` / `.providerMessage` / `.responseBody`
(`String?`), `NoSuchModelError.modelId` / `.modelType`, and
`NoSuchProviderError.providerId`. A code outside the enum is a header/library
mismatch and fails with `StateError`, not an error type.

`APICallError` additionally exposes the sanitized URL/request values,
response headers/body, parsed provider data/code, and retryability.
`RetryError` preserves every concrete attempt error in `errors`, with
`lastError` and `reason`.

```dart
import 'package:aimux/aimux.dart'; // exports errors.dart

try {
  final result = model.generateText('hi');
} on APICallError catch (e) {
  if (e.status == 429) {
    // rate limited; e.retryMs is the hint
  } else if (e.status == 401) {
    // auth failure
  }
} on AimuxException catch (e) {
  // any AiMuxError failure
}
```

**C ABI failures** — the call was wrong, not the model — are native Dart
errors, never an aimux type. Raw JSON string parameters (`configJson`,
`optsJson`, `valuesJson`, …) are validated with `jsonDecode` in Dart *before*
the C call. Empty/blank is rejected for required raw-JSON params (`valuesJson`,
the `optsJson` of speech/image/video/rerank/search, and the `configJson` of
`registerProviders` / `initProxy`) and treated as "defaults" for optional ones
(nullable in the signature — provider/router/moa `configJson`,
embed/upload/transcription `optsJson`), matching the C ABI; use-after-close is
guarded locally. A returned C code in 200..206 is a binding/library invariant:

| Failure | Dart |
|---|---|
| bad raw JSON string parameter | `FormatException` (`source` = parameter name, e.g. `config_json`) — before C |
| string not representable on the wire (below) | `FormatException` — before C |
| use-after-close | `StateError('… is closed')` — before C |
| C code 200..206 | `StateError('aimux ffi: …')` (invariant) |

### Strings the wire format cannot carry

A Dart `String` is UTF-16 and may hold an **unpaired surrogate** — `s.substring(0,
n)` cutting an emoji in half is enough. `jsonDecode` and `jsonEncode` both accept
one (`jsonEncode` writes it back as a `\uD800` escape), but the Rust side's
`serde_json` rejects that escape, and `toNativeUtf8` would otherwise silently
substitute U+FFFD. An **interior NUL** is worse: it is not rejected anywhere, it
just ends the C string early. Both are rejected in Dart with a `FormatException`
naming the parameter, for raw JSON parameters *and* for the JSON the binding
builds itself (`prompt`, `options`, `ProviderConfig`):

```dart
final chunk = userText.substring(0, 100);   // may split an emoji
try {
  model.generateText(chunk);
} on FormatException catch (e) {
  // 'prompt: unpaired surrogate U+D83D at index 99 is not representable in UTF-8'
}
```

The same check rejects a number Dart parses to `Infinity` (`1e999`) and nesting
deeper than 128 levels — both of which `serde_json` also rejects. Without it
each of these arrived as an uncatchable-looking invariant `StateError` from the
C ABI.

Recording errors are a **separate type**, mirroring Rust's independent
`recording::RecordingError`: `recordingTryFlush()` throws
`RecordingException` (`implements Exception`, *not* an `AimuxException`) with
`code` (`RecordingErrorCode.init / openFile / spawn / writerGone /
flushTimeout / write`, from C `aimux_error_code_t`; only the last
three are reachable from a flush) and `message`. It returns normally when
nothing is recording. `initRecording(dir)` throws the same
`RecordingException` with code `init` / `openFile` / `spawn` when the recorder
cannot be constructed (the previous recorder stays in place). The legacy
`recordingFlush()` stays and never reports.

Use-after-close on a Dart wrapper (`Model`, `ProviderHandle`, the multimodal
models, `TranscriptionSession`, `Files`) throws `StateError`.
Stream terminal failures surface via `Stream.addError` with whatever the
decoder produced (`AimuxException`, or the native `StateError` for a C ABI
failure) and the stream is then closed — there is no `on_error` callback;
provider mid-stream `StreamPart::Error` is data on `on_part`. A
`TranscriptionSession.nextPart` timeout is a poll state, not an error:
`AimuxTranscriptionTimeoutException` (session still live); a normal end is
`AimuxTranscriptionEndedException`.

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

The cache-probing / trace API (RFC-0015 — `aimux_trace_new` and the
`aimux_trace_*` queries) is **not** exposed by this binding: there is no
`trace()` wrapper and no trace query method, so the trace store cannot be
reached from Dart. Use the raw [C ABI](c.md) if you need it.
