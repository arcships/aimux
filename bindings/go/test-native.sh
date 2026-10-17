#!/bin/sh
# Run against a directory containing the matching Rust shared library.
set -eu
cd "$(dirname "$0")"
: "${AIMUX_LIBRARY_DIR:?Set AIMUX_LIBRARY_DIR to the Rust library directory}"
export CGO_ENABLED=1
export CGO_LDFLAGS="-L$AIMUX_LIBRARY_DIR -Wl,-rpath,$AIMUX_LIBRARY_DIR ${CGO_LDFLAGS:-}"
unset AIMUX_TEST_ABI
go test -race -timeout 60s "$@" ./...
