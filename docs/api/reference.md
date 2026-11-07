# aimux API reference

> **0.6.0.** The generated DTOs and the ops manifest are the source of truth.
> This page is an index; it deliberately does not duplicate every constructor
> or wire field. See the [0.6 upgrade guide](../upgrading-to-0.6.md) for the
> breaking changes.

## Authoritative sources

| Surface | Source |
|---|---|
| Operation names and DTO schemas | [`aimux-ops/src/manifest.rs`](../../aimux-ops/src/manifest.rs), [`docs/api/ops.md`](ops.md), `runtime.describe` |
| Rust types | [`aimux-core/src/`](../../aimux-core/src/), [`aimux-providers/src/`](../../aimux-providers/src/) |
| C header | [`aimux-ffi/include/aimux.h`](../../aimux-ffi/include/aimux.h) (generated) |
| Node DTOs | [`bindings/node/src/generated/types.ts`](../../bindings/node/src/generated/types.ts) |
| Python DTOs | [`bindings/python/python/aimux/_types.py`](../../bindings/python/python/aimux/_types.py) (generated; `arcships-aimux[typed]` also provides Pydantic models) |
| Go DTOs | [`bindings/go/types_gen.go`](../../bindings/go/types_gen.go) |
| Java DTOs | [`bindings/java/generated/ai/arcships/aimux/types/`](../../bindings/java/generated/ai/arcships/aimux/types/) under `ai.arcships.aimux.types` |
| Kotlin DTOs | Java DTOs plus [`bindings/kotlin/generated/AimuxOps.kt`](../../bindings/kotlin/generated/AimuxOps.kt) |
| Swift DTOs | [`bindings/swift/generated/Types.swift`](../../bindings/swift/generated/Types.swift) |
| Flutter DTOs | [`bindings/flutter/lib/src/generated/types.dart`](../../bindings/flutter/lib/src/generated/types.dart) |
| Provider catalogue | [`docs/api/providers.md`](providers.md) (generated) |

## Core Rust types

| Type | Source | Notes |
|---|---|---|
| `LanguageModel` | [`aimux-core/src/language_model.rs`](../../aimux-core/src/language_model.rs) | object-safe provider trait; `generate_text` / `stream_text` call it |
| `Provider` | [`aimux-core/src/provider.rs`](../../aimux-core/src/provider.rs) | AI SDK-shaped provider trait: language, embedding, image, speech, transcription, reranking, video, search, files, decision |
| `CallOptions` | [`aimux-core/src/options.rs`](../../aimux-core/src/options.rs) | base options for every modality |
| `GenerateTextOptions` | [`aimux-core/src/generate.rs`](../../aimux-core/src/generate.rs) | text options: output, tools, reasoning, retry/timeout, provider options |
| `GenerateTextResult` | [`aimux-core/src/generate.rs`](../../aimux-core/src/generate.rs) | `steps`, `text`, `content`, `usage`, `finish_reason`, `output`, request/response |
| `TextStreamPart` | [`aimux-core/src/stream_part.rs`](../../aimux-core/src/stream_part.rs) | call-level stream parts, AI SDK tags |
| `ContentPart` | [`aimux-core/src/content.rs`](../../aimux-core/src/content.rs) | text / reasoning / tool / file / source / custom parts |
| `AiMuxError` | [`aimux-core/src/error.rs`](../../aimux-core/src/error.rs) | typed core error; `source()` is the cause chain |
| `Tool` / `ToolChoice` | [`aimux-core/src/tool.rs`](../../aimux-core/src/tool.rs) | function and provider tools |
| `Output` | [`aimux-core/src/output.rs`](../../aimux-core/src/output.rs) | structured output option |

## Operations

The operation list is in [`ops.md`](ops.md). Every binding uses the same
names. A typical sequence is:

```text
provider.create → provider.model → text.generate / text.stream / ...
```

`runtime.describe` returns the manifest, ABI/ops versions, manifest digest and
the operation names available in the loaded native library. A binding must
reject a mismatched ABI, ops version or digest before any call.

## Errors

The shared envelope is:

```json
{ "kind": "aimux", "code": 1, "message": "...", "error": { "name": "..." } }
```

Code ranges: core `1..20`, recording `100..105`, C ABI/binding `200..206`,
ops protocol `300..306`. `error.cause` carries a nested error or an underlying
OS/TLS/parser error. Retry delays live in `error.responseHeaders`
(`retry-after-ms`, then `retry-after`). See
[`error-model.md`](../error-model.md).

## See also

- [API overview](../API.md)
- [Per-language guides](.)
- [0.6 upgrade guide](../upgrading-to-0.6.md)
- [Ops ABI upgrade](../upgrading-to-ops-abi.md)
- [Providers](providers.md)
- [Error model](../error-model.md)
