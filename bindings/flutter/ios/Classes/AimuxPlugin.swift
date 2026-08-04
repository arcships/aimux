import Flutter
import UIKit

/// iOS plugin registration stub.
///
/// aimux is a Dart-only plugin (dart:ffi, no platform channels) — this class
/// exists so Flutter adds the plugin to the Podfile, which installs the
/// podspec and its vendored `aimux_ffi.xcframework` (with `-force_load`).
public class AimuxPlugin: NSObject, FlutterPlugin {
  public static func register(with registrar: FlutterPluginRegistrar) {
    // FFI-only — nothing to register.
  }
}
