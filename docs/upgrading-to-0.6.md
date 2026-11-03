# Upgrading to aimux 0.6

> **0.6.0 is a breaking release.** It moves every binding onto the shared
> operations ABI (ABI 1 / ops 1.0) and aligns the public JSON with the Vercel
> AI SDK V4. There are no compatibility aliases for the 0.5 constructors,
> error getters or wire shapes.

This guide is the entry point. The exhaustive, itemized list is the
`Breaking` section of the [0.6.0 changelog](../CHANGELOG.md#060---2026-11-03); each
binding's `README.md` under [`bindings/`](../bindings/) documents its current
API and limits.

## Upgrade all layers together

A 0.6 runtime requires the matching header, generated metadata and language
packages. Do not mix 0.5 and 0.6 artifacts:

| Layer | 0.6 requirement |
|---|---|
| Rust crates | `0.6.0` (workspace version) |
| C ABI | ABI major `1` (`AIMUX_ABI_VERSION == 65536`) |
| Ops protocol | `1.0` (`AIMUX_OPS_VERSION`) |
| C header | `aimux-ffi/include/aimux.h` generated from the same commit |
| Manifest | the runtime's manifest `digest` must equal the generated `MANIFEST_DIGEST` |
| Bindings | Node/Python/Go/Java/Kotlin/Swift/Flutter/C from the same release |

A native load fails fast with `UnsupportedOpsVersion` (protocol code 302) when
the ABI, ops version or manifest digest differs. Rebuild native consumers
against the new header; 0.5 forwarding aliases were intentionally removed.

## Shared changes

### 1. Operations and handles replace provider constructors

The bindings now address the shared operation dispatcher instead of
provider-specific constructors. A typical flow is:

```text
provider.create  ->  provider.model  ->  text.generate / text.stream / ...
```

- Provider-specific constructors such as `openai(...)`, `Model.openai(...)`,
  `NewOpenAI(...)` and `aimux_openai_new(...)` are gone.
- Handles are decimal strings, including values above 2^53. Adopt each
  returned handle exactly once and close it explicitly.
- `runtime.describe` returns the manifest, versions and available operation
  names. Use it to discover supported methods for a provider.
- Streams deliver business event frames plus one explicit end frame. A data
  error frame is ordinary data; the end frame describes termination.
- The C ABI has 14 primitives; the generated header is the source of truth.

### 2. Public JSON follows AI SDK V4

The public JSON is no longer aimux's own dialect. The important changes are:

- struct fields are `camelCase` (`toolCallId`, `providerOptions`, `inputTokens`);
- unions carry the AI SDK's tag and strings (`{"type":"text-delta"}`,
  `"tool-call"`, `"reasoning-file"`, `Source.sourceType`);
- file content is the AI SDK's `FilePart`; generated files are `GeneratedFile`;
- `usage`, `steps`, `finishReason`, response messages, tool parts and typed
  outputs use the AI SDK's shapes;
- timestamps use JavaScript's `Date.toISOString()` form;
- `null` and an absent key are distinct where the AI SDK distinguishes them;
- request options reject unknown control fields instead of silently ignoring
  them.

The generated DTOs in every binding mirror these shapes. Do not keep a
hand-written 0.5 DTO alongside the generated one.

### 3. Errors are the shared envelope, with causes

All public errors use:

```json
{ "kind": "aimux", "code": 1, "message": "...", "error": { "name": "..." } }
```

- `kind` is one of the error domains (`aimux`, `recording`, `binding`,
  `protocol`).
- `code` is the stable numeric code. Ops protocol codes are `300..306`.
- `error` is the typed AI SDK-shaped payload; unknown fields are retained.
- The payload may carry `cause`: a nested aimux error (with `name`) or an
  underlying OS/TLS/parser error (`{message, cause?}`). The native `cause` /
  `__cause__` / `getCause()` / `Unwrap()` follows the same chain.
- Retry hints are no longer fields on the envelope. Read
  `error.responseHeaders["retry-after-ms"]`, then `retry-after`, as with the
  AI SDK.

See [error-model.md](error-model.md) for the code ranges and per-language
mapping.

### 4. Recording and replay

Recording stays on schema 2 in 0.6; schema 3 and transport replay are a
separate follow-up. Existing 0.5 recordings remain readable. The recorder,
session store and tracing are still process-wide configuration; they are not
created implicitly by a provider constructor.

## Rust

- `ProviderName` and the generated provider-name enum are removed. Use
  `provider_names()` / `create_provider(name, PresetSettings)` for presets and
  the vendor `create_xxx(XxxProviderSettings { .. })` packages.
