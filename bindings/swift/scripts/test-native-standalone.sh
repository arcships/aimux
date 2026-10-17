#!/bin/bash
# Runs the real-library checks directly when SwiftPM is unavailable.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${AIMUX_LIBRARY_DIR:?Set AIMUX_LIBRARY_DIR to the directory containing aimux_ffi}"
mkdir -p .build/native .build/native-module-cache
sdk="${AIMUX_SWIFT_SDK:-$(xcrun --sdk macosx --show-sdk-path)}"
flags=(-sdk "$sdk" -target "$(uname -m)-apple-macosx12.0" -module-cache-path "$PWD/.build/native-module-cache" -swift-version 6 -strict-concurrency=complete -warnings-as-errors)
swiftc "${flags[@]}" -emit-library -emit-module -enable-testing -module-name Aimux -I Sources/CAimuxFFI Sources/Aimux/*.swift generated/AimuxOps.swift -L "$AIMUX_LIBRARY_DIR" -laimux_ffi -Xlinker -rpath -Xlinker "$AIMUX_LIBRARY_DIR" -emit-module-path .build/native/Aimux.swiftmodule -o .build/native/libAimux.dylib
swiftc "${flags[@]}" -D AIMUX_STANDALONE -parse-as-library -I .build/native -I Sources/CAimuxFFI -L .build/native -lAimux -Xlinker -rpath -Xlinker "$PWD/.build/native" Tests/AimuxTests/{ContractTests,WireTests,NativeSmokeTests}.swift Tests/Standalone/NativeRunner.swift -o .build/native/NativeTests
.build/native/NativeTests
