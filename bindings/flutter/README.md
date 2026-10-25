# aimux for Flutter / Dart

The binding uses the versioned C ops ABI. `Aimux.open()` checks the ABI
major, ops major, manifest structure and any required operations before use.
The previous unversioned constructors and per-feature native exports are removed.

```dart
import 'package:aimux/aimux.dart';

final client = await Aimux.open(requiredOps: ['runtime.describe']);
final description = await client.call('runtime.describe');
print(description);
```

`call(op, handle: ..., params: ..., abort: ...)` accepts the canonical request DTO
from the manifest. Global operations omit `handle`. Provider settings, model
methods and request fields come from the runtime descriptor; this package does
not maintain a second provider factory/config table. Adopt each returned handle
exactly once with `client.adopt(result['handle'] as String)` and `await handle.close()`
when finished. All wire handles stay decimal strings, including values above
2^53 and the full uint64 range. Native calls use uint64, never a pointer address.

Calls, manifest loading, push, poll and resource drop execute on background
isolates. Futures do not implicitly cancel native work: create an `abort.create`
handle, pass it to `call`, and use `client.abort(abort)` to signal cancellation.
Dropping an abort handle does not signal it. Native abort and the small bridge
control functions are nonblocking and may run on the caller isolate.

```dart
// `model` is an adopted Language handle; params follows text.stream's schema.
final subscription = client.stream('text.stream', handle: model, params: params)
    .listen((frame) {
  if (frame is StreamFrameEvent) print(frame.event);
  if (frame is StreamFrameEnd) print('${frame.outcome}: ${frame.error}');
});
// Pausing withholds native credit; cancelling signals abort even while paused.
subscription.pause();
subscription.resume();
await subscription.cancel();
```

Each stream owns its bridge and creates an abort handle unless the request
provides `abort` as a decimal handle string. Supplied abort handles remain
caller-owned; cancellation signals the same handle. The native C callback copies a frame and
posts through `NativeCallable.listener` to its owning isolate; it never invokes
a Dart function on a Rust thread. A callback waits for an ack after consumer
delivery. The C bridge holds at most one frame and the isolate transport grants one
event credit (each frame is at most 1 MiB), plus an independent terminal slot.
Pause withholds event credit. Cancel wakes the producer
without needing credit. The bridge and listener remain alive until both native
return and the final queued notification have completed. Single-subscription
streams start only on listen.

A rejected start is a stream error with no frames. After acceptance, raw streams
expose the explicit `StreamFrameEnd` (success/error/cancelled); an error in that terminal
is not thrown a second time. `TypedModel` projects events into existing typed
results and throws the terminal error once. Recoverable error *events* stay data.
Host `onData` handlers run under Dart's normal Stream/Zone error rules. Internal
copy/decode failures are latched once, signal abort, and are reported after native
return. No callback exception crosses the C boundary.

`OpsException` preserves the complete error envelope, including unknown payload
names/fields. AI SDK payloads keep their `name`, flat fields, `isRetryable` and
`responseHeaders`; read `retry-after` there, as with the AI SDK. The binding does
not invent HTTP status or retry policy.
`cause` is the payload's cause: a nested AI SDK error, or an OS, TLS or parser
error as `{message, cause?}`. A retry error's cause is its last attempt.
`TranscriptionOperation` wraps the new push/next primitives and typed
part/ended/timeout states. `ToolCallRepair` exposes the three pure DTO ops.
`DecisionModel` exposes `decision.decide` and `decision.capabilities` over an
adopted Decision handle. Create its provider through `provider.create` and use
`provider.model` with method `decision`.

Explicit close clears the local handle, detaches `NativeFinalizer`, frees its
independent token, and schedules drop on a worker isolate. Repeated close returns
the same Future. Forgotten handles use a native `void(void*)` token trampoline:
it queues cleanup to a native reaper thread, without running Dart or blocking
on native drop in the GC callback. Finalization is a leak safeguard; it does not
flush a recorder or complete an operation normally.

DTOs are generated from the Rust manifest into `lib/src/generated/types.dart`.
Import `package:aimux/types.dart` for immutable classes, enums and union variants.
`Field<T>` keeps an absent field distinct from an explicit JSON null. Integer
fields, including Rust `u64` ones, are Dart `int` (64-bit signed on the VM), so a
`u64` above 2^63-1 cannot be held; `ErrorEnvelope/001.json` in the DTO vectors is
the one vector skipped for this reason. Handle and sequence ids stay decimal
strings in the DTOs, and `_u64String` rejects a string outside the `u64` range.
There is no `BigInt` or custom JSON codec in the DTOs: they use `dart:convert`.
Unknown union tags raise `FormatException` with the type and tag.

## Native packaging

Android retains the prebuilt `android/src/main/jniLibs/<abi>/libaimux_ffi.so`
loader and builds `libaimux_dart.so` with the NDK/CMake. iOS retains
`DynamicLibrary.process()` and force-loads the Rust xcframework; the CocoaPods
plugin is static and retains every bridge symbol via explicit linker roots.
The iOS CI `tool/check_symbols.sh` gate verifies all defined ABI and bridge symbols
across app executables, including `Runner.debug.dylib`.

For desktop use, build `native/CMakeLists.txt` and place the resulting
`libaimux_dart.dylib`, `libaimux_dart.so`, or `aimux_dart.dll` beside the Rust
library on the loader path. Explicit `libraryPath` and `bridgePath` overrides are
available for development/tests; iOS normally uses process lookup for both.
The stream credit transport currently requires POSIX pipes (Android, iOS, macOS
and Linux); Windows streaming is unsupported.

aimux-codegen owns the packaged `native/aimux_ops.h`. `tool/generate_ffi.py`
generates Dart declarations from that header and checks all 14 ABI primitives,
including the nonblocking native handle finalizer. Dart uses its own allocated
uint64 token trampoline to preserve handles on 32-bit targets.
No old ABI is accepted.

## Verification

```sh
python3 tool/generate_ffi.py --check
python3 tool/build_test_fixture.py
mise exec java@temurin-17 gradle@8 flutter@3 -- dart pub get --offline
mise exec java@temurin-17 gradle@8 flutter@3 -- dart analyze lib test
mise exec java@temurin-17 gradle@8 flutter@3 -- dart test
mise exec java@temurin-17 gradle@8 flutter@3 -- flutter test
```

The native test double checks isolate responsiveness, arbitrary native callback
threads, lossless ordered streams, pause/resume, cancellation at zero credit,
concurrent streams, rejection/terminal separation, malformed/oversized frames,
uint64 tokens, explicit close, the reaper trampoline, and binary push/poll. It
requires a POSIX C compiler and is not a Rust/provider integration test.
To enable the separate real-library smoke test, set `AIMUX_NATIVE_LIBRARY` and
`AIMUX_DART_BRIDGE` to new-ABI artifacts. Android ABI packaging, iOS simulator
linking and real GC timing need their platform/runtime checks; host fixture tests
do not establish that those pass.
