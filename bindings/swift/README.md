# Aimux Swift

The Swift package uses the versioned C ABI and its generated header. Link
`aimux_ffi`, or provide it in a packaged XCFramework. Generated `AimuxOps`
constants are part of the Swift module.

`AimuxRuntime` addresses global scope; `AimuxHandle` owns a nonzero `UInt64`
handle. Native dispatch validates operation names, handle kinds and control
fields. `handle.id` provides a borrowed ID for secondary handle fields; encode
it as a decimal string and retain its owner until the call returns. `open` accepts the native handle record or the decimal string returned
by `transcription.start`. Codable DTOs in `generated/Types.swift` use the Rust
type names. Structs and associated-value enums conform to `Sendable`.
`FieldPresence` distinguishes absent keys, explicit nulls and values where the
wire contract requires it. `JSONValue` retains integral numbers as `Int64` or
`UInt64` and other numbers as `Decimal`.

```swift
let manifest = try await AimuxRuntime.manifest()
let provider = try await AimuxRuntime.open(AimuxOps.OP_PROVIDER_CREATE, params: .object([
    "package": .string("openai"),
    "settings": .object(["apiKey": .string(apiKey)]),
]))
defer { provider.close() }
let model = try await provider.open(AimuxOps.OP_PROVIDER_MODEL, params: .object([
    "method": .string("languageModel"), "modelId": .string("gpt-4o-mini"),
]))
defer { model.close() }
let request: JSONValue = .object(["prompt": .string("Hello"), "options": .object([:])])
let result = try await model.call(AimuxOps.OP_TEXT_GENERATE, params: request)
let stream = try await model.stream(AimuxOps.OP_TEXT_STREAM, params: request)
defer { stream.close() }
for try await frame in stream {
    print(frame.seq, frame.event)
}
```

Decision models use `provider.model` with `method: "decision"`, then
`decision.decide` and `decision.capabilities`. Other modalities and pure tool
repair operations use the same call interface and manifest request types.

Blocking native calls run on GCD workers. `close()` atomically ends admission,
signals active work, and queues native drop after admitted calls return. ARC
cleanup follows the same path. Adopting a raw ID with `init(owning:)` transfers
its unique ownership to the facade.

Streams use `BoundedStream` with a blocking producer and one consumer. The
queue holds at most 256 frames / 8 MiB; each serialized frame is at most 1 MiB.
`StreamLimits` allows smaller budgets. Full queues block native callbacks.
Terminals have a separate slot; errors throw once after buffered events.
`AimuxError` retains domain, code, message, retry hint and unknown JSON fields.

Task cancellation signals Abort. Exiting a loop releases its temporary iterator
and cancels production even while the sequence remains alive. Stored iterators
require `close()` when abandoned. Explicit close wakes full queues and readers.
Callback context is released once after the native stream call returns.
Callbacks copy and decode borrowed data synchronously and latch conversion
failures before signalling Abort.

`push(Data)` and `nextPart(timeoutMs:)` use binary push/poll primitives. Poll
returns `.part`, `.ended` or `.timeout`; timeouts must be nonnegative. Cancelling
push/poll or closing an active Operation requests `operation.cancel` on a worker.

## Tests

Build the native library, then provide its directory for linking and loading:

```sh
swift test --disable-sandbox \
  -Xlinker -L -Xlinker "$AIMUX_LIBRARY_DIR" \
  -Xlinker -rpath -Xlinker "$AIMUX_LIBRARY_DIR"
```

This runs the bounded queue tests, wire checks and real-library checks for
manifest, provider/model ownership, Decision, error terminals, rejection and
push/poll validation. These native checks require no provider credentials or
network. Successful provider requests and realtime audio require a configured
provider. Recorder, session-store and inference configuration remain process
wide. Transport replay and credential-resolver creation are reserved native
capabilities; business-type schemas and per-platform packaging verification
are separate from this package's tests.

The synthetic C backend separately exercises callback/lifecycle races:

```sh
AIMUX_SWIFT_TEST_ABI=1 swift test --disable-sandbox \
  -Xswiftc -strict-concurrency=complete -Xswiftc -warnings-as-errors
```

For a working compiler with unavailable SwiftPM, direct runners are provided:

```sh
AIMUX_SWIFT_SDK=/path/to/MacOSX.sdk scripts/test-standalone.sh
AIMUX_SWIFT_SDK=/path/to/MacOSX.sdk \
  AIMUX_LIBRARY_DIR=/path/to/native/library scripts/test-native-standalone.sh
```

Direct runners use Swift 6 strict concurrency. iOS linking requires the iOS
native artifact and a full Xcode SDK; the macOS and synthetic checks do not
verify simulator linkage.

Run the shared DTO vectors from the repository root without a native library:

```sh
python3 scripts/check_dto_vectors.py swift
```
