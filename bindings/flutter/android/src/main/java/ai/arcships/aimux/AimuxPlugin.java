package ai.arcships.aimux;

import io.flutter.embedding.engine.plugins.FlutterPlugin;

/**
 * Android plugin registration stub.
 *
 * <p>aimux is a Dart-only plugin (dart:ffi, no platform channels) — this class
 * exists so Flutter treats the plugin as an Android module, which is what
 * pulls the per-ABI {@code libaimux_ffi.so} files (android/src/main/jniLibs)
 * into the consuming app's APK.
 */
public class AimuxPlugin implements FlutterPlugin {
  @Override
  public void onAttachedToEngine(FlutterPluginBinding binding) {
    // FFI-only — nothing to register.
  }

  @Override
  public void onDetachedFromEngine(FlutterPluginBinding binding) {
    // FFI-only — nothing to release.
  }
}