- Providers implement the AI SDK-shaped `Provider` surface:
  `language_model`, `embedding_model`, `image_model`, `speech_model`,
  `transcription_model`, `reranking_model`, `video_model`, `search_model`,
  `files`, `decision_model`.
- Provider packages follow the AI SDK settings shape; the old `XxxConfig`
  builders and `from_env()` helpers are removed. Use `XxxProviderSettings`
  with `create_xxx`, or `create_provider` for presets.
- `CallOptions` / `GenerateTextOptions` still exist, but their public JSON is
  camelCase and their fields changed (for example the structured-output
  `output` option). `CallOptions.call_id`, `for_step` and the old per-call
  recording context are gone.
- `GenerateTextResult`, `TextStreamPart`, `StepResult`, content parts and tool
  calls use the AI SDK V4 shapes. Tool inputs are parsed and validated at the
  Core boundary; invalid calls are returned as `invalid: true` with a typed
  error.
- Decision models use the AI SDK `DecisionModelV4` shape. Obtain one with
  `provider.decision_model` or `provider.model(..., method: "decision")` and
  call `decision.capabilities` / `decision.decide`; the old decision options
  and result shapes are replaced.
- Retry is a call-level concern only. `RetryConfig` and provider-level retry
  settings are gone; use `maxRetries` and the call timeout settings.
- Provider options and metadata have one typed shape; arbitrary
  `body_overrides` are removed.
- Middleware wraps generation and streaming through the shared hooks.
- The C ABI and Rust API both expose the same operation names; the Rust
  `aimux_ops` crate is the shared dispatcher.

Start with [docs/api/rust.md](api/rust.md), then check the `Breaking` section of
the changelog for the exact field you use.

## Node.js

The top-level `openai(...)` / `generateText(...)` helpers are gone. Use the
typed ops facade:

```ts
import { Client, LanguageModel } from '@arcships/aimux'

const client = new Client()
const provider = await client.provider({
  package: 'openai',
  settings: { apiKey: process.env.OPENAI_API_KEY },
})
const model = await provider.model('gpt-4o-mini', 'languageModel')
try {
  if (!(model instanceof LanguageModel)) throw new Error('Expected Language')
  console.log((await model.generate('Hello')).text)
  for await (const frame of await model.stream('Hello')) {
    if (frame.type === 'event') console.log(frame.event)
    else console.log(frame.outcome, frame.error)
  }
} finally {
  await model.close()
  await provider.close()
}
```

- The raw `@arcships/aimux/raw` entrypoint exposes `Runtime.call` /
  `Runtime.stream` and the same DTOs.
- Errors are `AimuxError` with `kind`, numeric `code`, `message`, `error` and
  the standard `error.cause`.
- A stream's explicit end frame describes success, error or cancellation.
  Recoverable event errors remain data.
- `close()` is idempotent; flush a recorder explicitly before closing.

See [bindings/node/README.md](../bindings/node/README.md) and the
[Node guide](api/node.md).

## Python

The top-level model helpers are gone. Use `Client` and `Handle`:

```python
from aimux import Client

client = Client()
with client.adopt(client.call("provider.create", {
    "package": "openai", "settings": {"apiKey": "your-api-key"},
})["handle"]) as provider:
    with client.adopt(provider.call("provider.model", {
        "method": "languageModel", "modelId": "gpt-4o-mini",
    })["handle"]) as model:
        result = model.call("text.generate", {
            "prompt": "Hello", "options": {},
        })
        print(result["text"])
```

- `Client.call` / `Handle.call` return parsed JSON; `stream` returns a
  context-managed iterator of `{type, seq, event}` frames.
- Adopt every returned handle once; `close()` cancels, joins and clears.
- Python errors are native exceptions with `envelope`, `error`, `kind` and
  `code`; the payload's `cause` is `__cause__`.
- `arcships-aimux[typed]` adds the optional Pydantic schemas and text helpers.

See [bindings/python/README.md](../bindings/python/README.md) and the
[Python guide](api/python.md).

## Go

Provider constructors are replaced by operation calls:

```go
provider, err := aimux.Create(ctx, nil, "provider.create", map[string]any{
    "package": "openai",
    "settings": map[string]any{"apiKey": os.Getenv("OPENAI_API_KEY")},
})
// ...
model, err := aimux.Create(ctx, provider, "provider.model", map[string]any{
    "method": "languageModel", "modelId": "gpt-4o-mini",
})
result, err := aimux.CallInto[aimux.GenerateTextResult](ctx, model, "text.generate", map[string]any{
    "prompt": "Hello", "options": aimux.GenerateTextOptions{},
})
```

