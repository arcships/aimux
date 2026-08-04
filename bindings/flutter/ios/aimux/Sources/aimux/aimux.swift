// aimux.swift — Swift package entry point for the aimux Flutter plugin.
//
// aimux is a Dart-only plugin: no platform channels, no Swift API surface.
// This file exists because SwiftPM requires at least one source file in the
// target. The native core (aimux-ffi) is vendored via the aimux_ffi binary
// target; the anchor below keeps its symbols in the app binary so Dart can
// resolve them through DynamicLibrary.process().

import Foundation

// Symbol-table anchor defined in the aimux_ffi_shim C target (shim.c).
// Declared via @_silgen_name to avoid Clang-importing the incomplete array
// type from the header.
@_silgen_name("aimux_ffi_all_symbols")
private var aimuxFFIAllSymbols: UnsafeMutableRawPointer?

/// References the shim's symbol table so the linker pulls the aimux-ffi
/// static archive objects into the app binary. Never called at runtime —
/// it exists purely to create an undefined-symbol reference at link time.
@inline(never)
func aimuxKeepAlive() {
    _ = aimuxFFIAllSymbols
}
