# aimux documentation

Public documentation for aimux (arcships) — an in-process embedded SDK for LLM provider access, written in Rust. Not a proxy gateway.

## Guides & references

| Doc | Contents |
|-----|----------|
| [../README.zh-CN.md](../README.zh-CN.md) | **简体中文 README** |
| [../FAQ.md](../FAQ.md) | What aimux is, proxy gateways, languages, tool calling, license, versioning |
| [../examples/](../examples/) | Runnable Rust examples (chat, streaming, switching providers) |
| [../llms.txt](../llms.txt) | Short summary for AI answer engines |
| [upgrading-to-0.6.md](upgrading-to-0.6.md) | **0.6 migration guide** — breaking changes by language and the upgrade checklist |
| [API.md](API.md) | **API overview** — architecture, language guides, providers, operations, errors |
| [api/reference.md](api/reference.md) | **API reference index** — generated DTOs, core types, ops and error sources |
| [api/ops.md](api/ops.md) | **Shared operations** — operation names and families |
| [api/providers.md](api/providers.md) | **Provider list** — every provider with entry points and the canonical counts (generated) |
| [api/gaps.md](api/gaps.md) | **Archived** — historical 0.5 binding gap list; current capabilities live in each binding README and `runtime.describe` |
| [api/](api/) | **Per-language guides** — Node.js, Python, Rust, Go, C/C++, Swift, Kotlin, Flutter, Java |
| [error-model.md](error-model.md) | **错误模型** — 错误域、跨语言映射、cause 链与兼容性约定 |
| [PROJECT-OVERVIEW.md](PROJECT-OVERVIEW.md) | Project overview, design decisions, and benchmark summary |
| [research/decision-api/](research/decision-api/) | Decision model/API 调研：OpenAI、TypeSafe/Jev、vLLM、SGLang 接口材料与 aimux 设计建议 |
| [api/decision.md](api/decision.md) | Decision API（AI SDK `DecisionModelV4` 与 `decide`）：typed questions、概率与各 provider adapter |
| [PERF-RESULTS.md](PERF-RESULTS.md) | Performance benchmark results (aimux vs OpenAI SDK / Vercel AI SDK) |
| [aimux-vs-aisdk-node.md](aimux-vs-aisdk-node.md) | Historical Node.js developer-experience comparison vs Vercel AI SDK (0.5 era) |
| [ai-sdk-request-pipeline.md](ai-sdk-request-pipeline.md) | AI SDK-aligned request pipeline — operation retry, response handlers, timeout/abort design |
| [aisdk-architecture-alignment.md](aisdk-architecture-alignment.md) | Full-chain AI SDK alignment design (provider → model, call runtime, message protocol); implementation design under RFC-0036 (Chinese) |
| [aisdk-architecture-impact-map.md](aisdk-architecture-impact-map.md) | Appendix: repository-wide impact map behind the alignment design (Chinese) |

For the project README, quickstart, and provider/binding tables, see the
[top-level README](../README.md).

## Design docs (RFCs)

Design decisions are recorded as RFCs under [`../rfc/`](../rfc/). See the
README for the full list.

## Internal notes

`internal/` contains historical research, audit, and handoff notes that are
not part of the public surface. It is kept for reference only.
