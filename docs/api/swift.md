# aimux · Swift API

> Unified LLM service access layer — one API to access 325 AI providers

Swift wraps the Rust core through the `aimux-ffi` C ABI (module `CAimuxFFI`),
with ARC-managed model handles.

## Install

Add the package via SPM (the git tag is the version):

```swift
.package(url: "https://github.com/arcships/aimux", from: "0.3.0"),
.target(name: "Aimux", dependencies: ["Aimux"])
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

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`ProviderName` is an enum with one case per provider:

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`Aimux.anthropic(apiKey:modelId:)`); custom endpoints → base-URL variant.
> Full list: [providers.md](providers.md).

```swift
// 推荐:ProviderName enum case(类型检查 + 补全)
let model = try Aimux.provider(name: ProviderName.groq.rawValue, modelId: "llama-3.3-70b")
let result = try model.generateText(prompt: "\"Hello\"")

// 字符串形式同样可用 + 可选 config JSON ({"base_url": "..."}):
let model2 = try Aimux.provider(name: "groq", apiKey: "sk-...", modelId: "llama-3.3-70b")
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
| `streamText` | `func streamText(prompt: String, options: String? = nil, onPart: @escaping (String) -> Void, onDone: @escaping () -> Void, onError: @escaping (String) -> Void)` | Streaming via push callbacks |
| `streamTextAsync` | `func streamTextAsync(prompt: String, options: String? = nil) -> AsyncThrowingStream<String, Error>` | Streaming as an `AsyncSequence` |
| `generate` | `func generate(prompt: String, options: [String: Any]? = nil) throws -> [String: Any]` | Convenience: parses `generateText` into a dictionary |

### Errors

`AimuxError` is a structured Swift `Error` enum mapped from the C ABI
`AimuxError` / `AimuxErrorCode` (see `aimux-error.h`). Fallible FFI calls pass
an out-param; on failure the return sentinel is `0` / `NULL` and the filled
struct is converted with `AimuxError.fromC(_:)`.

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
| `.other` | `AIMUX_E_OTHER` (14) | Unclassified core error |
| `.unknown` | `AIMUX_E_UNKNOWN` (1) / unexpected | Future or empty err |
| `.invalidHandle` | *(local)* | Zero / freed handle |
| `.serializationError` | *(local)* | Binding encode/decode |

Every engine-mapped case carries `message`, `status` (HTTP or `-1`), and
`retryMs` (`-1` if none; `0` = retry now). Accessors:

```swift
do {
    _ = try model.generateText(prompt: "\"hi\"")
} catch let e as AimuxError {
    print(e.message, e.status, e.retryMs)
    if case .apiCall = e, e.status == 429, e.retryMs >= 0 {
        // rate limited — back off
    }
}
```

Streaming: `aimux_stream_text` returns non-zero on success and fills `err` on
failure (no C `onError` callback). The Swift push API still takes `onError`
and receives `e.message`; `streamTextAsync` throws the structured `AimuxError`.

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
