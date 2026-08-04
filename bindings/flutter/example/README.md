# aimux_example

Demo app for the [aimux](https://pub.dev/packages/aimux) Flutter plugin —
unified LLM service layer backed by a Rust core (dart:ffi C ABI, no platform
channels).

## Run

```bash
flutter run
```

The demo calls `Model.openai(...)` against `http://localhost:3000` by
default — start any OpenAI-compatible mock (or the contract-test server in
this repository) and press **Generate**. Point the base URL at a real
provider and use a real API key for live calls.

This app is also the integration test bed for the plugin: CI builds it for
iOS (SwiftPM path, symbol check) and Android (APK, per-ABI `.so` check).
