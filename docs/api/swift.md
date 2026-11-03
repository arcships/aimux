# aimux · Swift API

> **0.6.0.** This binding uses ABI 1 / ops 1.0 and the generated header. The
> 0.5 provider constructors and unversioned exports are removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

The Swift package links `aimux_ffi` through the versioned C ABI. Generated
`AimuxOps` constants and Codable DTOs in `generated/Types.swift` are part of
the module.

## Install

The repository root is not a Swift package; add the package under
`bindings/swift` from a checkout as a local dependency:

```swift
.package(path: "../aimux/bindings/swift"),
.target(
    name: "YourApp",
    dependencies: [.product(name: "Aimux", package: "aimux")]
)
```

In Xcode, use **File → Add Package Dependencies → Add Local** and select
`bindings/swift`. A remote URL dependency is not provided for 0.6.0.

Provide the matching `libaimux_ffi.dylib` / XCFramework from the release on the
loader path. For a local build:

```bash
cargo build -p aimux-ffi --release
swift test --package-path bindings/swift --disable-sandbox \
  -Xlinker -L -Xlinker "$PWD/target/release" \
  -Xlinker -rpath -Xlinker "$PWD/target/release"
```

## Quick start

```swift
let manifest = try await AimuxRuntime.manifest()
let provider = try await AimuxRuntime.open(AimuxOps.OP_PROVIDER_CREATE, params: .object([
    "package": .string("openai"),
    "settings": .object(["apiKey": .string(apiKey)]),
]))
defer { provider.close() }

let model = try await provider.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
    "method": .string("languageModel"),
    "modelId": .string("gpt-4o-mini"),
]))
defer { model.close() }

let request: JSONValue = .object([
    "prompt": .string("Hello"),
    "options": .object([:]),
])
let result = try await model.call(AimuxOps.OP_TEXT_GENERATE, params: request)
let stream = try await model.stream(AimuxOps.OP_TEXT_STREAM, params: request)
defer { stream.close() }
for try await frame in stream {
    print(frame.seq, frame.event)
}
```

`AimuxRuntime` addresses global scope; `AimuxHandle` owns a nonzero `UInt64`
handle. `handle.id` provides a borrowed ID for secondary handle fields; encode
it as a decimal string and retain its owner until the call returns.

## DTOs and handles

- `Generated/Types.swift` uses the Rust type names; structs and
  associated-value enums conform to `Sendable`.
- `FieldPresence` distinguishes absent keys, explicit nulls and values.
- `JSONValue` retains integral numbers as `Int64`/`UInt64` and other numbers
  as `Decimal`.
- Adopt a raw ID with `init(owning:)` to transfer unique ownership.

## Streams and cancellation

`BoundedStream` uses a blocking producer and one consumer. The queue holds at
most 256 frames / 8 MiB; each serialized frame is at most 1 MiB. Full queues
block native callbacks. Terminals have a separate slot; errors throw once
after buffered events.

Task cancellation signals Abort. Exiting a loop releases its temporary
iterator and cancels production even while the sequence remains alive. Stored
iterators require `close()` when abandoned. Callback context is released once
after the native stream call returns.

`push(Data)` and `nextPart(timeoutMs:)` use binary push/poll primitives. Poll
returns `.part`, `.ended` or `.timeout`.

## Errors

`AimuxError` retains domain, code, message and unknown JSON fields. `cause` is
the payload's cause: a nested AI SDK error, or an OS/TLS/parser error as
`{message, cause?}`. A retry error's cause is its last attempt. Read a retry
delay from `error.responseHeaders`, not from a dedicated retry field.

## Breaking changes in 0.6

- Provider constructors such as `Model.openai(...)` and `Aimux.openai(...)` are
  removed; use `AimuxRuntime.open` / `AimuxHandle.open` with operation names.
- The binding requires the exact ABI, ops version and manifest digest.
- Generated DTOs replace the 0.5 hand-written types.
- Errors are the shared envelope with a native `cause` chain.

## See also

- [bindings/swift/README.md](../../bindings/swift/README.md) — package,
  bounded-stream and test details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Swift provider list](providers.md).
