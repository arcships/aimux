# Contributing to aimux

Thanks for your interest in contributing to aimux! This document explains how
to set up a development environment, run the tests, and submit changes.

## Project layout

```
aimux/
├── aimux-core/            # Core abstractions: LanguageModel / Provider / Message / StreamPart
├── aimux-providers/       # 325 provider implementations + cassettes
├── aimux-stream/          # SSE / NDJSON stream parsing
├── aimux-provider-utils/  # HTTP utilities: retry, backoff, error parsing, API-key loading
├── aimux-ffi/             # C ABI (opaque handle + JSON + push callback) for non-native bindings
├── bindings/              # Node, Python, Swift, Kotlin, Flutter, Go, C — share one Rust core
├── contract-tests/        # Shared JSON fixtures exercised across languages
├── rfc/                   # Design docs (RFCs)
├── docs/                  # Public product docs (see docs/README.md)
└── scripts/               # Code-generation and maintenance scripts
```

`docs/internal/` contains historical research and audit notes; it is not part
of the public surface and generally does not need changes.

## Prerequisites

- **Rust**: stable toolchain (see `rust-toolchain.toml`). The workspace targets
  edition 2024 and MSRV 1.85.
- **Node.js** 20+ (for the Node binding, `bindings/node`).
- **Python** 3.11+ with `maturin` and `pytest` (for the Python binding).
- For C-ABI bindings (Swift/Kotlin/Flutter/Go/C): build `aimux-ffi` first
  (`cargo build -p aimux-ffi --release`), then follow `bindings/README.md`.

## Building

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Testing

Tests run entirely on **cassette playback** — no network access and no API keys
are required.

```bash
# Whole workspace
cargo test --workspace

# A single crate
cargo test -p aimux-providers --tests

# Contract tests (cross-language shared fixtures)
cargo test --test contract_test -p aimux-core
```

If you add or change a provider's request/response shapes, record a new
cassette rather than mocking inline. See `rfc/0003-test-cassette.md` for the
recording workflow.

## Adding a provider

aimux distinguishes three kinds of providers:

1. **Native protocol** — providers with their own request/response model
   (OpenAI, Anthropic, Google, Bedrock, …). Implement the full `convert`/model
   path and handle provider-specific differences.
2. **OpenAI-compatible thin wrapper** — the majority. Describe differences via
   `OpenAICompatProfile` (top_k, tools, response_format, streaming usage,
   request-body post-processing) so the thin wrapper does not erase
   provider-specific behavior.
3. **Modality-specific** — speech, image, video, transcription, etc.

Before submitting a provider, read `rfc/0006-provider-development.md` for the
minimum acceptance criteria, core contracts, and required tests.

## Adding a binding

All bindings share the same Rust core via two paths:

- **Native path** (Node, Python): direct mapping of Rust types + async.
- **C ABI path** (Swift, Kotlin, Flutter, Go, C): through `aimux-ffi` using an
  opaque handle + JSON boundary + push callback.

See `rfc/0001-multilang-bindings.md` and `bindings/README.md`.

## Commit messages

This project uses [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Mistral image generation
fix: correct Groq streaming usage passthrough
refactor: deduplicate OpenAI-compatible request builders
docs: translate RFC-0004 to English
test: add cassette for DeepSeek tool-call
```

Keep the subject line imperative and ≤ 72 characters.

## Pull requests

1. Fork the repo and create a branch from `master`.
2. Make your change with focused commits.
3. Ensure `cargo fmt`, `cargo clippy -D warnings`, and `cargo test --workspace`
   all pass locally. CI runs the same.
4. If you add a provider or binding, include cassette tests or contract
   fixtures as appropriate.
5. Update the relevant doc (README, `docs/`, or `rfc/`) if your change affects
   the public surface.
6. Open a PR against `master` and fill in the template.

## RFCs

Non-trivial design changes go through an RFC in `rfc/`. Copy the highest-numbered
existing RFC as a template, increment the number, and open a PR titled
`RFC-NNNN: <title>`. RFCs are discussed in PR review.

## License

By contributing, you agree that your contributions will be licensed under the
MIT License that covers the project.
