// aimux.swift — Swift package entry point for the aimux Flutter plugin.
//
// aimux is a Dart-only plugin: no platform channels, no Swift API surface.
// This file exists only because SwiftPM requires at least one source file in
// the target. The native core (aimux-ffi) is vendored via the
// aimux_ffi binary target and linked with -all_load (see Package.swift), so
// Dart can resolve its symbols through DynamicLibrary.process().

import Foundation

// Empty by design — see file comment above.
