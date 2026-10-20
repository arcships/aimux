#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .build/dto-vectors .build/dto-module-cache
sdk="${AIMUX_SWIFT_SDK:-$(xcrun --sdk macosx --show-sdk-path)}"
swiftc -sdk "$sdk" -swift-version 6 -strict-concurrency=complete -warnings-as-errors \
    -module-cache-path "$PWD/.build/dto-module-cache" -parse-as-library \
    generated/Types.swift generated/DtoVectorTypes.swift Tests/DtoVectors/main.swift \
    -o .build/dto-vectors/runner
.build/dto-vectors/runner ../../contract-tests/dto
