# AI SDK protocol fixtures

Records what the official Vercel AI SDK (`ai` + `@ai-sdk/*`, versions pinned in
`package.json` and `fixtures/aisdk/VERSIONS.json`) sends and returns for a fixed input over
a mocked transport. These are the AI SDK half of the RFC-0036 section 9.3 protocol
differential: Rust tests replay the same inputs against aimux's providers and compare URL,
headers, body, normalized result, stream parts and error classification.

## Run

```sh
cd scripts/aisdk-fixtures
npm ci
node record.mjs                      # all cases
node record.mjs openai               # one package
node record.mjs openai/chat-basic    # one case (VERSIONS.json / SKIPPED.md untouched)
```

No network is used. The mock `fetch` (`mock-fetch.mjs`) throws on any URL other than the
case's expected one and on a second request.

## Layout

- `record.mjs` - runner. `mock-fetch.mjs` - `createMockFetch(canned)`. `common.mjs` - fake keys, SSE helper.
- `cases/<package>.mjs` - default-exports an array of cases:
  `{ name, build(mock), call(model, run), response, env?, expectError?, expectRequests?, observe?, skip? }`.
  `build` creates the provider/model with `fetch: mock`; `call` invokes `run('generateText' | 'streamText' | 'embed' | 'embedMany' | 'generateImage', input)`;
  `response` is `{ url, status, headers, body }` (JSON object, or a string with `content-type: text/event-stream` for SSE).
  `observe` adds an `observations` object (e.g. which header names were sent, `usedEnvKey`).
  `skip: '<reason>'` skips the case and records it in `fixtures/aisdk/SKIPPED.md`.
- Output: `fixtures/aisdk/<package>/<case>.json`, where `<package>` is `openai`, `openai-compatible`, `anthropic`, `google`.

## What a fixture captures

`package`, `version`, `case`; `sdk.{operation,input}` (the options passed to the SDK, minus the model; tools
are serialized as `{ description, inputSchema }`); `model.{provider,modelId}`; `request.{method,url,headers,body}`
(headers lower-cased and sorted, JSON body parsed); `response.{status,headers,body}` (the canned reply);
and either `result` (normalized SDK output) or `error.{name,message}` (then `request` is `null` if nothing was sent);
optionally `observations`. All object keys are sorted so diffs are stable.

`result` per operation: `generateText` -> `content, finishReason, rawFinishReason, providerMetadata, response{id,modelId,headers}, text, usage, warnings`;
`streamText` -> `parts` (full `fullStream`), `finishReason`, `usage`; `embed`/`embedMany` -> `embeddings, usage, providerMetadata`;
`generateImage` -> `images[{base64Length, mediaType}], warnings`.

## Redaction and determinism

- Headers named `authorization`, `x-api-key`, `api-key`, `x-goog-api-key`, or whose value contains one of the fake keys,
  become `"<redacted>"` (name kept). Only fake keys are used (`sk-test-fixture`, `sk-env-should-not-be-used`);
  the recorder aborts if either string appears anywhere in a fixture.
- `Date` is frozen (2025-01-01T00:00:00Z), `Math.random` is seeded, and `_internal.generateId/generateCallId/now`
  are injected into `generateText`/`streamText` (these `_internal` options are not recorded in `sdk.input`).
- `navigator.userAgent` is pinned to `Node.js/22` so the `user-agent` header (`ai/<v> ai-sdk-provider-utils/<v> node.js/22`)
  does not depend on the local Node version. The user-agent string is otherwise kept verbatim.
- Provider env vars (`OPENAI_*`, `ANTHROPIC_*`, `GOOGLE_*`, `GROQ_*`, `AI_GATEWAY_*`, `VERCEL_*`) are cleared per case; a case sets what it needs via `env`.
- Canned responses always carry explicit ids, so provider-side generated ids never enter the fixtures.

## Policy

Fixtures are committed and are the reference that Rust tests replay. Regenerate them only on purpose: when bumping the
pinned versions (update `package.json` + `package-lock.json` together) or adding/changing a case. Review the diff; a changed
fixture means the SDK's wire behavior changed. Running `node record.mjs` twice must yield no diff.
