# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] — pending (planned 2026-08-17)

**Breaking release.** 196 commits since 0.2.1: observability primitives
(recording / replay / sessions / tracing), composite models, streaming
transcription, a reworked error model across the C ABI and all eight
bindings, a browser console, and a large provider-correctness sweep.

### Breaking

**Rust (aimux-core / aimux-provider-utils)**

- `AiMuxError::RateLimited` gained `message: String` (now
  `RateLimited { retry_after_ms, message }`); `#[serde(default)]` keeps
  old payloads deserializable, but the generated TypeScript marks
  `message` required — update exhaustive destructuring.
- `GenerateTextOptions` / `CallOptions` / `StreamTextResult` gained public
  fields (`session_id`, recording/trace controls, stream-result metadata).
  Use struct-update (`..Default::default()`) instead of exhaustive
  initializers.
- `shared_client()` / `shared_streaming_client()` (aimux-provider-utils)
  now return `Result<&'static Client, AiMuxError>`: client-build failures
  (TLS backend, resource exhaustion) surface as a sticky, non-retryable
  `ApiCall` instead of aborting the host process (#147).
- Panicking convert wrappers are deprecated in favor of fallible variants.

**C ABI & the six FFI bindings (Kotlin / Java / Swift / Go / Flutter / C)**

- Error transport switched to an `AimuxError` out-parameter with typed
  code + HTTP status + retry hint; the JSON error envelope, the streaming
  `on_error` callback, and `aimux_last_error()` are removed.
- All six bindings restructured to the typed error model.
- **Kotlin**: `topK` is now `Double` (was `Long` — 40.5 truncated
  silently); published artifacts require **JDK 17** (`jvmToolchain(17)`).
- **Java**: `topK` likewise typed as double-valued (#106).
- `init_recording_ring(0)` now throws instead of a silent no-op
  (consistent across all seven languages).

**Python / Node (native bindings)**

- Streaming-transcription sessions surface in-stream errors by raising
  (`next_part`) / rejecting (`nextPart`) the typed hierarchy, and part
  payloads are no longer wrapped in a `{"Ok": ...}` envelope — both now
  match the C-ABI session shape and the other six bindings (#145/#150).
  Code that parsed the envelope manually must catch the exception.

### Added

- **Request recording & replay** (RFC-0023) — `Recorder` /
  `JsonlRecorder` plus a bounded in-memory `RingRecorder` with drop
  counting; layer-B HTTP choke-point recording (per-attempt, streaming,
  credential redaction); mock & request replay with matchers and the
  `aimux-replay` CLI; cross-binding exposure via C ABI, Python, Node, Go,
  Swift, Kotlin, Java, Flutter; `config_snapshot()` captures the minimal
  provider/model identity. `aimux_recording_try_flush` FFI export reports
  write failures (sticky first error) across the ABI (#133/#137).
- **Session grouping** (RFC-0024) — `session_id` groups related calls;
  `SessionStore` + `SessionInferer` with query APIs in all bindings;
  session-cache trajectory export.
- **Cache-hit tracing** (RFC-0015) — `TraceLayer`, verdict engine, and
  `RingTraceStore` detect prompt-cache hits from provider headers
  (vLLM / SGLang / LMCache behaviours), with cluster/route-aware gating;
  exposed in every binding.
- **`aimux-cli` cache-probe client** (RFC-0025) — offline / session /
  provider probing over the trace store.
- **OpenAI-compatible output** (RFC-0026) — `generateOpenAIOutput`
  across all eight bindings.
- **Model catalogue & listing** (RFC-0027) — `Provider::list_models` and
  `get_model_specs` with reasoning/capability metadata.
- **External provider config overlay** (RFC-0020) — register or override
  OpenAI-compatible providers from JSON at runtime.
- **Composite models** — drop-in `LanguageModel` wrappers, usable from every
  binding with zero call-site changes:
  - `RouterModel` (RFC-0021) routes each call to one child model through a
    pluggable `Router` strategy (built-ins: `RuleRouter`, `WeightedRouter`)
    with automatic fallback to the remaining children on failure.
  - `MoaModel` (RFC-0022) implements mixture-of-agents in a single call:
    reference models run in parallel, their outputs are spliced into an
    aggregator prompt, and the aggregated answer is returned — no agent
    loop involved.
- **Core API growth** — `streamText` aggregation, `generateObject`,
  top-level result aggregation (reasoning / sources / files /
  responseMessages), proxy configuration, `rawFinishReason`, logprobs,
  `usage.raw`, streaming warnings, `includeRawChunks`,
  `ResponseMetadata.timestamp`.
- **Streaming transcription** (RFC-0028) — realtime WebSocket sessions
  with push-audio / next-part / abort / first-chunk and idle timeouts,
  an FFI session API, and first-class support in all eight bindings.
- **`aimux-web` console** (RFC-0029) — browser-based model-call testing
  and trace visualization, shipped as release artifacts.
- **FFI/binding ergonomics** — default-capacity ring init, cancellable Go
  streams, `ProviderWithConfig` (Go), full `ProviderOptions` for
  `provider()` (Python), optional recording-ring capacity in every
  language.

### Fixed

**Provider correctness**

- Gemini: `functionResponse.name` now uses the tool name instead of the
  opaque call id (multi-turn tool calls no longer 400) (#127).
- Anthropic: in-stream errors now emit the terminal `Finish` part
  (contract parity with OpenAI/Google) (#128); assistant reasoning —
  including its signature — is echoed back when thinking is enabled
  (#131/#138).
- Bedrock & Anthropic: streaming no longer drops reasoning signatures;
  extended-thinking multi-turn keeps its context (#131).
- Vertex: grounding / url-context / code-execution / server-tool results
  are no longer silently dropped from streams; finish metadata restored
  (#141/#143).
- AWS SigV4 signs the host header with non-default ports; local gateways
  and proxied environments no longer fail (#125/#129).
- Six providers (openai/bedrock/cohere/mistral/huggingface/anthropic)
  stop discarding response fields they had already parsed (#101/#139).
- Recording: writer I/O failures (e.g. ENOSPC) surface through
  `try_flush` instead of a silent Ok, and the completion barrier requires
  the input record before finalizing (#110/#133).
- Structured `ApiCall`/`NoSuchProvider` fields with unified retry
  classification (#94); retry config honored for vertex/anthropic-aws;
  credential-source accuracy for xai/open-responses; real provider
  identity surfaced on the OpenAI chat path.
- Kotlin: the documented retryable timeout sentinel for `nextPart` is
  actually reachable (#116/#144); close-race read/write locks for handle
  types in Kotlin/Java/Flutter; Python exposes recording / mock-replay /
  `get_model_specs` with typed exceptions.

**Tests & fixtures**

- Cassette bodies are no longer blanked for every streaming response —
  157 recovered recordings across 640 files (#102).
- Contract fixtures now type-check (field *values*, not just names)
  across Rust and all eight bindings; the `top_k` drift that hid for
  months is regression-locked (#106).

### Engineering

- Quality gates: workspace fmt, clippy baseline plus a permanent
  5-lint subset (1,521 fixes, incl. 146 hand-written `# Errors` docs),
  `rustdoc -D warnings` in CI, ts-rs type-drift check, ProviderName
  generator-drift check.
- Coverage infrastructure (cargo-llvm-cov) with a 78.5% workspace
  baseline; unified e2e suite extended to six protocols; a 96-export FFI
  smoke harness; RFC-0028 error-path coverage in Rust, Python and Node.
- Round 4 quality audit: 16 verified findings, full reports under
  `docs/quality-audit/round4/`; unused-dependency sweep; rustdoc errors
  47 → 0.
- Release pipeline hardened from the 0.2.1 post-mortem (JVM Central
  Portal routing, napi rebuild before publish, Flutter xcframework
  embedding) plus a troubleshooting handbook.

### Removed

- `aimux_last_error()` (C ABI) — replaced by the `AimuxError` out-param.
- The C ABI JSON error envelope and the streaming `on_error` callback.

## [0.2.1] - 2026-08-04

Patch release following 0.2.0. First Maven Central (Java + Kotlin) and pub.dev
(Flutter) release; Rust / Node / Python re-released to stay aligned with the
new bindings.

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

[Unreleased]: https://github.com/arcships/aimux/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/arcships/aimux/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/arcships/aimux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/arcships/aimux/compare/v0.1.5...v0.2.0
[0.1.0]: https://github.com/arcships/aimux/releases/tag/v0.1.0
