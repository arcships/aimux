## 0.2.0

- First pub.dev release under publisher `arcships.ai`.
- Flutter plugin conversion: Android `libaimux_ffi.so` per ABI and iOS
  `aimux_ffi.xcframework` embedded in the package; Dart-only plugin
  (`dartPluginClass`), no platform channels.
- iOS integration: SwiftPM (`ios/aimux/Package.swift`, binary target +
  `-all_load`) and CocoaPods fallback (`aimux.podspec`, vendored +
  `-force_load`); privacy manifest (`PrivacyInfo.xcprivacy`) bundled.
- `example/` app demonstrating the API; CI builds it for iOS and Android
  to validate native integration.
