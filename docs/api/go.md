# aimux · Go API

> **0.6.0.** This binding uses ABI 1 / ops 1.0. Provider-specific Go
> constructors were removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

The Go binding statically links `libaimux_ffi.a` through cgo, so the Rust core
is compiled into the executable and the result is a single binary. It requires
Go 1.24+, cgo, the matching generated header and a matching Rust archive.

## Install

```bash
go get github.com/arcships/aimux/bindings/go
go generate github.com/arcships/aimux/bindings/go
go build ./...
```

`go generate` downloads the platform archive and `aimux.h` from the matching
release. Pin it with `AIMUX_FFI_VERSION` when needed.

For local development:

```bash
cargo build -p aimux-ffi --profile staticlib-release
cd bindings/go
go test ./...
```

## Quick start

```go
ctx := context.Background()
provider, err := aimux.Create(ctx, nil, "provider.create", map[string]any{
    "package": "openai",
    "settings": map[string]any{"apiKey": os.Getenv("OPENAI_API_KEY")},
})
if err != nil {
    return err
}
defer provider.Close()

model, err := aimux.Create(ctx, provider, "provider.model", map[string]any{
    "method": "languageModel", "modelId": "gpt-4o-mini",
})
if err != nil {
    return err
}
defer model.Close()

result, err := aimux.CallInto[aimux.GenerateTextResult](ctx, model, "text.generate", map[string]any{
    "prompt": "Hello", "options": aimux.GenerateTextOptions{},
})
if err != nil {
    return err
}
fmt.Println(result.Text)
```

`Create` owns the returned entry, whether the operation returns a decimal
handle string or an object containing `handle`. `Call` invokes a global
operation; `Handle.Call` invokes an operation on an owned handle. Names and
versions come from `generated/ops.go` and `Manifest()`.

## Types and handles

- `Call` accepts typed values or `json.RawMessage` and returns canonical raw
  JSON. `CallInto[T]` decodes a typed result.
- `Params` preserves absent keys versus explicit null. Optional scalars and
  structs use pointers; optional slices and maps omit nil.
- `Field[T]` distinguishes absent, explicit null and a typed value through
  `Present`, `Null` and `Value`.
- Tagged unions expose a sealed `Value` variant and reject unknown tags.
- Handle IDs encode as decimal strings, including values above 2^53. Never
  adopt an ID twice.

## Streams and operations

`Handle.Stream` returns copied event JSON through `Events()`. The queue holds
seven events plus one pending producer, with at most 1 MiB per event. The
producer closes the channel after callbacks return. `Cancel` releases a full
queue even without a consumer. Always close an abandoned stream.

`StartOperation` retains its context signal through the transcription
operation lifetime. `Push`, `InputDone`, `NextPart`, `Status`, `Cancel` and
`Close` expose operation controls. `NextPart` distinguishes `Part`, `Ended`
and `PollTimeout`.

## Errors

```go
var aimuxErr *aimux.Error
if errors.As(err, &aimuxErr) {
    fmt.Println(aimuxErr.Envelope.Kind, aimuxErr.Envelope.Code, aimuxErr.Envelope.Message)
    fmt.Println(errors.Unwrap(err)) // cause chain
}
```

- `Envelope` contains the domain, code, message and complete error payload.
- `RawEnvelope` retains unknown envelope fields.
- `Unwrap()` returns the payload's cause as `*aimux.Cause`; a retry error's
  cause is its last attempt.
- Read a retry delay from `error.responseHeaders`, not from a dedicated retry
  field.
- The binding adds no retry policy.

## Breaking changes in 0.6

- `NewOpenAI` / `NewAnthropic` and the other provider-specific constructors are
  removed; use `Create` / `Call` / `CallInto`.
- Operations replace the 0.5 provider/model API; handles are decimal strings.
- Errors are the shared envelope; typed error getters were replaced by the
  payload and cause chain.
- Generated DTOs replace the hand-written wire types.

## See also

- [bindings/go/README.md](../../bindings/go/README.md) — build, archive,
  platform and test details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Go provider list](providers.md).
