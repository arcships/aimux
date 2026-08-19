# aimux documentation

Public documentation for aimux — a unified LLM access layer written in Rust.

## Guides & references

| Doc | Contents |
|-----|----------|
| [API.md](API.md) | **API overview** — features, shared reference tables, factory functions, coverage matrix |
| [api/reference.md](api/reference.md) | **API reference** — all public types & functions, with sources |
| [api/providers.md](api/providers.md) | **Provider list** — all 325 providers with entry points (generated) |
| [api/gaps.md](api/gaps.md) | **Binding API gaps** — per-binding missing API tracking (Swift/Kotlin/Flutter multimodal, Go base_url, search factories, C ABI `_with_base`), with C ABI function mapping and reference implementations |
| [api/](api/) | **Per-language guides** — Node.js, Python, Rust, Go, C/C++, Swift, Kotlin, Flutter, Java |
| [error-model.md](error-model.md) | **错误模型** — 错误来源、跨语言映射、所有权与兼容性约定 |
| [PROJECT-OVERVIEW.md](PROJECT-OVERVIEW.md) | Project overview, design decisions, and benchmark summary |
| [PERF-RESULTS.md](PERF-RESULTS.md) | Performance benchmark results (aimux vs OpenAI SDK / Vercel AI SDK) |
| [aimux-vs-aisdk-node.md](aimux-vs-aisdk-node.md) | Node.js developer-experience comparison vs Vercel AI SDK |

For the project README, quickstart, and provider/binding tables, see the
[top-level README](../README.md).

## Design docs (RFCs)

Design decisions are recorded as RFCs under [`../rfc/`](../rfc/). See the
README for the full list.

## Internal notes

`internal/` contains historical research, audit, and handoff notes that are
not part of the public surface. It is kept for reference only.
