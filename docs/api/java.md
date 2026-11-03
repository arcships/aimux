# aimux · Java API

> **0.6.0.** This binding uses ABI 1 / ops 1.0. The unversioned 0.5 binding and
> its provider constructors are removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

Java uses the C ABI through JNA. Java 8 bytecode is supported; no native
toolchain is needed at build time. Wire DTOs are generated in
`ai.arcships.aimux.types`, and the generated operation constants are in
`AimuxOps`.

## Install

Maven Central:

```groovy
implementation("ai.arcships:aimux-java:0.6.0")
```

JNA loads `aimux_ffi` by name. Provide `libaimux_ffi.so` / `.dylib` /
`aimux_ffi.dll` from the matching GitHub Release on the JNA search path
(`-Djava.library.path=...` or `LD_LIBRARY_PATH`).

## Quick start

```java
Aimux aimux = Aimux.load();
String providerJson = "{\"package\":\"openai\",\"settings\":{\"apiKey\":\"your-key\"}}";
String requestJson = "{\"prompt\":\"Hello\",\"options\":{\"maxRetries\":0}}";

try (ProviderHandle provider = aimux.createProvider(providerJson);
     Model model = provider.languageModel("gpt-4.1-mini")) {
    System.out.println(model.generate(requestJson));
    model.stream(requestJson, frame -> {
        if (frame instanceof ai.arcships.aimux.types.StreamFrame.Event) {
            System.out.println(((ai.arcships.aimux.types.StreamFrame.Event) frame).getEvent());
        }
    });
}
```

`Aimux.load()` (or `Aimux.load(path)`) validates the ABI, ops version and
manifest before the client becomes usable. Provider settings are plain data;
provider-specific constructors no longer exist.

## Resources and operations

- `ProviderHandle` exposes Language, Embedding, Image, Speech, Transcription,
  Reranking, Video, Search, Decision and Files.
- `decisionModel(id)` selects the native `decision` method; support depends on
  the provider.
- Every resource supports `call`, `stream` and `create` with the operations
  accepted by its native handle kind.
- `Aimux.call` uses global handle 0. `Aimux.create` creates registries,
  recorders, transports, middleware, trace stores and replay sessions.
- `own(id)` transfers ownership; wrap each returned handle exactly once.
- `TypedModel` owns a supplied `Model` and decodes generated result classes.

Use try-with-resources. `close()` is idempotent and stops new calls
immediately; admitted calls retain the resource until they return. Closing a
model does not signal cancellation.

## Async and streams

Every asynchronous method requires an explicit `Executor`. The internal
`Cancellation` helper forwards `Future.cancel(false/true)` to native Abort;
running native work retains its resources until it returns. For synchronous
cancellation, use `createAbort()` and the `call`/`stream` overloads that
accept it.

Consumers receive copied `StreamFrame` values, including the unique End frame.
An error End is delivered before its typed exception is thrown. A slow
Consumer applies synchronous backpressure. Callback exceptions are latched,
trigger Abort and become `BindingException.CallbackFailure` after native
return; no exception crosses the C callback.

## Errors

- `NativeException` retains `kind`, `code`, message and the full `error`
  payload.
- Domain exceptions select subclasses by `error.name`; unfamiliar variants use
  the domain's `Unknown` subclass and retain their payload.
- Retryability comes from `error.isRetryable`; a retry delay comes from
  `error.responseHeaders` (`retry-after`), as with the AI SDK.
- The payload's `cause` is `getCause()`: an `ErrorCause` chain. A retry
  error's cause is its last attempt.

## Breaking changes in 0.6

- The unversioned binding is replaced by ABI 1 / ops 1.0.
- Provider-specific constructors (`Model.openai`, `TypedModel.openai`, …) are
  removed; use `Aimux.createProvider` / `provider.languageModel` and the
  operation API.
- Hand-written `Types` / `MultimodalTypes` are replaced by generated DTOs in
  `ai.arcships.aimux.types`.
- Errors are the shared envelope; old getters and retry hints are removed.

## See also

- [bindings/java/README.md](../../bindings/java/README.md) — lifecycle, async,
  callback and test details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Java provider list](providers.md).
