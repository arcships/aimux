# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.5] - 2026-08-01

### Added
- **Six desktop native targets for Node binding** — expanded from 2 to 6
  platform-specific Node-API packages: Windows x64/ARM64 (MSVC), macOS
  x64/ARM64, GNU/Linux x64/ARM64. The root `@arcships/aimux` package
  auto-selects the matching platform package at install time. Targets
  Node-API 8 (compatible with Node.js and Electron without rebuild).
  - Linux built against glibc 2.17 baseline (napi-cross).
  - Windows statically links MSVC CRT (no runtime dependency).
  - macOS deployment target set to 10.13.

### Changed
- CI/release matrices updated for all six Node binding targets.
- Node.js 24 + `npm ci` in binding workflows.
- `package-lock.json` tracked, `@napi-rs/cli` pinned to 3.8.2.
- AVA worker threads disabled (avoids napi-rs/Tokio teardown panics).

## [0.1.3] - 2026-08-01

### Added
- **`bodyOverrides` (JSON deep-merge)** — per-call and provider-level request
  body overrides. Objects merge recursively, scalars overwrite, `null` deletes
  keys. Applied after built-in vendor overrides; per-call overrides
  provider-level. Lets users inject vendor-specific fields (e.g.
  `enable_thinking`, `thinking_budget`) without closure bridging — critical
  for aimux's multi-language C ABI architecture where closures can't cross the
  JSON string boundary. (RFC-0017)
- **`maxRetries` (per-call)** — override the provider's retry count. `Some(0)`
  disables retries. Available on both `GenerateTextOptions` (per-call) and
  provider factory config (provider-level, Node only).
- **Provider factory config object (Node)** — `openai()`/`anthropic()`/
  `deepseek()` 3rd param now accepts `string | ProviderConfig` (backward
  compatible). `ProviderConfig` exposes `baseUrl`, `headers`, `organization`,
  `project`, `maxRetries`, `bodyOverrides`.
- **All 7 language typed wrappers** now expose `body_overrides` + `max_retries`
  on `GenerateTextOptions`: Node, Python, Go, Java, Kotlin, Swift, Flutter.
- RFC-0016 (Vercel AI SDK gap analysis) and RFC-0017 (provider config DX
  design).

### Fixed
- **`build_headers` now reads `config.headers` and `config.project`** —
  previously `OpenAIConfig.with_headers()` / `with_project()` set fields that
  `build_headers` silently ignored. Provider-level headers and project ID now
  reach the wire.
- **Anthropic factory now applies `bodyOverrides`** — was missing in the
  initial implementation (OpenAI/DeepSeek had it, Anthropic didn't).
- **`openai_compat` macro** — added `with_headers`/`with_retry_config`/
  `with_body_overrides` pass-through methods to all 251 OpenAI-compatible
  thin-wrapper providers (previously only `with_base_url` was exposed).

## [0.1.2] - 2026-08-01

### Fixed
- **`tool`-role `ContentPart[]` with the legacy `output` field is now accepted.**
  `ContentPart::ToolResult` renamed `output` → `result` in 0.1.1, but
  deserialization only accepted `result`, so multi-part `tool` messages built
  with `output` (the Vercel AI SDK / 0.1.0 TypeScript shape) were rejected with
  "data did not match any variant of untagged enum ModelPrompt". `result` now
  accepts `output` as a serde alias, so both shapes round-trip and
  `tool_call_id` reaches the OpenAI wire format. Serialization still emits
  `result`.
- **`reasoning` ContentPart is now replayed as `reasoning_content` on the
  request side.** Thinking models (e.g. DeepSeek `deepseek-v4-flash`) require
  prior assistant `reasoning_content` to be passed back on later turns,
  including tool-call turns; the OpenAI message converter previously dropped
  `ContentPart::Reasoning` parts, producing "The `reasoning_content` in the
  thinking mode must be passed back to the API." Reasoning parts are now lifted
  to a top-level `reasoning_content` string on assistant messages (mirroring
  the Vercel AI SDK `openai-compatible` assistant conversion), for both the
  tool-call and text paths. Groq's `reasoning` field name is unchanged.
- Regenerated the stale TypeScript bindings (`ToolResult.ts`, `ContentPart.ts`,
  `GenerateContent.ts`, `StreamPart.ts`, `ToolCall.ts`, `Usage.ts`) so the npm
  copy matches the Rust source of truth (`result` field, added fields).
- Made the `release.yml` crates.io idempotency checks read the version
  dynamically from `Cargo.toml` instead of a hardcoded `0.1.0` (which would
  have skipped the 0.1.2 publish).

### Changed
- Rewrote the top-level `README.md` in English with badges, architecture
  overview, and curated quickstart.
- Translated the public docs (`docs/`, `bindings/README.md`) and all RFCs to
  English.
- Moved internal research, audit, and handoff notes into `docs/internal/`.
- Removed committed Windows build artifacts (`.exe`/`.pdb`) and gitignored them.

### Fixed (prior)
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
