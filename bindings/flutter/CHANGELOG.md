## 0.2.1

- First pub.dev release under publisher `arcships.ai`.
- Flutter plugin conversion: Android `libaimux_ffi.so` per ABI and iOS
  `aimux_ffi.xcframework` embedded in the package; Dart-only plugin
  (`dartPluginClass`), no platform channels.
- iOS integration via CocoaPods (`aimux.podspec`): vendored static
  framework slices staged by `script_phase`, symbols force-loaded into the
  app binary (`user_target_xcconfig`) — verified by CI symbol scan
  (issue #25). SwiftPM is not used (Flutter 3.44 cannot link plugin binary
  targets); privacy manifest (`PrivacyInfo.xcprivacy`) bundled.
- `example/` app demonstrating the API; CI builds it for iOS and Android
  to validate native integration.
