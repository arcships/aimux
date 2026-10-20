# aimux Python binding

PyO3 calls `aimux_ops::dispatch` directly. Provider selection, wire validation,
retries, recording, replay and operation lifecycle run in the shared Rust layer.
The package requires CPython 3.9 or newer. Install `arcships-aimux[typed]` for
optional Pydantic schemas and text helpers in `aimux.wrapper`.

The typed layer exports generated Rust wire DTOs from `aimux._types`.
Text helpers accept a `TextRequest`, for example
`generate_text(model, TextRequest(prompt="Hello", options=Options()))`.
Stream helpers yield generated `TextStreamPart` or `ChatCompletionChunk` values.
Use `base64_wire(value, decode=True)` to read bytes from a base64 wire string.

```python
from aimux import Client

client = Client()
with client.adopt(client.call("provider.create", {
    "package": "openai", "settings": {"apiKey": "your-api-key"},
})["handle"]) as provider:
    with client.adopt(provider.call("provider.model", {
        "method": "languageModel", "modelId": "your-model",
    })["handle"]) as model:
        result = model.call("text.generate", {"prompt": "Hello", "options": {}})
        with model.stream("text.stream", {"prompt": "Hello", "options": {}}) as events:
            for frame in events:
                print(frame["seq"], frame["event"])
```

`Client.call(op, params=None, *, handle="0", abort=None)` returns parsed JSON.
Operation names and DTO identifiers come from `aimux.manifest`; business DTO
schemas remain in the existing core wire types. Control fields use camelCase.
Handle IDs are decimal strings. Adopt each returned handle for explicit and GC
cleanup. Provider/model constructors have been replaced by shared operations.
`provider.create` and `provider.model` return objects with `handle` fields;
`provider.files` and `transcription.start` return handle strings directly.

`Client.stream(op, params=None, *, handle, abort=None)` and `Handle.stream`
return synchronous context-managed iterators over `{type, seq, event}` frames.
Validation errors raise before acceptance. Successful exhaustion produces
`StopIteration`; terminal failures raise typed errors. End frames are consumed
by the adapter. `join()` waits for producer teardown without draining events;
drain or cancel a full stream before joining. `close()` cancels, joins and
clears the queue, with a five-second cleanup deadline. Cancellation after a
settled terminal preserves its outcome.

Use `Abort.cancel()` from another thread to interrupt a call or stream.
Closing a stream, context exit, cancellation and GC trigger teardown. Blocking
native waits release the GIL and check Python signals every 20 ms;
`KeyboardInterrupt` cancels the worker. The bridge holds at most 64 events and
8 MiB of queued data, with at most 1 MiB per serialized event. Terminal state
has independent storage. Worker tasks never acquire the GIL.

For realtime transcription, adopt the handle returned by `transcription.start`.
`Handle.push(data)`, or `client.operation_push(handle, data)`, admits raw bytes
through the shared binary service:

```python
with client.adopt(model.call("transcription.start")) as operation:
    operation.push(b"\x00\x00")
    operation.call("operation.inputDone")
    part = operation.call("operation.next", {"timeoutMs": 500})
```

Push accepts bytes, bytearray and memoryview; chunks above 1 MiB or a full
64-item / 8 MiB input queue raise `ResourceLimit` (protocol code 305).
Admission returns immediately. Polling returns `part`, `ended` or `timeout`.
A JSON call named `operation.push` is invalid; use the binary method.

Each interpreter receives a private token from the shared owner allocator.
Clients in one interpreter share that token. CPython C APIs identify the
interpreter; monkeypatching Python modules cannot forge an owner. Handle GC
queues teardown, and owner teardown queues release of remaining resources.
PyO3 0.22 limits subinterpreter imports; this package does not extend support.

`AimuxError` subclasses preserve the shared `envelope`, `error`, `kind` and
`code`. Payload attributes use snake_case; a retry delay is read from
`error.response_headers` (for example `retry-after`), as with the AI SDK. Retry
`errors` retain typed attempts in order; `last_error` is the last attempt.
Typed causes also appear as `__cause__`. Recording, binding and protocol errors
have separate bases. Python never performs retries.

`ABI_VERSION`, `OPS_VERSION`, `manifest` and `manifest_digest()` are public.
Import checks the exact ABI/ops versions and digest pinned in the generated
metadata. `verify_contract(other_digest)` checks an additional consumer pin.
Recorder/session/infer state is process-wide. Recording schema migration,
transport replay, credential creation and complete
platform install verification are outside this package's current guarantees.

## Development checks

```sh
# Build/install the native wheel and optional test dependencies:
python3 -m maturin develop --extras test
AIMUX_REAL_OPS=1 python3 -m unittest discover -s tests -p 'test_ops_*.py' -v

# Include localhost provider HTTP/WS fixtures and per-entry GIL assertions:
AIMUX_REAL_OPS=1 AIMUX_HTTP_TESTS=1 python3 -m unittest discover -s tests -p 'test_ops_*.py' -v
python3 -m pytest tests/test_gil_release.py -v

# Test the already-built production cdylib before wheel installation:
python3 tests/run_real.py /absolute/path/to/libaimux.dylib

# Isolated boundary tests for queue limits, terminals, cancellation, interrupts,
# concurrent consumers, GC and error chains (no provider behavior assertions):
python3 tests/run_native_contract.py
AIMUX_CORE_DOUBLE=1 python3 tests/run_native_contract.py
```

The isolated runner substitutes test doubles only in a temporary copy. By
default it uses real core and a dispatcher double; the faster mode uses a core
error double with the real AbortSignal source. `AIMUX_BUILD_COMMAND` can supply
a build executable, and `AIMUX_CONTRACT_TARGET` can select its target directory.
Neither mode verifies wheel installation or real provider behavior. All public
execution APIs are synchronous; use an application executor for asyncio.

Ordinary pytest/unittest discovery also replays all 11 shared JSON scenarios from
`AIMUX_CONTRACT_VECTORS/scenarios` (default: `contract-tests/ops/scenarios` in the
repository), without requiring provider integration flags. It builds an isolated
PyO3 module with `contract-tests`, using the production call boundary, real
Dispatcher, framing and bounded queue with an injected scripted source. Release
builds do not expose test operations. Set `AIMUX_BUILD_COMMAND` to a Cargo build
wrapper when needed. Expected results are kept in Python and never passed into
the native driver. This complements the adapter-specific bridge lifecycle tests.

The process-wide runtime is stored in a Rust `OnceLock` and is never dropped at
interpreter shutdown. `InterpreterOwner::drop` queues `release_owner` on that
still-live runtime, including when the module guard or a late client is collected.
