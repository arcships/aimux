#!/usr/bin/env bash
# Check defined global symbols across every executable in an iOS app bundle.
set -euo pipefail
app=${1:?usage: check_symbols.sh path/to/Runner.app}
native=$(cd "$(dirname "$0")/../native" && pwd)
# The ABI is what the generated header declares plus the transport's EXPORTs.
required=($(sed -n 's/.*\(aimux_[a-z0-9_]*\)(.*/\1/p' "$native/aimux_ops.h"
  sed -n '/^EXPORT/s/.*\(aimux_dart_[a-z0-9_]*\)(.*/\1/p' "$native/aimux_dart.c"))
symbols=$(mktemp)
trap 'rm -f "$symbols"' EXIT
while IFS= read -r -d '' file; do
  # Undefined imports do not establish that force_load retained the ABI.
  nm -gU "$file" >> "$symbols" 2>/dev/null || true
done < <(find "$app" -type f -perm -111 -print0)
for symbol in "${required[@]}"; do
  if ! awk '{print $NF}' "$symbols" | grep -qx "_\{0,1\}$symbol"; then
    echo "FAIL: defined symbol $symbol missing from $app" >&2
    exit 1
  fi
done
echo "OK: all ${#required[@]} required aimux symbols present"
