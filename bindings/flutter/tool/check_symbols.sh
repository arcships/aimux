#!/usr/bin/env bash
# Verify the embedded dynamic aimux_ffi.framework (all C ABI exports, MH_DYLIB,
# expected architectures, @rpath install_name) and the host app's aimux_dart_*
# bridge symbols and @executable_path/Frameworks rpath inside an iOS Runner.app.
set -euo pipefail

app=${1:?usage: check_symbols.sh path/to/Runner.app}
native=$(cd "$(dirname "$0")/../native" && pwd)

fw_dir="$app/Frameworks/aimux_ffi.framework"
fw_bin="$fw_dir/aimux_ffi"
fw_plist="$fw_dir/Info.plist"
expected_install_name="@rpath/aimux_ffi.framework/aimux_ffi"

test -f "$fw_bin" || {
  echo "FAIL: embedded dynamic framework binary missing: $fw_bin" >&2
  ls -la "$app/Frameworks" >&2 || true
  exit 1
}
test -f "$fw_plist" || {
  echo "FAIL: embedded dynamic framework Info.plist missing: $fw_plist" >&2
  exit 1
}

archs=($(lipo -archs "$fw_bin"))
if [ "${#archs[@]}" -eq 0 ]; then
  echo "FAIL: could not determine Mach-O architectures of $fw_bin" >&2
  exit 1
fi

abi_required=($(sed -n 's/.*\(aimux_[a-z0-9_]*\)(.*/\1/p' "$native/aimux_ops.h"))
bridge_required=($(sed -n '/^EXPORT/s/.*\(aimux_dart_[a-z0-9_]*\)(.*/\1/p' "$native/aimux_dart.c"))

fw_symbols=$(mktemp)
app_symbols=$(mktemp)
trap 'rm -f "$fw_symbols" "$app_symbols"' EXIT

for arch in "${archs[@]}"; do
  hdr="$(otool -arch "$arch" -hv "$fw_bin")"
  grep -q 'DYLIB' <<< "$hdr" || {
    echo "FAIL: $fw_bin ($arch) is not a Mach-O MH_DYLIB:" >&2
    echo "$hdr" >&2
    exit 1
  }
  install_name="$(otool -arch "$arch" -D "$fw_bin" | tail -n +2 | head -n 1 | xargs)"
  if [ "$install_name" != "$expected_install_name" ]; then
    echo "FAIL: $fw_bin ($arch) install_name is '$install_name', expected '$expected_install_name'" >&2
    exit 1
  fi
  nm -arch "$arch" -gU "$fw_bin" | awk '{print $NF}' > "$fw_symbols"
  for symbol in "${abi_required[@]}"; do
    if ! grep -qx "_\{0,1\}$symbol" "$fw_symbols"; then
      echo "FAIL: defined C ABI symbol $symbol missing from $fw_bin ($arch)" >&2
      exit 1
    fi
  done
done

has_rpath=0
while IFS= read -r -d '' file; do
  nm -gU "$file" 2>/dev/null | awk '{print $NF}' >> "$app_symbols" || true
  if otool -l "$file" 2>/dev/null | grep -Fq "@executable_path/Frameworks"; then
    has_rpath=1
  fi
done < <(find "$app" -path "$fw_dir" -prune -o -type f -perm -111 -print0)

for symbol in "${bridge_required[@]}"; do
  if ! grep -qx "_\{0,1\}$symbol" "$app_symbols"; then
    echo "FAIL: defined Dart bridge symbol $symbol missing from host app binaries in $app" >&2
    exit 1
  fi
done

if [ "$has_rpath" -ne 1 ]; then
  echo "FAIL: @executable_path/Frameworks rpath missing from host app binaries in $app" >&2
  exit 1
fi

echo "OK: embedded dynamic aimux_ffi.framework verified (archs: ${archs[*]}, ${#abi_required[@]} ABI exports, ${#bridge_required[@]} bridge symbols)"
