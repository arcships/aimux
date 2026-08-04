# aimux · Flutter/Dart

Unified LLM service layer — one API to access 325 AI providers. The Rust
core (`aimux-ffi`) is called directly via `dart:ffi` (C ABI, no platform
channels, no codegen).

| | |
|---|---|
| Platforms | iOS, Android (plugin, native core embedded), plus Linux/macOS/Windows for dev & tests |
| Native core | `libaimux_ffi.so` per ABI (Android) / `aimux_ffi.xcframework` (iOS), shipped inside the package |
| Publisher | [`arcships.ai`](https://pub.dev/publishers/arcships.ai) |

## Install

```bash
flutter pub add aimux
```

The native libraries ship inside the package — no extra setup.

## Quick Start

```dart
import 'package:aimux/aimux.dart';

final model = Model.openai('sk-...', 'gpt-4o', baseUrl: 'http://localhost:3000');
final result = model.generateText('What is Rust?');
model.close();
```

## Dev / Tests

Desktop platforms resolve the library from the platform library path
(host builds), which is also how the test suite runs:

```bash
cargo build -p aimux-ffi --release
LD_LIBRARY_PATH=../../target/release flutter test   # Linux/macOS
```

## License

MIT — see the repository root LICENSE.