- `Create` owns the returned entry, whether it is a decimal handle string or
  an object containing `handle`. Never adopt the same ID twice.
- `Close` no longer joins an in-flight call; handles are atomic and owners must
  not be copied after first use. `go vet` flags copies.
- `Call` / `CallInto` use canonical JSON; `Params` preserves absent vs null.
- Streams deliver copied event JSON; `Events()` is bounded and must be drained
  or cancelled.
- `errors.As` yields `*aimux.Error`; `Unwrap()` exposes the cause chain.

See [bindings/go/README.md](../bindings/go/README.md) and the
[Go guide](api/go.md).

## Java and Kotlin

Java and Kotlin use the C ABI through JNA. Java 8 bytecode remains supported.

- `Aimux.load()` validates ABI and ops versions before use.
- Provider-specific constructors are gone; use `Aimux.call` /
  `ProviderHandle` and the generated operation constants in `AimuxOps`.
- Wire DTOs are generated in `ai.arcships.aimux.types`; the hand-written
  `Types` / `MultimodalTypes` copies are removed.
- Kotlin no longer has an independent JNA implementation. It depends on the
  matching Java artifact and adds coroutine helpers in
  `ai.arcships.aimux.kotlin`.
- Every asynchronous Java method takes an explicit `Executor`; cancellation
  maps to native Abort. A callback must return in bounded time.

See [bindings/java/README.md](../bindings/java/README.md),
[bindings/kotlin/README.md](../bindings/kotlin/README.md), the
[Java guide](api/java.md) and the [Kotlin guide](api/kotlin.md).

## Swift

- Use `AimuxRuntime` for global operations and `AimuxHandle` for owned
  handles.
- DTOs are Codable generated types in `generated/Types.swift`; the operation
  constants are in `AimuxOps`.
- Link the matching `aimux_ffi` library or the packaged XCFramework.
- `BoundedStream` uses a blocking producer and one consumer; call `close()`
  when abandoning a stored iterator.
- `AimuxError` retains the full envelope; `cause` follows the native chain.

See [bindings/swift/README.md](../bindings/swift/README.md) and the
[Swift guide](api/swift.md).

## Flutter / Dart

- `Aimux.open()` validates the ABI, ops version and manifest before use.
- Use `call(op, handle:, params:, abort:)`; adopt each returned handle once
  with `client.adopt(...)`.
- Streams expose `StreamFrameEvent` and an explicit `StreamFrameEnd`; an error
  in the end frame is not thrown a second time.
- Native calls run on background isolates; pass an abort handle for
  cancellation.
- `OpsException` preserves the complete envelope; `cause` is the native cause
  chain.

See [bindings/flutter/README.md](../bindings/flutter/README.md) and the
[Flutter guide](api/flutter.md).

## C and C++

- Rebuild against `aimux-ffi/include/aimux.h`; the old `aimux_openai_new` and
  other provider exports are removed.
- Use the 14 ABI primitives: create a provider via `provider.create`, then a
  model via `provider.model`, then call the operation by name.
- Errors are one owned `aimux_error_t *`; read the envelope JSON with
  `aimux_error_json` and free it exactly once. `NULL` means success.
- Stream callbacks receive borrowed frame bytes; copy before returning.
- The C++ wrapper's `aimux::NativeError` carries the envelope JSON, including
  `error.cause`.
- See [upgrading-to-ops-abi.md](upgrading-to-ops-abi.md) for the old-to-new C
  export mapping.

See [bindings/c/README.md](../bindings/c/README.md) and the
[C guide](api/c.md).

## Migration checklist

- [ ] Pin every package and native artifact to `0.6.0`; do not mix versions.
- [ ] Replace provider-specific constructors with `provider.create` +
      `provider.model` (or the binding's typed facade).
- [ ] Regenerate and re-import DTOs; delete hand-written 0.5 wire types.
- [ ] Replace 0.5 error getters with the shared envelope and `cause` chain.
- [ ] Read retry delays from `responseHeaders`, not `retryMs`.
- [ ] Update stream handling to expect one explicit end frame.
- [ ] Update recording fixtures that stored 0.5 JSON; schema 2 itself is
      unchanged, but the business JSON inside a recording follows the new
      AI SDK shapes.
- [ ] Run the binding's native contract tests and your own integration tests
      against the new header and manifest digest.

## Rollback

If you must roll back, pin the entire 0.5 release: Rust crates, Node/Python
packages, JVM artifacts, native libraries and the generated header. A 0.5
binding cannot load a 0.6 native library, and a 0.6 binding cannot load a 0.5
library. Do not mix them in one process.
