#!/bin/bash
# Runs exactly the contract checks used by SwiftPM, with the test-only C ABI.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .build/standalone .build/module-cache
sdk="${AIMUX_SWIFT_SDK:-$(xcrun --sdk macosx --show-sdk-path)}"
flags=(-sdk "$sdk" -target "$(uname -m)-apple-macosx12.0" -module-cache-path "$PWD/.build/module-cache" -swift-version 6 -strict-concurrency=complete -warnings-as-errors)
clang -isysroot "$sdk" -mmacosx-version-min=12.0 -I Tests/CAimuxTestABI/include -c Tests/CAimuxTestABI/mock.c -o .build/standalone/mock.o
swiftc "${flags[@]}" -emit-library -emit-module -enable-testing -module-name Aimux -I Sources/CAimuxFFI Sources/Aimux/*.swift generated/*.swift .build/standalone/mock.o -emit-module-path .build/standalone/Aimux.swiftmodule -o .build/standalone/libAimux.dylib
swiftc "${flags[@]}" -D AIMUX_STANDALONE -parse-as-library -I .build/standalone -I Sources/CAimuxFFI -I Tests/CAimuxTestABI/include -L .build/standalone -lAimux -Xlinker -rpath -Xlinker "$PWD/.build/standalone" Tests/AimuxTests/{ContractTests,WireTests}.swift Tests/Standalone/Runner.swift -o .build/standalone/ContractTests
.build/standalone/ContractTests
