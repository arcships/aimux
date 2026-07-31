# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Rewrote the top-level `README.md` in English with badges, architecture
  overview, and curated quickstart.
- Translated the public docs (`docs/`, `bindings/README.md`) and all RFCs to
  English.
- Moved internal research, audit, and handoff notes into `docs/internal/`.
- Removed committed Windows build artifacts (`.exe`/`.pdb`) and gitignored them.

### Fixed
- Corrected the `repository` URL in `Cargo.toml` (`yourusername` → `arcships`).
- Fixed the CI workflow trigger branch (`main` → `master`) so CI now runs on
  the actual default branch.

### Added
- `LICENSE` (MIT), `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`.
- GitHub issue templates and a pull request template.

## [0.1.0] - 2026-07-31

### Added
- Core abstractions: `LanguageModel` trait (object-safe, `Box<dyn>` across
  providers), `Provider`, `Message`, `StreamPart`.
- 172 provider modules: 11 native protocol implementations (OpenAI, Anthropic,
  Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI, DeepSeek,
  Anthropic-AWS) + 145 OpenAI-compatible thin wrappers + 15 modality-specific
  + 1 generic Responses API wrapper.
- 8 modality traits: text, embedding, image, video, speech, transcription,
  reranking, search.
- `OpenAICompatProfile` descriptor capturing per-provider differences
  (top_k, tools, response_format, streaming usage, request-body post-processing).
- Streaming via SSE / NDJSON parsing with safe cancellation (`AbortSignal`).
- Request resilience: shared HTTP client, Full-Jitter backoff, timeout, error
  mapping with `error_type` + `status_code` passthrough.
- 2,650 cassette tests replaying real API responses — no network or keys needed.
- 7 language bindings sharing one Rust core:
  - Node.js (native, napi-rs v3)
  - Python (native, PyO3 + maturin)
  - Swift (C ABI, Swift Package)
  - Kotlin (C ABI, JNA)
  - Flutter (C ABI, dart:ffi)
  - Go (C ABI, cgo, static link, single binary)
  - C / C++ (C ABI, direct link)
- TypeScript type definitions auto-generated from Rust via `ts-rs` (79 types).
- Release profile optimized for binary size (`lto`, `codegen-units=1`,
  `panic="abort"`, `strip`, `opt-level="z"`).

### RFCs
- RFC-0001 Multi-language bindings
- RFC-0002 Provider improvements (config descriptor + thin wrappers)
- RFC-0003 Test cassette scheme
- RFC-0004 Provider inventory (172 providers)
- RFC-0005 Protocol conversion & adaptation layer
- RFC-0006 Provider development: minimum acceptance, core contract, tests
- RFC-0007 Search model trait
- RFC-0008 Multimodal bindings
- RFC-0009 Request resilience (shared client / jitter / timeout)
- RFC-0010 Performance benchmark vs Vercel AI SDK
- RFC-0011 Go bindings (cgo static link + push callback → channel streaming)
- RFC-0012 Source dedup (product source 68K → 51K lines, −25%)

[Unreleased]: https://github.com/arcships/aimux/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/arcships/aimux/releases/tag/v0.1.0
