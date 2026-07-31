# chatgpt cassettes — to be implemented

These 33 recordings come from rig (MIT) and capture the ChatGPT web **Codex
Responses API** (`/backend-api/codex/responses`), not the official OpenAI API.

## Why no playback tests are mounted

There is **no chatgpt provider implementation** under `aimux-providers/src/`.
The request/response format of this endpoint differs from the OpenAI Responses
API (`/v1/responses`):

- The path is `/backend-api/codex/responses` (not `/v1/responses`).
- The request body uses the `input` / `instructions` / `store` / `include`
  field set.
- Authentication uses ChatGPT web OAuth, not `Authorization: Bearer <api-key>`.

To replay these recordings, a standalone `ChatGPTProvider` must be implemented
first; it cannot reuse `OpenAIProvider`.

## Recording contents

All 33 recordings hit the path `/backend-api/codex/responses` and cover:
- Streaming / non-streaming completions
- Multi-turn tool calling (parallel / sequential / nested arguments)
- Reasoning sessions
- Prompt cache / `store` fields
- Unicode parameters, zero-argument tool calls
- 401 error responses

## Next steps

Once `ChatGPTProvider` is implemented, add `mod chatgpt_conformance` in
`conformance_test.rs` and mount it following the bedrock/openai pattern.
