// aimux.swift — Swift package entry point for the aimux Flutter plugin.
//
// aimux is a Dart-only plugin: no platform channels, no Swift API surface.
// This file exists because SwiftPM requires at least one source file in the
// target. The native core (aimux-ffi) is vendored via the aimux_ffi binary
// target; the shim below anchors its symbols so Dart can resolve them
// through DynamicLibrary.process().

import Foundation
import aimux_ffi_shim

/// References the shim's symbol table so the linker pulls the aimux-ffi
/// static archive objects into the app binary. Never called at runtime —
/// it exists purely to create an undefined-symbol reference at link time.
@inline(never)
func aimuxKeepAlive() {
    _ = aimux_ffi_all_symbols
}
