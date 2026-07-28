# ollama cassettes — 待实现

这 21 个录像来自 rig (MIT)，记录的是 **Ollama 原生 API**（`/api/chat`、`/api/tags`），
不是 OpenAI 兼容接口。

## 为什么没有挂载回放测试

`aimux-providers/src/` 下**没有 ollama provider 实现**。Ollama 原生 API 的请求/响应
格式与 OpenAI Chat Completions 完全不同：

- 路径是 `/api/chat`（非 `/v1/chat/completions`）
- 请求体用 `messages` / `model` / `options` / `think` / `stream` 字段
- 响应体结构独立，流式是逐行 JSON（NDJSON），不是 SSE

要回放这些录像，需要先实现一个独立的 `OllamaProvider`，不能复用 `OpenAIProvider`。

## 录像内容

| 路径 | 数量 | 覆盖 |
|------|------|------|
| `/api/chat` | 20 | 流式/非流式 completion、结构化输出、工具调用、thinking |
| `/api/tags` | 1 | list models |

## 后续

实现 `OllamaProvider` 后，在 `conformance_test.rs` 加 `mod ollama_conformance`，
照 bedrock 的模式挂载即可。
