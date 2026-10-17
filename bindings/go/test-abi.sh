#!/bin/sh
# Exercise the Go bridge against a deterministic C double. The generated
# header is shared with production; run test-native.sh for the Rust library.
set -eu
cd "$(dirname "$0")"
fixture_dir=$(pwd)/testdata/abi
header_dir=$(pwd)/../../aimux-ffi/include
fixture_build=$(mktemp -d "${TMPDIR:-/tmp}/aimux-go-abi.XXXXXX")
trap 'rm -rf "$fixture_build"' EXIT HUP INT TERM
cc -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -Werror -pthread -I"$header_dir" -c "$fixture_dir/fixture.c" -o "$fixture_build/fixture.o"
ar rcs "$fixture_build/libaimux_ffi.a" "$fixture_build/fixture.o"
export CGO_LDFLAGS="-L$fixture_build ${CGO_LDFLAGS:-}"
export AIMUX_TEST_ABI=1
export CGO_ENABLED=1
go test -race -timeout 60s "$@" ./...
