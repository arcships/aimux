# aimux API Documentation

> **Current release: 0.6.0.** 0.6 is a breaking release: every binding now uses
> the shared operations ABI (ABI 1 / ops 1.0), and the public JSON follows the
> Vercel AI SDK V4. Start with the [0.6 upgrade guide](upgrading-to-0.6.md) if
> you are coming from 0.5.

aimux is a Rust LLM access layer. Providers are reached through one object-safe
language-model interface, or through the shared operations dispatcher that the
language bindings use.

## Architecture

```text
Rust callers ──→ aimux-core + aimux-providers
                        │
                        ├─ aimux-ops: shared operation dispatcher (ops 1.0)
                        │
bindings ───────────────┼─→ native path: Node (napi-rs), Python (PyO3)
                        └─→ C ABI path: Swift, Kotlin, Flutter, Go, Java, C/C++
```

- **Rust core** — typed providers, traits, stream parts, errors and recording.
- **aimux-ops** — operation names, request/response DTOs, handle lifecycle,
  manifest and versions. Every binding calls the same dispatcher.
- **C ABI** — 14 primitives in `aimux-ffi/include/aimux.h`; opaque handles,
  JSON results and one owned `aimux_error_t *`.
- **Native bindings** — Node and Python link the Rust crates directly.

## Language guides

| Language | Path | Current guide |
|---|---|---|
| Rust | native | [api/rust.md](api/rust.md) |
| Node.js | native (napi-rs) | [api/node.md](api/node.md) · [README](../bindings/node/README.md) |
| Python | native (PyO3) | [api/python.md](api/python.md) · [README](../bindings/python/README.md) |
| Go | C ABI (cgo, static `.a`) | [api/go.md](api/go.md) · [README](../bindings/go/README.md) |
| Java | C ABI (JNA) | [api/java.md](api/java.md) · [README](../bindings/java/README.md) |
| Kotlin | C ABI (JNA, Java artifact + coroutines) | [api/kotlin.md](api/kotlin.md) · [README](../bindings/kotlin/README.md) |
| Swift | C ABI (Swift Package) | [api/swift.md](api/swift.md) · [README](../bindings/swift/README.md) |
| Flutter / Dart | C ABI (dart:ffi plugin) | [api/flutter.md](api/flutter.md) · [README](../bindings/flutter/README.md) |
| C / C++ | C ABI (direct link) | [api/c.md](api/c.md) · [README](../bindings/c/README.md) |

The per-language pages are orientation guides. The maintained implementation
notes, limits and test commands live in each binding's `README.md`.

## Quick start

Every binding uses the same three-step shape:

```text
provider.create  →  provider.model  →  call an operation (text.generate, text.stream, …)
```

Node.js:

```ts
import { Client, LanguageModel } from '@arcships/aimux'

const client = new Client()
const provider = await client.provider({
  package: 'openai',
  settings: { apiKey: process.env.OPENAI_API_KEY },
})
const model = await provider.model('gpt-4o-mini', 'languageModel')
if (!(model instanceof LanguageModel)) throw new Error('Expected LanguageModel')
try {
  console.log((await model.generate('Hello')).text)
} finally {
  await model.close()
  await provider.close()
}
```

Rust:

```rust
use aimux_core::prelude::*;
use aimux_providers::openai::openai;

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let model = openai().chat("gpt-4o-mini");
    let result = generate_text(
        &model,
        "Hello",
        GenerateTextOptions::default(),
    ).await?;
    println!("{}", result.text());
    Ok(())
}
```

The other languages' shortest complete examples are in their guides and
binding READMEs.

## Providers

Public copy says 300+ provider names. The generated catalogue is the exact
list: [api/providers.md](api/providers.md) (registry rows in
`provider_registry.json` plus the vendor packages).

Rust:

- `create_provider(name, PresetSettings)` covers presets and vendor packages.
- Vendor packages expose `create_xxx(XxxProviderSettings { .. })` plus a
  default `xxx()` where the package provides one.
- `provider_names()` lists accepted names; `Provider::discovery()` exposes
  runtime model listing where supported.

Bindings:

- Use `provider.create` with `{ "package": "<name>", "settings": { ... } }`.
- Use `provider.model` with `method` (`languageModel`, `embeddingModel`,
  `imageModel`, `speechModel`, `transcriptionModel`, `rerankingModel`,
  `videoModel`, `searchModel`, `decision`) and `modelId`.
