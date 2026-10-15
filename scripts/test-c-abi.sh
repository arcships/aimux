#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:---run}
case "$mode" in
  --run|--asan|--asan-lsan|--compile-only|--asan-compile-only) ;;
  *) echo "Usage: $0 [--run|--asan|--asan-lsan|--compile-only|--asan-compile-only]" >&2; exit 2 ;;
esac
header=${AIMUX_C_HEADER_DIR:-"$root/aimux-ffi/include"}
if [[ ! -f "$header/aimux.h" ]]; then
  echo "ERROR: missing generated $header/aimux.h; no fallback header is used." >&2
  exit 1
fi
build=$(mktemp -d "${TMPDIR:-/tmp}/aimux-c-abi.XXXXXX")
trap 'rm -rf "$build"' EXIT
cc=${CC:-cc}
cxx=${CXX:-c++}
flags=(-Wall -Wextra -Werror -pedantic -g -O1 -pthread -I"$header" -I"$root/bindings/c" -I"$root/aimux-ffi/tests/c")
if [[ "$mode" == --asan || "$mode" == --asan-lsan || "$mode" == --asan-compile-only ]]; then
  flags+=(-fsanitize=address -fno-omit-frame-pointer)
fi
if [[ "$mode" == --asan-lsan ]]; then
  if [[ $(uname -s) != Linux ]]; then
    echo 'ERROR: --asan-lsan requires Linux LeakSanitizer; use --asan on macOS.' >&2
    exit 2
  fi
  flags+=(-fsanitize=leak)
  export ASAN_OPTIONS="${ASAN_OPTIONS:+$ASAN_OPTIONS:}detect_leaks=1:halt_on_error=1"
  export LSAN_OPTIONS="${LSAN_OPTIONS:+$LSAN_OPTIONS:}exitcode=23"
fi
sources=("aimux-ffi/tests/c/primitives.c" "aimux-ffi/tests/c/raii.cpp" "bindings/c/example.c" "bindings/c/example.cpp")
objects=()
for index in "${!sources[@]}"; do
  source=${sources[$index]}
  compiler=$cc
  standard=c11
  if [[ $source == *.cpp ]]; then compiler=$cxx; standard=c++17; fi
  object="$build/$index.o"
  "$compiler" -std="$standard" "${flags[@]}" -c "$root/$source" -o "$object"
  objects+=("$object")
  echo "COMPILED $source"
done
if [[ "$mode" == --compile-only || "$mode" == --asan-compile-only ]]; then
  echo 'COMPILE ONLY: no cdylib was linked, no test was run.'
  exit 0
fi
libdir=${AIMUX_C_LIB_DIR:-}
if [[ -z "$libdir" ]]; then
  # JSON artifact messages name the built library wherever CARGO_TARGET_DIR points.
  cargo build -p aimux-ffi --message-format=json-render-diagnostics > "$build/artifacts.jsonl"
  library=$(python3 - "$build/artifacts.jsonl" <<'PY'
import json, sys
libraries = []
for line in open(sys.argv[1]):
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get('reason') == 'compiler-artifact' and message.get('target', {}).get('name') == 'aimux_ffi':
        libraries.extend(p for p in message.get('filenames', []) if p.endswith(('.dylib', '.so', '.dll')))
if not libraries:
    sys.exit('ERROR: cargo produced no aimux-ffi cdylib artifact')
print(libraries[-1])
PY
)
  libdir=$(dirname "$library")
fi
# Retain the tested artifact while compiling and running C consumers.
mkdir -p "$build/native"
for filename in libaimux_ffi.dylib libaimux_ffi.so aimux_ffi.dll; do
  if [[ -f "$libdir/$filename" ]]; then cp "$libdir/$filename" "$build/native/$filename"; fi
done
libdir="$build/native"
failures=0
for index in "${!sources[@]}"; do
  compiler=$cc
  [[ ${sources[$index]} == *.cpp ]] && compiler=$cxx
  "$compiler" "${flags[@]}" "${objects[$index]}" -L"$libdir" -Wl,-rpath,"$libdir" -laimux_ffi -o "$build/test-$index"
  echo "LINKED ${sources[$index]}"
  if DYLD_LIBRARY_PATH="$libdir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" LD_LIBRARY_PATH="$libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$build/test-$index"; then
    echo "PASSED ${sources[$index]} ($mode)"
  else
    result=$?
    echo "FAILED ${sources[$index]} ($mode), exit $result" >&2
    failures=$((failures + 1))
  fi
done

if [[ $failures != 0 ]]; then
  echo "FAILED $failures / ${#sources[@]} C/C++ executables" >&2
  exit 1
fi
echo "PASSED ${#sources[@]} / ${#sources[@]} C/C++ executables"
