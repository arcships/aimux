# aimux · Flutter/Dart API

> **0.6.0.** This binding uses ABI 1 / ops 1.0. The previous unversioned
> constructors and per-feature native exports are removed. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md).

The Flutter plugin wraps the Rust core through the C ABI with `dart:ffi`. The
native core ships inside the pub.dev package: Android `.so` per ABI and the iOS
XCFramework. iOS integrates through CocoaPods, because Flutter's SwiftPM
integration does not link plugin binary targets as of Flutter 3.44.

## Install

```bash
flutter pub add aimux
```

The package is a Flutter plugin; no extra download or build step is needed for
Android/iOS. For a local repo dependency:

```yaml
dependencies:
  aimux:
    path: ../aimux/bindings/flutter
```

## Quick start

```dart
import 'package:aimux/aimux.dart';

final client = await Aimux.open(requiredOps: ['runtime.describe']);
final description = await client.call('runtime.describe');
print(description);

final providerJson = await client.call('provider.create', params: {
  'package': 'openai',
  'settings': {'apiKey': 'your-key'},
}) as Map<String, Object?>;
final providerHandle = client.adopt(providerJson['handle'] as String);

final modelJson = await client.call('provider.model', handle: providerHandle, params: {
  'method': 'languageModel',
  'modelId': 'gpt-4o-mini',
}) as Map<String, Object?>;
final modelHandle = client.adopt(modelJson['handle'] as String);

final result = await client.call('text.generate', handle: modelHandle, params: {
  'prompt': 'Hello',
  'options': {},
}) as Map<String, Object?>;
print(result['text']);

await modelHandle.close();
await providerHandle.close();
```

`call(op, handle:, params:, abort:)` accepts the canonical request DTO from the
manifest. Global operations omit `handle`. `Aimux.open()` checks the ABI major,
ops major, manifest structure and required operations before use.

## Handles and streams

- Adopt each returned handle exactly once with `client.adopt(...)` and
  `await handle.close()` when finished.
- All wire handles stay decimal strings, including values above 2^53.
- Native calls use uint64 handles, never pointer addresses.
- Streams expose `StreamFrameEvent` and an explicit `StreamFrameEnd`. A
  rejected start is a stream error with no frames; after acceptance, the end
  frame describes success, error or cancellation.
- A `TypedModel` projects events into typed results and throws the terminal
  error once. Recoverable error events remain data.
- Pausing withholds native credit; cancelling signals abort even while paused.

## Cancellation and isolates

Calls, manifest loading, push, poll and resource drop run on background
isolates. Futures do not implicitly cancel native work: create an
`abort.create` handle, pass it to `call`, and use `client.abort(abort)` to
signal cancellation. Dropping an abort handle does not signal it.

Each stream owns its bridge and creates an abort handle unless the request
provides one. Supplied abort handles remain caller-owned. Native abort and the
small bridge control functions are nonblocking.

## Errors

`OpsException` preserves the complete error envelope, including unknown payload
names and fields. AI SDK payloads keep `name`, `isRetryable` and
`responseHeaders`; read `retry-after` there. `cause` is the payload's cause: a
nested AI SDK error, or an OS/TLS/parser error as `{message, cause?}`. A retry
error's cause is its last attempt.

## Breaking changes in 0.6

- `Aimux.open()` replaces the unversioned constructor entrypoints.
- Provider-specific constructors are removed; use `provider.create` +
  `provider.model`.
- Streams use the explicit ops frame model; the old per-feature native exports
  are gone.
- DTOs and operation names come from the manifest; the package does not
  maintain a second provider factory table.
- Errors are the shared envelope with a native cause chain.

## See also

- [bindings/flutter/README.md](../../bindings/flutter/README.md) — plugin
  packaging, stream bridge and test details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Flutter provider list](providers.md).
