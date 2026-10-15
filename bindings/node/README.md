# Node ops binding

The napi adapter awaits `aimux_ops::dispatch` on napi-rs's Tokio runtime. It does
not use the C ABI, create a blocking runtime, retry calls, or accept JavaScript
fetch/model/token/telemetry implementations.

```ts
import { Client, LanguageModel } from '@arcships/aimux'

const client = new Client()
const provider = await client.provider({ package: 'openai', settings: { apiKey: '...' } })
const model = await provider.model('gpt-4o-mini', 'languageModel')
try {
  if (!(model instanceof LanguageModel)) throw new Error('Expected Language')
  console.log((await model.generate('Hello')).text)
  const controller = new AbortController()
  for await (const frame of await model.stream('Hello', {}, { signal: controller.signal })) {
    // Data errors remain data. Only the explicit end frame describes termination.
    if (frame.type === 'event') console.log(frame.event)
    else console.log(frame.outcome, frame.error)
  }
} finally {
  await model.close()
  await provider.close()
}
```

The raw entrypoint `@arcships/aimux/raw` exposes `Runtime.call(handle, op,
paramsJson, abort?)` and `Runtime.stream(...)`. It transports the same DTOs and
frames as the typed facade. `runtime.abort(signal?)` creates a native Abort.
`runtime.operationPush(handle, bytes)` copies bytes before returning a Promise.
Handle IDs and sequence numbers are decimal strings, including beyond 2^53.
Resources belong to their Node Env; they cannot be transferred to workers.

Streams have a 64-item / 8 MiB data bridge and a 1 MiB serialized frame limit.
They stop pulling upstream when full. The terminal has a separate slot, so
cancellation cannot lose its outcome behind a full queue. `return()`, `throw()`,
`readable(iterator).getReader().cancel()`, object collection, and Env teardown
cancel the underlying work. Explicit cancellation can interrupt pending reads
and capacity waits. A forgotten live iterator can retain its bounded queue;
close it explicitly when it is no longer needed.

Errors expose the shared `kind`, numeric `code`, `message`, `error` payload and
optional `retryAfterMs`. The AI SDK payload retains `name`, `isRetryable` and
other variant fields. Data error events and end-frame error envelopes stay
plain DTOs. Raw and typed call/next rejections use `AimuxError`.

`close()` is idempotent. Recorder persistence is explicit: await `flush()` and
inspect its result before closing; finalization does not flush. Dropping a
provider does not revoke a model's strong reference. Dropping an Operation
cancels it; dropping an Abort releases the handle without signalling shared work.

The binding rejects incompatible ABI/ops versions at load and exposes the
manifest and digest. The shared manifest/codegen integration must additionally
pin its consumed type-schema revision before release.

Run `npm run test:facade` for the JSON-transport facade tests (Node with TypeScript stripping,
22.18+). These tests use a transport double. Run `npm run test:native` after
building a real napi binary for owner/error/Env integration checks. `npm run test:lifecycle` runs local HTTP timer, slow-consumer, cancellation and
shutdown scenarios; it requires permission to listen on loopback sockets. `npm test`
runs all three suites and requires that binary. Rust bridge
unit tests exercise real channel/byte limits and cancellation with synthetic
streams. Build the addon with `npm run build:debug` and the typed facade with
`npm run build:typed`. Run Rust unit tests with
`cargo test --manifest-path bindings/node/Cargo.toml --lib`.

The older benchmark programs have not been migrated to the breaking ops API;
their previous measurements do not describe this adapter.

For C ABI parity, build `aimux-ffi`, set `AIMUX_FFI_LIBRARY` to its absolute library
path, and run `npm run test:c-parity` (Python 3 is required). This compares the
same provider, registry, trace, recorder, session, replay and error vectors.

Recorder, session storage and inference state are process-wide. Trace middleware
creation currently reports UnsupportedFunctionality because the shared runtime
has no canonical model-wrapper hook. Credential resolver creation and replay
transport capabilities also report UnsupportedFunctionality. Recording schemas
are unchanged; business type schema generation and per-platform installation
verification are separate release work.
