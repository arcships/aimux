// aimux_plugin.dart — Flutter plugin registration entry point.
//
// aimux is a Dart-only plugin: it calls the aimux-ffi C ABI directly via
// dart:ffi and uses no platform channels. `registerWith()` is a no-op that
// exists to satisfy Flutter's `dartPluginClass` contract — the flutter tool
// generates a call to it during app startup.
//
// The native libraries themselves ship inside this package:
//   - Android: android/src/main/jniLibs/<abi>/libaimux_ffi.so
//   - iOS:     ios/libaimux_ffi.a (device + simulator fat archive)

/// Dart-only plugin entry point required by pubspec.yaml's `dartPluginClass`.
class AimuxPlugin {
  /// Called by the generated plugin registrant at app startup.
  static void registerWith() {
    // FFI-only — nothing to register.
  }
}