- Provider-specific constructors from 0.5 were removed. See the
  [upgrade guide](upgrading-to-0.6.md).

## Operations

The authoritative operation list is [api/ops.md](api/ops.md) and
`runtime.describe`. Families include:

| Family | Examples |
|---|---|
| runtime | `runtime.describe`, `runtime.configure` |
| provider | `provider.create`, `provider.model`, `provider.files`, `provider.invoke`, `provider.wrap` |
| text | `text.generate`, `text.stream`, `text.generateObject`, `text.consume` |
| embedding | `embedding.embed`, `embedding.embedMany` |
| speech / transcription | `speech.generate`, `transcription.start`, `operation.push`, `operation.next` |
| image / video | `image.generate`, `video.generate` |
| reranking / search | `reranking.rerank`, `search.search` |
| files | `files.upload` |
| decision | `decision.decide`, `decision.capabilities` |
| recording / replay | `recorder.*`, `replay.*` |
| sessions / trace | `sessionStore.*`, `trace.*` |
| composite / middleware | `composite.router`, `composite.moa`, `middleware.create`, `model.wrap` |
| tools | `toolRepair.context`, `toolRepair.apply`, `toolRepair.applyResult` |
| transport | `transport.create` |

Operation request and response DTOs are generated from the Rust types and
published in each binding. Use the generated types rather than hand-written
JSON when the binding offers them.

## Handles and streams

- Handles are decimal strings, including values above 2^53.
- Adopt each returned handle exactly once; `close()` is idempotent.
- A provider keeps its models alive; closing a provider does not invalidate an
  already-created model.
- Streams deliver business event frames plus one explicit end frame. Data
  errors stay data; the end frame carries the terminal outcome.
- Queue limits are binding-specific and documented in each binding README.
  Slow consumers apply backpressure; cancellation must wake a full queue.
- Do not adopt a handle twice or move a handle between processes/isolates
  unless the binding explicitly supports it.

## Errors

All public errors use the shared envelope:

```json
{ "kind": "aimux", "code": 1, "message": "...", "error": { "name": "..." } }
```

| Range | Domain |
|---|---|
| `1..20` | core `AiMuxError` |
| `100..105` | recording |
| `200..206` | C ABI / binding boundary |
| `300..306` | ops protocol |

`error` is the typed AI SDK-shaped payload. `cause` is either a nested aimux
error or an underlying OS/TLS/parser error; the native cause chain follows it.
Retry delays are read from `error.responseHeaders` (`retry-after-ms`, then
`retry-after`), not from a dedicated retry field. See
[error-model.md](error-model.md).

## Recording and replay

Recording is opt-in and explicit: create a recorder, attach it, then flush it
before closing. 0.6 records schema 2; schema 3 and transport replay are a
follow-up. Recorded business JSON follows the same AI SDK V4 shapes as live
calls. See [api/ops.md](api/ops.md), [RFC-0023](../rfc/0023-runtime-request-recording.md)
and the `aimux-replay` tool.

## Feature coverage

The shared ops manifest is the single capability source. All bindings expose
the same operations; a provider may support only a subset of methods, which
`runtime.describe`, `provider.discovery()` or `model.info` reports. The
per-language guides document binding-specific wrappers (for example Node's
`LanguageModel` class or Kotlin's Flow helpers).

High-level surface:

- text generation, streaming, tools, structured output, middleware;
- embeddings, reranking, search, files;
- speech, transcription, realtime transcription operations;
- image and video generation;
- decision models;
- composite router / MoA;
- recording, replay, sessions and tracing.

## Breaking changes

0.6.0 removes the 0.5 constructors, error getters and hand-written DTOs. The
central checklist is [upgrading-to-0.6.md](upgrading-to-0.6.md). The exhaustive
itemized list is the `Breaking` section of [CHANGELOG.md](../CHANGELOG.md).

## Design documents

- [Project overview](PROJECT-OVERVIEW.md)
- [Error model](error-model.md)
- [Ops ABI upgrade](upgrading-to-ops-abi.md)
- [AI SDK alignment](aisdk-architecture-alignment.md)
- [RFC index](../rfc/)

## License

MIT
