# ollama cassettes — to be implemented

These 21 recordings come from rig (MIT) and capture the **Ollama native API**
(`/api/chat`, `/api/tags`), not the OpenAI-compatible interface.

## Why no playback tests are mounted

There is **no ollama provider implementation** under `aimux-providers/src/`.
The Ollama native API request/response format is entirely different from
OpenAI Chat Completions:

- The path is `/api/chat` (not `/v1/chat/completions`).
- The request body uses `messages` / `model` / `options` / `think` / `stream`
  fields.
- The response body has its own structure; streaming is line-delimited JSON
  (NDJSON), not SSE.

To replay these recordings, a standalone `OllamaProvider` must be implemented
first; it cannot reuse `OpenAIProvider`.

## Recording contents

| Path | Count | Coverage |
|------|------|----------|
| `/api/chat` | 20 | Streaming / non-streaming completions, structured output, tool calling, thinking |
| `/api/tags` | 1 | list models |

## Next steps

Once `OllamaProvider` is implemented, add `mod ollama_conformance` in
`conformance_test.rs` and mount it following the bedrock pattern.
