# aimux

> **A unified LLM access layer written in Rust. One API for 172+ AI providers.**

[![CI](https://github.com/arcships/aimux/actions/workflows/ci.yml/badge.svg)](https://github.com/arcships/aimux/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Providers](https://img.shields.io/badge/providers-172%2B-green.svg)](rfc/0004-provider-inventory.md)
[![Bindings](https://img.shields.io/badge/bindings-7-9cf.svg)](bindings/)

aimux is a Rust implementation of a unified LLM provider access layer. It
collapses the HTTP APIs of every AI provider into a single
`dyn LanguageModel` interface that anything upstream can call.

Unlike **rig** or **langchain**, aimux does **not** build agent loops, RAG, or
orchestration — it focuses exclusively on unifying service access. That is the
difference: aimux is an access layer, those are orchestration layers.

---

## Why aimux

- **172 provider modules** — 11 native protocol implementations
  (OpenAI, Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI,
  DeepSeek, Anthropic-AWS) + 145 OpenAI-compatible thin wrappers + 15
  modality-specific (speech/image/video) + 1 generic Responses API wrapper.
- **Unified, object-safe interface** — the `LanguageModel` trait supports
  `Box<dyn>` so providers are interchangeable without changing call sites.
- **Full multimodal** — text, streaming, tool calling, embeddings, image,
  speech, transcription, video, reranking, files.
- **Config-driven thin wrappers** — `OpenAICompatProfile` describes each
  provider's quirks (top_k, tools, response_format, streaming usage, request
  body post-processing), so thin wrappers never erase provider differences.
- **Fast and small** — Rust core, release profile tuned for binary size
  (`lto`, `codegen-units=1`, `panic="abort"`, `strip`, `opt-level="z"`).
- **7 language bindings** from one core: Node, Python, Swift, Kotlin, Flutter,
  Go, C.
- **Hermetic tests** — 2,650+ cassettes replay real API responses; no network
  or API keys required.

## Architecture

```
aimux/
├── aimux-core            # Core abstractions: LanguageModel / Provider / Message / StreamPart
├── aimux-providers       # 172 provider implementations
├── aimux-stream          # SSE / NDJSON stream parsing
├── aimux-provider-utils  # HTTP utilities: retry, backoff, error parsing, API-key loading
└── aimux-ffi             # C ABI (opaque handle + JSON + push callback) for non-native bindings
```

```
           ┌─ native path ──→ aimux-core + aimux-providers (direct Rust types + async)
bindings ──┤
           └─ C ABI path  ──→ aimux-ffi (opaque handle + JSON + push callback)
```

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

## Switch providers

```rust
// OpenAI → DeepSeek: only the provider changes
let provider = DeepSeekProvider::new(
    DeepSeekConfig::from_env()?
);
let model = provider.model("deepseek-chat");
// model usage is identical — it's all dyn LanguageModel
```

## Provider coverage

| Type | Count | Examples |
|------|:-----:|----------|
| Native protocol | 11 | OpenAI, Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI, DeepSeek |
| OpenAI-compatible | 145 | Groq, Fireworks, Together, Perplexity, Ollama, OpenRouter, Alibaba Tongyi, Zhipu, Baidu, Tencent, iFlytek, Moonshot, SiliconFlow… |
| Speech / transcription | 7 | ElevenLabs, Deepgram, AssemblyAI, Cartesia… |
| Image / video | 8 | Black Forest Labs, Replicate, Fal, KlingAI… |

Full list: [rfc/0004-provider-inventory.md](rfc/0004-provider-inventory.md).

## Language bindings

aimux ships 7 bindings that share the same Rust core:

| Binding | Path | Tool | Directory |
|---------|------|------|-----------|
| **Node.js** | native | napi-rs v3 | [bindings/node/](bindings/node/) |
| **Python** | native | PyO3 + maturin | [bindings/python/](bindings/python/) |
| **Swift** | C ABI | Swift Package | [bindings/swift/](bindings/swift/) |
| **Kotlin** | C ABI | JNA | [bindings/kotlin/](bindings/kotlin/) |
| **Flutter** | C ABI | dart:ffi | [bindings/flutter/](bindings/flutter/) |
| **Go** | C ABI | cgo (static link, single binary) | [bindings/go/](bindings/go/) |
| **C / C++** | C ABI | direct link | [bindings/c/](bindings/c/) |

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
| [docs/API.md](docs/API.md) | **API reference** (all modalities × all languages) |
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

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for the
development setup, testing workflow, provider/binding conventions, and the pull
request process. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE)
