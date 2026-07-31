# Test cassette proposal

## Goal

Tests do not depend on the network or keys. When running tests, replay cassette files instead of making real calls to provider APIs.

## Data sources

### Ready-to-use

The rig project recorded 505 real-response cassettes under the MIT license, which can be used. They cover 16 providers:

| Provider | Cassette count |
|------|:---:|
| Gemini | 116 |
| OpenAI | 64 |
| Anthropic | 65 |
| OpenRouter | 43 |
| DeepSeek | 36 |
| xAI | 34 |
| Copilot | 28 |
| Bedrock | 18 |
| Ollama | 16 |
| Groq | 10 |
| Perplexity | 9 |
| Mistral | 9 |
| Other | 7 |

rig's format is YAML; each file contains `when` (request: path, method, headers, body) and `then` (response: status code, headers, body). For streaming responses, the body is the raw SSE text, chunk by chunk.

### Recording our own

For providers not covered by rig, use the open-source tool llmtape to start a local proxy and make a real API call to record it. This costs API quota, so supplement as needed.

### Not used

litellm's cassettes are not committed to the repository; vcrpy generates them at runtime by default and discards them after running, so they are unavailable.

aimock uses fabricated fake data, not real responses, so it cannot verify whether our parsing is correct; not used.

## Why convert to our own format

Legally, using rig's files directly is fine (the MIT license permits it). But converting to our own format is for engineering cleanliness — we define the structure and naming ourselves, and are not locked into rig's directory structure. The converted files just need to retain a one-line source declaration.

## Format design

Use JSON, one file per scenario. Structure:

```json
{
  "source": "rig (MIT license)",
  "provider": "anthropic",
  "scenario": "streaming tool calling",
  "request": {
    "path": "/v1/messages",
    "method": "POST",
    "headers": { "content-type": "application/json" },
    "body": { ... }
  },
  "response": {
    "status_code": 200,
    "headers": { "content-type": "text/event-stream" },
    "body": "event: message_start\ndata: {...}\n\nevent: ..."
  }
}
```

Key points:

- **The response body stores raw text**. For non-streaming it is a JSON string; for streaming it is the raw SSE text. This way it is sent back as-is during replay, and our parsing code processes real data.
- **No sensitive information is stored**. rig has already replaced IDs with `REDACTED`; we keep this practice.
- **Scenario names in plain language**. For example "streaming tool calling", "structured output", "empty finish round", rather than rig's directory names.

## Replay mechanism

During testing, start a local mock service that reads cassette files, matches by request, and returns the corresponding response.

Matching rule: match by path + features of the request body. Do not match by the full request body, because the body may contain unstable fields such as timestamps and random numbers. Exactly which fields to match will be decided at implementation time.

The mock service is written in Rust and integrated into the tests. During replay, the provider's URL is pointed at the local mock service, and the provider code does not know it is being tested.

## Unified contract tests

Cassettes solve "having real data to replay". Add another layer of unified tests to ensure consistent behavior across all providers.

Approach: define a set of standard inputs (e.g. "generate text", "streaming generation", "with tool calling"), run the same inputs for each provider, and assert the same behaviors (e.g. "returned text", "streaming has a start and an end", "tool calling parsed correctly").

This layer of tests does not check the specific returned content (each provider returns different content); it only checks the structure — whether our code parsed correctly.

## Implementation steps

1. **Write a conversion script**: convert rig's 505 YAML cassettes into our JSON format and store them under `tests/cassettes/`.
2. **Write the replay service**: a Rust-implemented local mock service that reads JSON cassettes and returns responses matched by request.
3. **Refactor existing tests**: change the tests currently hand-written with wiremock to use the replay service.
4. **Write unified contract tests**: define standard inputs, run all providers, and assert the behavior structure.
5. **Record missing providers**: use llmtape to record the providers not covered by rig.

## Directory structure

```
tests/
├── cassettes/           # recording files (JSON)
│   ├── openai/
│   ├── anthropic/
│   └── ...
├── replay/              # replay service implementation
│   └── mod.rs
└── conformance/         # unified contract tests
    └── mod.rs
```
