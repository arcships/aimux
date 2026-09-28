## 0.5.0

- Breaking release aligning the whole aimux family — see the
  [main CHANGELOG](https://github.com/arcships/aimux/blob/master/CHANGELOG.md)
  for the complete list (stable cross-language error model, request
  pipeline retries/timeouts, Core-boundary tool-input validation, host-side
  `repairToolCall`, WS proxy, SSRF hardening).
- Flutter binding: typed error model with retry context, host-side
  `repairToolCall`, `VideoPollOptions`.
- The embedded iOS `aimux_ffi.xcframework` no longer carries LTO bitcode /
  debug symbols: each slice shrinks from ~128MB to ~15MB (the 0.3.0
  archives had grown past pub.dev's 256MB package limit).

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
