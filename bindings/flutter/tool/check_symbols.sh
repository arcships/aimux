#!/usr/bin/env bash
# Check defined global symbols across every executable in an iOS app bundle.
set -euo pipefail
app=${1:?usage: check_symbols.sh path/to/Runner.app}
required=(aimux_abi_version aimux_manifest_json aimux_call aimux_stream
  aimux_operation_push aimux_operation_next_part aimux_abort aimux_drop
  aimux_handle_finalizer aimux_free_string aimux_error_free aimux_error_code aimux_error_message
  aimux_error_json aimux_dart_finalize aimux_dart_token_new aimux_dart_token_dispose
  aimux_dart_bridge_new aimux_dart_bridge_event aimux_dart_bridge_ack
  aimux_dart_bridge_cancel aimux_dart_bridge_pending aimux_dart_bridge_failure
  aimux_dart_bridge_free aimux_dart_frame_free)
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
