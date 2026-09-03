# aimux

<p align="center">
  <img src="assets/aimux-banner.png" alt="aimux banner" width="100%">
</p>

> **A unified LLM access layer written in Rust. One API for 329 AI providers.**

[![CI](https://github.com/arcships/aimux/actions/workflows/ci.yml/badge.svg)](https://github.com/arcships/aimux/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Providers](https://img.shields.io/badge/providers-329-green.svg)](docs/api/providers.md)
[![Bindings](https://img.shields.io/badge/bindings-8-9cf.svg)](bindings/)
[![crates.io](https://img.shields.io/crates/v/aimux-core)](https://crates.io/crates/aimux-core)
[![npm](https://img.shields.io/npm/v/@arcships/aimux)](https://www.npmjs.com/package/@arcships/aimux)
[![PyPI](https://img.shields.io/pypi/v/arcships-aimux)](https://pypi.org/project/arcships-aimux/)
[![Go Reference](https://pkg.go.dev/badge/github.com/arcships/aimux/bindings/go.svg)](https://pkg.go.dev/github.com/arcships/aimux/bindings/go)
[![GitHub Release](https://img.shields.io/github/v/release/arcships/aimux)](https://github.com/arcships/aimux/releases)
[![Maven Central](https://img.shields.io/maven-central/v/ai.arcships/aimux-java)](https://central.sonatype.com/artifact/ai.arcships/aimux-java)

aimux is a Rust implementation of a unified LLM provider access layer. It
collapses the HTTP APIs of every AI provider into a single
`dyn LanguageModel` interface that anything upstream can call.

Unlike **rig** or **langchain**, aimux does **not** build agent loops, RAG, or
orchestration — it focuses exclusively on unifying service access. That is the
difference: aimux is an access layer, those are orchestration layers.

---

## Why aimux

- **329 provider modules** — 251 registry-backed OpenAI-compatible
  (unified `provider(name, ...)` entry) + 10 native protocol
  implementations (OpenAI, Anthropic, Google, Bedrock, Vertex, Azure, Cohere,
  Mistral, xAI, Anthropic-AWS) + 68 standalone/modality/local/search providers
  (OpenRouter, DeepSeek, Ollama, vLLM, ElevenLabs, KlingAI, Tavily, …).
  Full list: [docs/api/providers.md](docs/api/providers.md).
- **Unified, object-safe interface** — the `LanguageModel` trait supports
  `Box<dyn>` so providers are interchangeable without changing call sites.
- **Full multimodal** — text, streaming, tool calling, embeddings, image,
  speech, transcription, video, reranking, files — plus **realtime
  transcription sessions** (RFC-0028: push audio, pull transcript parts,
  retryable timeouts).
- **Observability built in** (new in 0.3.0) — record every request/response
  pair to JSONL and replay it offline (RFC-0023), group calls into sessions
  (RFC-0024), and detect provider prompt-cache hits (RFC-0015).
- **Composite models** (new in 0.3.0) — `RouterModel` routes each call with
  fallback (RFC-0021); `MoaModel` aggregates parallel reference models
  mixture-of-agents style (RFC-0022). Both are plain `LanguageModel`s.
- **Config-driven provider registry** — `provider-registry.json` describes
  each of the 251 OpenAI-compatible providers (base URL, env var, profile
  quirks: top_k, tools, response_format, streaming usage, max_tokens key);
  one unified `provider(name, ...)` entry in every binding.
- **Fast and small** — Rust core, release profile tuned for binary size
  (`lto`, `codegen-units=1`, `panic="abort"`, `strip`, `opt-level="z"`).
- **8 language bindings** from one core: Node, Python, Swift, Kotlin, Flutter,
  Go, Java, C — plus `aimux-web`, a browser console for model-call testing
  and trace visualization (RFC-0029).
- **Hermetic tests** — 2,800+ cassettes replay real API responses; no network
  or API keys required.

## Performance

Benchmarked against the official OpenAI SDKs on the same machine, same mock
server, same abstraction layer (HTTP + JSON, no orchestration). Full results
and methodology in [docs/PERF-RESULTS.md](docs/PERF-RESULTS.md).

| | aimux | OpenAI SDK | aimux faster |
|---|---|---|---|
| **Node.js** (single req) | 0.101 ms | 1.488 ms | **14.7×** |
| **Python** (single req) | 0.080 ms | 0.595 ms | **7.5×** |

### Sustained stress (2000 requests, 200 KB context, 50 KB response)

| | aimux rps | SDK rps | aimux P99 | SDK P99 | RSS growth |
|---|---|---|---|---|---|
| **Node.js** (32 cores) | 1512 | 563¹ | 1.92 ms | 3.96 ms | +23 MB vs +103 MB |
| **Python** | 1393 | 987 | 0.94 ms | 1.37 ms | **+0 MB** vs +8 MB |

¹ vs Vercel AI SDK (not apples-to-apples — AISDK adds Zod validation,
middleware, and telemetry per request).

### Why it's fast

- **Rust core** — `reqwest` connection pool, no GC, no runtime pauses.
- **Zero memory growth** — Python aimux RSS did not grow a single byte across
  2000 requests; Node grew only 2 MB.
- **Stable tail latency** — no GC pauses means P99 stays flat even under CPU
  contention; the JS SDK's P99 spikes to 12.87 ms on a single core.
- **C ABI overhead is low** — serialization is ~50% of overhead only on large
  payloads; in real LLM requests (3–10 s) it is <0.1%.

## Architecture

```
aimux/
├── aimux-core            # Core abstractions: LanguageModel / Provider / Message / StreamPart
├── aimux-providers       # 329 provider implementations (251 registry-backed + native)
├── aimux-stream          # SSE / NDJSON stream parsing
├── aimux-provider-utils  # HTTP utilities: retry, backoff, error parsing, API-key loading
├── aimux-ffi             # C ABI (opaque handles + JSON results + owned aimux_error_t *) for non-native bindings
└── tools/                # aimux-cli (cache probe) · aimux-replay · aimux-web (console)
```

```
           ┌─ native path ──→ aimux-core + aimux-providers (direct Rust types + async)
bindings ──┤
           └─ C ABI path  ──→ aimux-ffi (handles + JSON results + owned errors)
```

## Installation

**Rust** (the core library):

```bash
cargo add aimux-core aimux-providers
```

| Crate | Description | crates.io |
|-------|-------------|-----------|
| `aimux-core` | Core abstractions: `LanguageModel` / `Provider` / `Message` / `StreamPart` | [crates.io](https://crates.io/crates/aimux-core) |
| `aimux-providers` | 325 provider implementations | [crates.io](https://crates.io/crates/aimux-providers) |
| `aimux-stream` | SSE / NDJSON stream parsing | [crates.io](https://crates.io/crates/aimux-stream) |
| `aimux-provider-utils` | HTTP utilities: retry, backoff, error parsing | [crates.io](https://crates.io/crates/aimux-provider-utils) |
| `aimux-ffi` | C ABI for non-native bindings | [crates.io](https://crates.io/crates/aimux-ffi) |

**Node.js**:

```bash
npm install @arcships/aimux
```

```typescript
import { openai, generateText } from '@arcships/aimux'

const model = await openai(process.env.OPENAI_API_KEY!, 'gpt-4o')
const result = await generateText(model, 'Explain Rust ownership in one sentence.')
console.log(result.text)
```

> The package ships a typed wrapper (`generateText` / `streamText`) on top of
> the raw napi API. Need the raw JSON-string interface? Use
> `import { openai } from '@arcships/aimux/raw'`.

## Quick start

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIConfig, OpenAIProvider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = OpenAIProvider::new(
        OpenAIConfig::new(std::env::var("OPENAI_API_KEY")?)
    );
    let model = provider.model("gpt-4o");

    let result = generate_text(
        &model,
        "Explain Rust ownership in one sentence.",
        GenerateTextOptions::default(),
    ).await?;

    println!("{}", result.text);
    Ok(())
}
```

## Streaming

```rust
use futures::StreamExt;

let result = stream_text(
    &model,
    "Write a haiku about Rust.",
    GenerateTextOptions::default(),
).await?;

let mut stream = result.stream;
while let Some(part) = stream.next().await {
    match part? {
        StreamPart::TextDelta { delta, .. } => print!("{}", delta),
        StreamPart::Finish { .. } => println!("\n[done]"),
        _ => {}
    }
}
```

## Record & replay (new in 0.3.0)

Every request/response pair can be captured to JSONL and replayed offline —
regression tests, incident forensics, and CI without network or keys
([RFC-0023](rfc/0023-runtime-request-recording.md)):

```rust
use aimux_core::recording::{init_recording, JsonlRecorder};
use std::sync::Arc;

// One line turns on recording for the process (every binding has the same
// switch; secrets are redacted before hitting disk).
init_recording(Some(Arc::new(JsonlRecorder::new("./recordings"))));

let result = generate_text(&model, "Explain Rust ownership.", Default::default()).await?;
// → ./recordings/recordings.jsonl now holds the full request + response,
//   replayable offline with the `aimux-replay` CLI.
```

Related: sessions group calls with `session_id` ([RFC-0024](rfc/0024-session-aggregation.md)),
and cache-hit tracing detects prompt-cache reuse from provider headers
([RFC-0015](rfc/0015-cache-trace-audit.md), surfaced by `aimux-cli`).

## Composite models: router & MoA (new in 0.3.0)

```rust
use aimux_core::composite::ChildModel;
use aimux_core::moa::{MoaConfig, MoaModel};
use aimux_core::router::{FallbackPolicy, RouterConfig, RouterModel, RuleRouter};

// Router: pick one model per call, fall back on failure (RFC-0021).
let children: Vec<ChildModel> = vec![
    Arc::new(provider.model("gpt-4o")) as ChildModel,
    Arc::new(provider.model("gpt-4o-mini")) as ChildModel,
];
let routed = RouterModel::new(
    children.clone(),
    Box::new(RuleRouter),
    FallbackPolicy::OnError,
    RouterConfig::default(),
);

// MoA: fan out to references, aggregate their answers (RFC-0022).
let moa = MoaModel::new(
    children,
    Arc::new(provider.model("gpt-4o")) as ChildModel,
    MoaConfig::default(),
);

// Both are plain LanguageModel — every API and binding works unchanged.
let result = generate_text(&routed, "…", Default::default()).await?;
```

## Realtime transcription sessions (new in 0.3.0)

WebSocket-based streaming transcription with push-audio / next-part /
retryable timeouts, in every binding ([RFC-0028](rfc/0028-transcription-streaming.md)).
TypeScript:

```typescript
import { openaiTranscription, startTranscriptionSession } from '@arcships/aimux/raw'

const model = await openaiTranscription(process.env.OPENAI_API_KEY!, 'gpt-realtime-whisper')
const session = await startTranscriptionSession(model, null)

session.pushAudio(pcmChunk) // push audio as it arrives

for (;;) {
  try {
    const raw = await session.nextPart(500)
    if (raw === null) break                       // stream ended
    const part = JSON.parse(raw)
    if ('TranscriptDelta' in part) process.stdout.write(part.TranscriptDelta.delta)
  } catch (e: any) {
    if (e.retryable === false && e.message.includes('timeout')) continue
    throw e                                       // real failure — typed error
  }
}
session.close()
```

## Switch providers

```rust
// OpenAI → DeepSeek: only the provider name changes (registry-backed;
// key read from the provider's env var)
use aimux_providers::{provider, provider_from_env, ProviderName};

// 推荐:类型化 ProviderName(IDE 补全 + 编译期检查)
let model = provider(ProviderName::Deepseek, None, "deepseek-chat", None)?;
// 字符串形式同样可用:
let model = provider_from_env("deepseek", "deepseek-chat", None)?;
// model usage is identical — it's all dyn LanguageModel
```

All 251 OpenAI-compatible providers are registry-backed: `provider(name, ...)`
in every binding, with typed `ProviderName` (enum/union/consts per language).
The retired per-provider shell types (`XxxConfig`/`XxxProvider`) are gone —
see [docs/API.md](docs/API.md#providers).

> **Scope:** OpenAI-compatible → `provider(name, ...)`; others (Anthropic,
> multimodal, local…) → their constructors. List:
> [docs/api/providers.md](docs/api/providers.md).

## Provider coverage

| Type | Count | Examples |
|------|:-----:|----------|
| Native protocol | 10 | OpenAI, Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI, Anthropic-AWS |
| OpenAI-compatible (registry) | 251 | Groq, Fireworks, Together, Perplexity, Ollama Cloud, DeepSeek, Alibaba Tongyi, Zhipu, Baidu, Tencent, Moonshot, SiliconFlow… |
| OpenAI-compatible (standalone + Vertex-hosted) | 35 | OpenRouter, Hugging Face, Ollama, vLLM, SGLang, Llama.cpp, LiteLLM Proxy, Vertex-hosted DeepSeek/Qwen/Llama… |
| Speech / transcription | 10 | ElevenLabs, Deepgram, AssemblyAI, AWS Polly, Cartesia, Hume, Gladia, RevAI, LMNT, Fal |
| Image / video | 8 | Black Forest Labs, Replicate, Luma, Prodia, KlingAI, Recraft, Stability, RunwayML |
| Embeddings / rerank / search | 13 | Voyage, Jina, Tavily, Exa, Firecrawl, Serper, SearXNG, You.com… |
| Other (Responses API, Bedrock/Mantle) | 2 | generic Responses API wrapper, Bedrock Mantle |

Full list: [rfc/0004-provider-inventory.md](rfc/0004-provider-inventory.md).

## Language bindings

aimux ships 8 bindings that share the same Rust core:

| Binding | Path | Tool | Get it | Native library |
|---------|------|------|--------|---------------|
| **Node.js** | native | napi-rs v3 | `npm install @arcships/aimux` — [npm](https://www.npmjs.com/package/@arcships/aimux) | bundled in the package, nothing to do |
| **Python** | native | PyO3 + maturin | `pip install arcships-aimux` — [PyPI](https://pypi.org/project/arcships-aimux/) | bundled in the wheel, nothing to do |
| **Go** | C ABI | cgo (static link) | `go get github.com/arcships/aimux/bindings/go` then `go generate` | auto-downloaded `.a` from [GitHub Releases](https://github.com/arcships/aimux/releases) |
| **Swift** | C ABI | SPM | SPM: `https://github.com/arcships/aimux` (`from: "0.3.0"`) | `libaimux_ffi.dylib` — see [guide](docs/api/swift.md#install) |
| **Kotlin** | C ABI | JNA | `ai.arcships:aimux-kotlin` — Maven Central · **requires JDK 17+** (0.3.0) | `libaimux_ffi.so/.dylib` or `aimux_ffi.dll` on the JNA search path — see [guide](docs/api/kotlin.md#install) |
| **Java** | C ABI | JNA | `ai.arcships:aimux-java` — Maven Central | same as Kotlin — see [guide](docs/api/java.md#install) |
| **Flutter** | C ABI | dart:ffi | `aimux` on pub.dev — Flutter plugin, native core embedded (publisher `arcships.ai`) | iOS/Android 开箱即用，桌面端开发/测试 — see [guide](docs/api/flutter.md#install) |
| **C / C++** | C ABI | direct link | `.so`/`.dylib`/`.dll` + `aimux-ffi.h` from [GitHub Releases](https://github.com/arcships/aimux/releases) | link against it — see [guide](docs/api/c.md#install) |

See [bindings/README.md](bindings/README.md) and the [API docs](docs/API.md).

## Testing

```bash
cargo test -p aimux-providers --tests
```

Tests run on cassette playback — no network and no keys. See
[rfc/0003-test-cassette.md](rfc/0003-test-cassette.md).

## Documentation

| Doc | Contents |
|-----|----------|
| [docs/API.md](docs/API.md) | **API overview** — shared reference + links to per-language guides |
| [docs/api/reference.md](docs/api/reference.md) | **API reference** — public types & functions lookup |
| [docs/api/providers.md](docs/api/providers.md) | **Provider list** — all 325 providers with entry points (generated) |
| [docs/api/](docs/api/) | **Per-language API guides** — Node.js, Python, Rust, Go, C/C++, Swift, Kotlin, Flutter |
| [docs/error-model.md](docs/error-model.md) | **错误模型** — 跨语言错误形态与兼容性约定 |
| [docs/PROJECT-OVERVIEW.md](docs/PROJECT-OVERVIEW.md) | Project overview, design decisions, benchmarks |
| [docs/PERF-RESULTS.md](docs/PERF-RESULTS.md) | Performance benchmark results |
| [docs/aimux-vs-aisdk-node.md](docs/aimux-vs-aisdk-node.md) | Node.js DX comparison vs Vercel AI SDK |
| [docs/README.md](docs/README.md) | Documentation index |

### Design docs (RFCs)

| RFC | Contents |
|-----|----------|
| [0001](rfc/0001-multilang-bindings.md) | Multi-language bindings (Node/Swift/Kotlin/Flutter/Python) |
| [0002](rfc/0002-provider-improvements.md) | Config descriptor & thin-wrapper improvements |
| [0003](rfc/0003-test-cassette.md) | Test cassette scheme |
| [0004](rfc/0004-provider-inventory.md) | Full provider inventory & implementation status |
| [0005](rfc/0005-protocol-conversion.md) | Protocol conversion & adaptation layer |
| [0006](rfc/0006-provider-development.md) | Provider minimum acceptance, core contract, tests |
| [0007](rfc/0007-search-model-trait.md) | Search model trait |
| [0008](rfc/0008-multimodal-bindings.md) | Multimodal bindings design |
| [0009](rfc/0009-request-resilience.md) | Request resilience (shared client / jitter / timeout) |
| [0010](rfc/0010-perf-benchmark-vs-aisdk.md) | Performance vs Vercel AI SDK benchmark |
| [0011](rfc/0011-golang-bindings.md) | Go bindings (cgo static link + push callback → channel) |
| [0012](rfc/0012-source-dedup.md) | Source dedup (product source −25%) |
| [0013](rfc/0013-java-bindings.md) | Java bindings (JNA + raw/typed two-layer API) |
| [0016](rfc/0016-align-with-aisdk.md) | Align with Vercel AI SDK (capability gaps) |
| [0017](rfc/0017-provider-config-dx.md) | Unified provider config & request body overrides (DX) |
| [0018](rfc/0018-codex-subscription.md) | Codex subscription channel provider (evaluation) |
| [0019](rfc/0019-session-affinity.md) | Session affinity lightweight support |
| [0014](rfc/0014-logging.md) | Logging (`AIMUX_LOG` controls) |
| [0015](rfc/0015-cache-trace-audit.md) | Cache-hit tracing & audit (TraceLayer / verdict evaluator) |
| [0020](rfc/0020-external-provider-config.md) | External OpenAI-compatible provider config (runtime registry overrides) |
| [0021](rfc/0021-composite-model-routing.md) | RouterModel — composite model routing with fallback |
| [0022](rfc/0022-moa-single-fanout.md) | MoaModel — single-fanout mixture-of-agents |
| [0023](rfc/0023-runtime-request-recording.md) | Request recording & replay (JSONL, ring, redaction) |
| [0024](rfc/0024-session-aggregation.md) | Session grouping & query APIs |
| [0025](rfc/0025-aimux-cli-cache-probe.md) | `aimux-cli` cache-probe client |
| [0026](rfc/0026-openai-compatible-output.md) | OpenAI-compatible output format |
| [0027](rfc/0027-model-catalogue-and-list-api.md) | Model catalogue & `list_models` |
| [0028](rfc/0028-transcription-streaming.md) | Realtime transcription streaming (WS sessions) |
| [0029](rfc/0029-web-console.md) | `aimux-web` browser console |
| [0033](rfc/0033-code-convergence-plan.md) | Code convergence plan (providers / FFI / bindings / recording / errors / tests / docs) |

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for the
development setup, testing workflow, provider/binding conventions, and the pull
request process. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE)
