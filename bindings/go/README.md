# aimux-go

> **0.6.0.** This binding uses ABI 1 / ops 1.0. Provider-specific
> constructors are removed. See
> [Upgrading to 0.6](../../docs/upgrading-to-0.6.md).

Go bindings for the operation C ABI. Requires cgo, Go 1.24+, the matching
generated `aimux-ffi/include/aimux.h`, and a matching Rust library
with the `aimux_call_async` and `aimux_stream_async` symbols.
Provider-specific constructors have been replaced by operation calls.

```go
ctx := context.Background()
provider, err := aimux.Create(ctx, nil, "provider.create", map[string]any{
    "package": "openai",
    "settings": map[string]any{"apiKey": os.Getenv("OPENAI_API_KEY")},
})
if err != nil { return err }
defer provider.Close()
model, err := aimux.Create(ctx, provider, "provider.model", map[string]any{
    "method": "languageModel", "modelId": "gpt-4o-mini",
})
if err != nil { return err }
defer model.Close()
result, err := aimux.CallInto[aimux.GenerateTextResult](ctx, model, "text.generate", map[string]any{
    "prompt": "Hello", "options": aimux.GenerateTextOptions{},
})
if err != nil { return err }
fmt.Println(result.Text())
```

`Call` invokes a global operation; `Handle.Call` invokes one on an owned
handle. `Create` owns the returned entry, whether the operation returns a
decimal handle string or an object containing `handle`. Names are available
in `generated/ops.go` and `Manifest()`. The first operation checks ABI major,
the exact generated operation version, and manifest compatibility (`MANIFEST_DIGEST`
fast-path with full `generated.TYPE_SCHEMA_HASHES` `schemaHash` fallback). Per-type `revision`
and transitive `schemaHash` metadata are generated in `schema/manifest.json` and `generated.TYPE_SCHEMA_HASHES`.

Calls accept typed values or `json.RawMessage` and return canonical raw JSON.
`CallInto[T]` decodes a typed result. `Params` preserves absent keys versus
explicit null. Optional scalars and structs use pointers; optional slices and
maps omit nil and retain empty values. Only Presence fields use `Field[T]` to
distinguish absence, explicit null and a typed value through `Present`, `Null`
and `Value`. Set `Present: true` when constructing a Presence field. Opaque JSON
uses `json.RawMessage` and retains large integers.
Tagged unions expose a sealed `Value` variant and reject unknown tags.
Nil parameters encode `null`; use `struct{}{}` for `{}`. Handle IDs encode as
decimal strings, including above 2^53. Never adopt an ID twice.

Context cancellation signals native Abort. `StartOperation` retains its
context signal through the transcription operation lifetime. `Push` submits
binary input and may return ResourceLimit when native admission is full.
`InputDone`, `NextPart`, `Status`, `Cancel` and `Close` expose operation
controls. `NextPart` distinguishes Part, Ended and PollTimeout; its timeout
must be nonnegative, with zero polling immediately. A terminal failure is
delivered once by polling and remains available through Status.

`Handle.Stream` returns copied event JSON through `Events()`. The queue holds
seven events plus one pending producer, with at most 1 MiB per event and
bounded frame overhead. The producer closes the channel after terminal callback
delivery. `Cancel` releases a full queue even without a consumer. `Err` and
`Terminal` wait for producer completion; drain Events first or cancel when
abandoning consumption. Terminal control state is stored separately from data.
A late cancellation preserves the recorded native outcome. Always close an
abandoned stream.

Native errors support `errors.As` with `*aimux.Error`. `Envelope` contains the
domain, code, message and complete error payload; a retry delay is read from
`error.responseHeaders`, as with the AI SDK. `RawEnvelope` also retains unknown
envelope fields. `Unwrap` returns the
payload's cause as a `*aimux.Cause` chain (`Name` is empty for an OS, TLS or
parser error; a retry error's cause is its last attempt), so `errors.Unwrap`
and `errors.As` reach it. The binding adds no retry
policy. Handles support concurrent calls and idempotent Close. Use Close for
normal cleanup; the fallback finalizer only enqueues native cleanup.

## Build and test

Build `aimux-ffi` with the matching source and header:

```sh
cargo build -p aimux-ffi --profile staticlib-release
```

`staticlib-release` is the release profile without LTO, the profile the
released archives are built with (a fat-LTO archive is twice the size for the
same linked code, see RFC-0043). By default cgo searches
`target/staticlib-release`, then `target/release`, for `libaimux_ffi`; a shared
library can also be selected with `CGO_LDFLAGS` and the platform loader's
library path. `go generate ./...`
downloads the platform archive and header from one release; pin it with
`AIMUX_FFI_VERSION`.

For a built shared library:

```sh
AIMUX_LIBRARY_DIR=/absolute/path/to/target/debug ./test-native.sh -count=1
```

This runs `go test -race -timeout 60s ./...` against Rust, including manifest
checks, provider/model/composite ownership, concurrent Close/call dispatch,
stream admission and terminal errors, and persistent operation errors. Tests
use local validation, unsupported operations, and a loopback cancellation test.
Set `GOEXPERIMENT=cgocheck2` for strict cgo pointer checking.

`./test-abi.sh -count=1` builds a deterministic C double against the production
header in a temporary directory. It tests cgo ownership, borrowed-memory
copies, malformed callbacks, full-queue cancellation, operation lifetime,
large handle IDs and callback-token cleanup. These tests skip against Rust;
Rust integration tests skip against the double. Both suites include bridge
race tests and typed DTO round trips. Live provider calls, successful audio
transport, cross-platform packaging and Go 1.24 itself require separate
verification.

## Async ABI benchmarks

Run each arm in a fresh process so threads created by one arm do not carry
over to the other. Alternate the order within each pair of runs.

```sh
go test -c -o ../../target/aimux-go-bench.test
for arm in CallFanOut/sync CallFanOut/async StreamBurst/per_frame StreamBurst/batch; do
    ../../target/aimux-go-bench.test -test.run '^$' \
        -test.bench "^Benchmark${arm%%/*}$/^${arm#*/}$" -test.benchtime 10x
done
```

Fan-out submits 64 requests to a loopback server with a 25 ms response delay.
An untimed warm-up samples threads and goroutines after all 64 requests reach
the server. The stream fixture sends 5,000 SSE text deltas. Both stream arms
use the same frame decoder and inline consumer to measure bridge costs with
identical work. The reported frame rate and callback count include the final
`End` and any other native control frames.
