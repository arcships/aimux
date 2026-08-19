# aimux · Swift API

> Unified LLM service access layer — one API to access 325 AI providers

Swift wraps the Rust core through the `aimux-ffi` C ABI (module `CAimuxFFI`),
with ARC-managed model handles.

## Install

Add the package via SPM (the git tag is the version):

```swift
.package(url: "https://github.com/arcships/aimux", from: "0.3.0"),
.target(name: "YourApp", dependencies: [.product(name: "Aimux", package: "aimux")])
```

The binding loads `libaimux_ffi.dylib` at runtime — make it available via
pkg-config (`aimux-ffi.pc`) or put the prebuilt library from
[GitHub Releases](https://github.com/arcships/aimux/releases) on the loader
path (`DYLD_LIBRARY_PATH` / `LD_LIBRARY_PATH`).

## Quick Start

```swift
import Aimux

let model = try Model.openai(apiKey: "sk-...", modelId: "gpt-4o", baseUrl: "http://localhost:3000")
let result = try model.generateText(prompt: "\"What is Rust?\"")
print(result)
```

## Providers

All 251 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` is an enum with one case per provider:

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`Model.anthropic(apiKey:modelId:)`); custom endpoints → base-URL variant.
> Full list: [providers.md](providers.md).

```swift
// 推荐:ProviderName enum case(类型检查 + 补全)
let model = try Model.provider(name: ProviderName.Groq.rawValue, modelId: "llama-3.3-70b")
let result = try model.generateText(prompt: "\"Hello\"")

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
let model2 = try Model.provider(name: "groq", apiKey: "sk-...", modelId: "llama-3.3-70b")
```

Unknown names throw `.noSuchProvider` (payload: the provider id); valid names
come from the generated `ProviderName` enum.

## Text Generation

`generateText(prompt:options:)` returns the raw `GenerateResult` JSON string.

```swift
import Aimux

let model = try Model.openai(apiKey: "sk-...", modelId: "gpt-4o")
// prompt is a JSON string; JSON-quote plain text prompts
let result = try model.generateText(prompt: "\"What is Rust?\"")
// or pass multi-role messages
let result2 = try model.generateText(prompt: #"[{"role":"user","content":"Hello"}]"#)
print(result2)
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

`streamText(prompt:options:onPart:onDone:onError:)` delivers each part as a
`StreamPart` JSON string.

```swift
model.streamText(prompt: "\"Write a haiku\"") { part in
    print(part) // StreamPart JSON string
} onDone: {
    print("[done]")
} onError: { error in
    print("error: \(error)")
}
```

## API Surface

| API | Signature | Description |
|------|------|------|
| `Model.openai` / `Model.anthropic` | `static func openai(apiKey: String, modelId: String) throws -> Model` | Create a model (official base URL) |
| `Model.openai` / `Model.anthropic` | `static func openai(apiKey: String, modelId: String, baseUrl: String) throws -> Model` | Create a model (custom base URL) |
| `generateText` | `func generateText(prompt: String, options: String? = nil) throws -> String` | Non-streaming; returns `GenerateResult` JSON |
| `streamText` | `func streamText(prompt: String, options: String? = nil, onPart: @escaping (String) -> Void, onDone: @escaping () -> Void, onError: @escaping (any Error) -> Void)` | Streaming via push callbacks |
| `streamTextAsync` | `func streamTextAsync(prompt: String, options: String? = nil) -> AsyncThrowingStream<String, Error>` | Streaming as an `AsyncSequence` |
| `generate` | `func generate(prompt: String, options: [String: Any]? = nil) throws -> [String: Any]` | Convenience: parses `generateText` into a dictionary |
| `Model.initRecording` | `static func initRecording(dir: String) throws` | Start JSONL recording; throws `RecordingError` (`.initFailed` / `.openFile` / `.spawn`) when the recorder cannot be constructed; the previous recorder stays in place |
| `Model.recordingTryFlush` | `static func recordingTryFlush() throws` | Checked recorder flush; throws `RecordingError` (own type, see below). Legacy `recordingFlush()` stays and never reports |

### Errors

Two independent aimux error types, mirroring two Rust types; neither
inherits from the other, both share only `Error`: `AimuxError` (core
`AiMuxError`) and `RecordingError` (recorder). C ABI codes 200...206
have no Swift type — see "C ABI failures" below.

Every fallible C function returns an opaque `aimux_error_t *`
(`OpaquePointer?`): `NULL` = success (the result is in the trailing
out-parameter), non-`NULL` = failure. One unified code selects `AimuxError`
(1...13), `RecordingError` (100...105), or a C ABI failure (200...206).
The three decoders enforce the range expected by each call and restore the
Swift error type; 200...206 collapses to `DecodingError.dataCorrupted`.
Every path copies its strings (freed with
`aimux_free_string`) and then releases the returned error with
`aimux_error_free` exactly once. A code outside the C enum is a header/library mismatch
and yields the invariant `DecodingError.dataCorrupted("aimux ffi: <context>:
<message>")`. Errors are not handles: `aimux_drop_handle` never sees one.

`AimuxError` is a structured Swift `Error` enum mapped from
`aimux_error_code_t` (see `aimux-error.h`).

| Case | C code | Notes |
|------|--------|--------|
| `.jsonParse` | `AIMUX_E_JSON_PARSE` (2) | JSON parse/serialize |
| `.invalidResponseData` | `AIMUX_E_INVALID_RESPONSE_DATA` (3) | Malformed response / stream data |
| `.tool` | `AIMUX_E_TOOL` (4) | Tool-related failure |
| `.invalidArgument` | `AIMUX_E_INVALID_ARGUMENT` (5) | Bad argument |
| `.invalidPrompt` | `AIMUX_E_INVALID_PROMPT` (6) | Bad prompt JSON |
| `.tokenExpired` | `AIMUX_E_TOKEN_EXPIRED` (7) | Expired token; `status` 401 |
| `.unsupportedFunctionality` | `AIMUX_E_UNSUPPORTED_FUNCTIONALITY` (8) | Unsupported feature |
| `.noSuchModel` | `AIMUX_E_NO_SUCH_MODEL` (9) | Registry miss |
| `.noSuchProvider` | `AIMUX_E_NO_SUCH_PROVIDER` (10) | Unknown provider id |
| `.apiCall` | `AIMUX_E_API_CALL` (11) | Every HTTP-shaped failure; branch on `status` (401 auth, 404 model, 429 rate limit) |
| `.timeout` | `AIMUX_E_TIMEOUT` (12) | Request timed out |
| `.aborted` | `AIMUX_E_ABORTED` (13) | Request aborted |
| `.other` | `AIMUX_E_OTHER` (1) | Unclassified core error |

There are no binding-local cases: only aimux-core produces an `AimuxError`.
Un-encodable typed input (typed `streamText` prompt/options) surfaces as the
`EncodingError` `JSONEncoder` threw, undecodable library output as the
`DecodingError` `JSONDecoder` threw. A code outside the expected range is a
header/library mismatch and fails
with `DecodingError.dataCorrupted` from the decoder, not an error type.

Every case carries `message`, `status` (`Int?` — `nil` when C reports no
status), `retryMs` (`Int64?` — `nil` if none; `0` = retry now) and
`retryable`. Three cases carry a
typed payload as extra associated values: `.apiCall(providerCode:providerMessage:requestId:responseBody:)`
(all optional), `.noSuchModel(modelId:modelType:)` and
`.noSuchProvider(providerId:)`; the same-named computed properties return
`nil` on every other case. `e.code` returns the mapped `aimux_error_code_t`
constant as `Int32`.

**Recording errors are a separate type.** `Model.initRecording(dir:)` and
`Model.recordingTryFlush()` throw `RecordingError` — `struct RecordingError: Error, Equatable { code: Code;
message: String }` with `Code` = `.initFailed`, `.openFile`, `.spawn`,
`.writerGone`, `.flushTimeout`, `.write` (mirrors `aimux_error_code_t`
in `aimux-error.h`; the first three come from `initRecording`, the last three
from a flush). It is not
a case of `AimuxError` and never appears in the code table above; catch it
with `catch let e as RecordingError`.

**C ABI failures are native errors, not an aimux type.** The binding is
a consumer of the C ABI: it catches misuse before the C call and maps codes
200...206 onto Swift's own errors.

| Failure | Swift |
|---------|-------|
| Bad raw JSON (`prompt` / `options` / `configJson` / `values` / `recordingsJsonl` strings, on every raw-JSON entry) | `DecodingError.dataCorrupted` naming the parameter, thrown by the binding (`JSONSerialization` pre-check) before the C call. Optional parameters follow the FFI's "blank means default" rule; required ones reject an empty string |
| Use-after-close (`TranscriptionSession`, the only closeable handle) | `DecodingError.dataCorrupted "aimux: transcription session is closed"` from `pushAudio` / `inputDone` / `nextPart`. Catchable, like Go's `ErrClosed` and Dart's `StateError`; `defer { session.close() }` can never crash the host app. Deliberately **not** `AimuxTranscriptionEndedError` — a pump loop that `break`s on "ended" must not read a transcript its own `close()` truncated as complete |
| `initRecordingRing(cap: 0)` | `AimuxError.invalidArgument` — the value goes to C and aimux-core classifies it (`AIMUX_E_INVALID_ARGUMENT`, `"cap: must be > 0"`), so a Swift caller sees exactly what a C caller sees. The function is `throws` |
| `router([])` | `DecodingError.dataCorrupted "aimux ffi: router: …"` — C's zero-children failure, surfaced unchanged. Not a trap: the array is as likely to come from a `.filter` as from a literal, and `router` already throws |
| C code 200...206 (re-entrant call, NULL arg, invalid UTF-8, marshalling / callback / internal) | `DecodingError.dataCorrupted` `"aimux ffi: <context>: <message>"` — a binding/library invariant, a correct binding never triggers it |

No caller-supplied value traps the process. The binding contains exactly one
`preconditionFailure`, in `initLogging(level:)`, and it is unreachable: the
only failure `aimux_init_logging` reports is a non-UTF-8 `level`, which a
Swift `String` cannot produce (NULL and an unrecognized level string are both
accepted — they fall back to the default).

Accessors:

```swift
do {
    _ = try model.generateText(prompt: "\"hi\"")
} catch let e as AimuxError {
    print(e.message, e.status, e.retryMs)
    if case .apiCall = e, e.status == 429, let ms = e.retryMs {
        // rate limited — back off `ms`
    }
}
```

Streaming: `aimux_stream_text` returns `NULL` after `on_done`, or an error
object on failure (no `on_done`, no C `onError` callback). The Swift push
API's `onError` is
`(any Error) -> Void` and receives the decoded error unchanged (`AimuxError`,
or `DecodingError` for bad raw JSON / a C ABI invariant); `streamTextAsync`
throws the same.

## Types

`bindings/swift/Sources/Aimux/Types.swift` declares lightweight Codable types
mirroring the shared JSON shape — usable with the JSON-string APIs:

`JSONValue` (recursive JSON enum with `stringValue` / `boolValue` /
`doubleValue` / `intValue` / `arrayValue` / `objectValue` accessors), `Role`,
`FinishReasonUnified`, `ReasoningEffort`, `FinishReason`, `TokenUsage`,
`Usage`, `ResponseMetadata`, `Warning`, `FunctionTool`, `ProviderTool`,
`Tool`, `ToolChoice`, `ResponseFormat`, `ContentPart`, `MessageContent`,
`ModelMessage`, `ModelPrompt`, `ToolCall`, `FileBytes`, `FileData`,
`GenerateContent`, `GenerateResult`, `GenerateTextResult`,
`GenerateTextOptions`, `StreamPart` (all `Codable, Equatable`).

Example:

```swift
let data = result.data(using: .utf8)!
let decoded = try JSONDecoder().decode(Usage.self, from: data)
print(decoded.inputTokens.total ?? 0)
```

## Coverage

Text generation and streaming are supported. Multimodal features (embedding,
TTS, STT, image, video, rerank, search, files) are reachable only through the
raw [C ABI](c.md) until the wrappers are extended — see the
[coverage matrix](../API.md#feature-coverage).
