# aimux · Python API

> **0.6.0.** This binding uses ABI 1 / ops 1.0. The 0.5 top-level
> `openai(...)` / `generate_text(...)` helpers are removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

The Python binding is the native PyO3 path: it calls
`aimux_ops::dispatch` directly. It requires CPython 3.9 or newer. Install
`arcships-aimux[typed]` for optional Pydantic schemas and text helpers.

## Install

```bash
pip install arcships-aimux
# optional typed schemas and helpers
pip install "arcships-aimux[typed]"
```

The wheel ships the native extension; no Rust toolchain is needed.

## Quick start

```python
from aimux import Client

client = Client()
with client.adopt(client.call("provider.create", {
    "package": "openai",
    "settings": {"apiKey": "your-api-key"},
})["handle"]) as provider:
    with client.adopt(provider.call("provider.model", {
        "method": "languageModel",
        "modelId": "gpt-4o-mini",
    })["handle"]) as model:
        result = model.call("text.generate", {
            "prompt": "Hello",
            "options": {},
        })
        print(result["text"])

        with model.stream("text.stream", {
            "prompt": "Hello",
            "options": {},
        }) as events:
            for frame in events:
                print(frame["seq"], frame["event"])
```

`Client.call(op, params=None, *, handle="0", abort=None)` returns parsed JSON.
`Client.stream` / `Handle.stream` return synchronous context-managed iterators
over `{type, seq, event}` frames.

## Handles and streams

- Adopt every returned handle with `client.adopt(...)`; `close()` cancels,
  joins and clears the queue.
- Handles are decimal strings, including values above 2^53.
- A stream produces business event frames plus one terminal. Validation errors
  raise before acceptance; terminal failures raise typed errors.
- The bridge holds at most 64 events / 8 MiB, with at most 1 MiB per event.
  Slow consumers apply backpressure.
- `Abort.cancel()` from another thread interrupts a call or stream. Blocking
  native waits release the GIL.

## Transcription and binary operations

```python
with client.adopt(model.call("transcription.start")) as operation:
    operation.push(b"\x00\x00")
    operation.call("operation.inputDone")
    part = operation.call("operation.next", {"timeoutMs": 500})
```

`push` accepts `bytes`, `bytearray` and `memoryview`; a full input queue raises
`ResourceLimit` (protocol code 305). Use the binary method, not a JSON
`operation.push` call.

## Errors

```python
from aimux import AimuxError

try:
    model.call("text.generate", {"prompt": "Hello", "options": {}})
except AimuxError as error:
    print(error.kind, error.code, error.message, error.error)
```

- `AimuxError` subclasses expose `envelope`, `error`, `kind` and `code`.
- Payload attributes use snake_case; `error.cause` is `__cause__`.
- Read a retry delay from `error.response_headers` (`retry-after-ms`, then
  `retry-after`), not from a dedicated retry field.
- Retry errors retain typed attempts; `last_error` is the last attempt.

## Breaking changes in 0.6

- Top-level provider/model helpers are removed; use `Client` / `Handle`.
- Provider-specific constructors are replaced by `provider.create` and
  `provider.model`.
- Errors are the shared envelope; `retry_ms` and the old serialized error
  companion are gone.
- DTOs are generated from the Rust types; hand-written 0.5 wire types are
  removed.

## See also

- [bindings/python/README.md](../../bindings/python/README.md) — limits, GIL
  behavior, test commands and typed helpers.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Python provider list](providers.md).
