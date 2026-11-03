# aimux_example

Demo app for the [aimux](https://pub.dev/packages/aimux) Flutter plugin —
an in-process LLM SDK backed by a Rust core (dart:ffi C ABI, no platform
channels).

## Run

```bash
flutter run
```

The demo opens the plugin and calls `runtime.describe`. It does not construct
a provider. `Model.openai(...)` was removed in 0.6.0; a live call uses
`provider.create` and `provider.model`. See [docs/api/flutter.md](../../../docs/api/flutter.md).

This app is also the integration test bed for the plugin: CI builds it for
iOS (SwiftPM path, symbol check) and Android (APK, per-ABI `.so` check).
