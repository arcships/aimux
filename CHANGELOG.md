# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-08-03

**Breaking release.** This version replaces the 250 per-provider shell types
with a single registry-backed `provider(name, ...)` factory, and adds request
cancellation + timeout control. See [Removed](#removed) for migration.

### Added

- **Unified `provider(name, ...)` factory** (RFC-0017 phase 4) — every one of
  the 250 OpenAI-compatible providers is now described in one
  `provider_registry.json` (base URL, API-key env var, per-vendor quirks) and
  constructed through a single entry point in **every binding**: Rust, Node,
  Python, Go, Kotlin, Swift, Java, Flutter, C.

  ```rust
  // before: remember a class per provider
  // let model = GroqProvider::new(GroqConfig::new(key)).model("llama-3.3-70b");

  // after: one factory — typed name, explicit key
  let model = provider(ProviderName::Groq, Some(key), "llama-3.3-70b", None)?;
  // or plain string; key falls back to the provider's env var
  let model = provider_from_env("groq", "llama-3.3-70b", None)?;
  ```

  ```ts
  // Node: same factory, typed ProviderName (lowercase keys)
  const model = await provider(ProviderName.groq, apiKey, 'llama-3.3-70b')
  ```

- **Typed `ProviderName` in 8 languages** — enum/union/const in Rust,
  TypeScript, Python, Go, Java, Kotlin, Swift, Flutter. Gives autocomplete and
  compile-time checking; plain strings still work everywhere.
- **Request cancellation** — Node: pass a standard `AbortSignal` as the 4th
  argument of `generateText` / `streamText`; Rust: `abort_signal` on
  `GenerateTextOptions`. Aborting cancels the in-flight HTTP request.
- **Timeout control** — new `timeout` option on `GenerateTextOptions` /
  `CallOptions` (works in all bindings): `total_ms` (whole call),
  `first_chunk_ms` (time to first token), `chunk_ms` (idle gap between stream
  chunks). Timeouts surface as `AiMuxError::Timeout` and are not retried.
- **Reasoning effort passes through verbatim** — the old `minimal→low` /
  `xhigh→max` normalization is gone; all 7 levels are sent as documented (e.g.
  Groq's effort values). Setting `reasoning` without an effort value now
  produces a warning instead of silently doing nothing.
- **Native protocol constructors in every binding** — Bedrock, Vertex,
  Azure, Cohere, Mistral, xAI and Anthropic-AWS now ship LLM constructors
  across the C ABI and all 8 bindings (previously Rust-only). Python's
  `google()` factory is now exported.
- **Correct `max_tokens` field per vendor** — providers that expect
  `max_completion_tokens` (Groq, Heroku) or `max_tokens` (Perplexity,
  SiliconFlow, StepFun, …) get the right key automatically.
- New design docs: RFC-0014 (logging), RFC-0015 (cache-hit audit & request
  tracing), RFC-0018 (Codex subscription channel), RFC-0019 (session affinity).

### Removed (breaking)

- **250 per-provider shell types retired** (`GroqConfig`, `DeepSeekProvider`,
  …) in Rust and all bindings — migrate to `provider(name, ...)` /
  `ProviderName` (examples above). The 10 native protocol providers (OpenAI,
  Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI,
  Anthropic-AWS) keep their existing types; DeepSeek is registry-backed.
- **`RequestBodyOverride`** and the `request_body_override` profile field
  removed — use `body_overrides` instead.
- **Reasoning-effort normalization** removed (values now pass through).

### Fixed

- **Streaming timeouts could silently never fire** — a pending deadline was
  dropped before it could trigger; streaming now enforces `total_ms` /
  `first_chunk_ms` / `chunk_ms` reliably.
- **7 wrong `base_url` entries in the provider registry** corrected (they
  would have failed at request time).
- Provider factory missing from some bindings' public surface (Python
  exports, Node npm package, Go DeepSeek).
- CI/test hygiene: formatting drift, clippy warnings, and tests that no
  longer depend on ambient environment variables.

### Changed

- All provider tests now go through the unified `provider()` entry; new test
  coverage for timeouts, cancellation, and registry wiring.

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
