# aimux · Node.js API

> **0.6.0.** This binding uses ABI 1 / ops 1.0. The 0.5 top-level
> `openai(...)` / `generateText(...)` helpers are removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

The Node binding is the native napi-rs path: it calls the shared
`aimux_ops::dispatch` directly, not the C ABI. It requires Node 18+ and a
matching platform package.

## Install

```bash
npm install @arcships/aimux
```

The package ships the platform `.node` binary in the matching
`@arcships/aimux-*` package; no Rust toolchain is needed.

## Quick start

```ts
import { Client, LanguageModel } from '@arcships/aimux'

const client = new Client()
const provider = await client.provider({
  package: 'openai',
  settings: { apiKey: process.env.OPENAI_API_KEY },
})
const model = await provider.model('gpt-4o-mini', 'languageModel')
try {
  if (!(model instanceof LanguageModel)) throw new Error('Expected Language')
  console.log((await model.generate('Hello')).text)

  const controller = new AbortController()
  for await (const frame of await model.stream('Hello', {}, { signal: controller.signal })) {
    if (frame.type === 'event') console.log(frame.event)
    else console.log(frame.outcome, frame.error)
  }
} finally {
  await model.close()
  await provider.close()
}
```

`Client` and the generated resource classes expose the same operation names as the manifest:
`provider.create`, `provider.model`, `text.generate`, `text.stream`,
`embedding.embed`, `speech.generate`, `transcription.start`,
`decision.decide`, `provider.files`, and so on. `LanguageModel` is the typed
facade for a language handle; other resource classes follow the same pattern.

## Raw entrypoint

`@arcships/aimux/raw` exposes the transport without the typed wrapper:

```ts
import { Runtime } from '@arcships/aimux/raw'

const runtime = new Runtime()
const manifest = await runtime.call('0', 'runtime.describe', '{}')
const result = await runtime.call(handle, 'text.generate', JSON.stringify(params))
```

`runtime.stream(handle, op, paramsJson, abort?)` returns the same event/end
frames. Handles and sequence numbers are decimal strings, including values
above 2^53.

## Streams and cancellation

- A stream delivers business event frames plus one explicit end frame.
  Recoverable error events remain data; the end frame describes the terminal
  outcome.
- `return()`, `throw()`, reader cancellation, object collection and Env
  teardown cancel the underlying work.
- A full queue applies backpressure. Explicit cancellation wakes a blocked
  producer.
- `AbortController.signal` maps to native Abort; aborting a shared handle
  cancels every user of that work.

## Errors

```ts
try {
  await model.generate('Hello')
} catch (error) {
  // AimuxError: kind, code, message, error, cause
  console.error(error.kind, error.code, error.message, error.error)
}
```

- The envelope is `{ kind, code, message, error }`.
- `error.cause` is an `AimuxCauseError` for a nested AI SDK error or an
  `AimuxSourceError` for an OS/TLS/parser error; follow its own `cause` chain.
- A retry error's cause is its last attempt.
- Read a retry delay from `error.responseHeaders` (`retry-after-ms`, then
  `retry-after`), not from a dedicated retry field.

## Breaking changes in 0.6

- Top-level `openai`, `generateText`, `streamText` and provider-specific
  constructors are removed; use `Client` and the resource classes.
- Errors are the shared envelope; `retryMs` and the old error-value companion
  are gone.
- DTOs are generated from the Rust types; hand-written 0.5 wire types are
  removed.
- The runtime requires the exact ABI, ops version and manifest digest.

## See also

- [bindings/node/README.md](../../bindings/node/README.md) — limits, test
  commands, raw transport and lifecycle details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Node provider list](providers.md).
