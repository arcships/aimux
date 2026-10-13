#!/usr/bin/env bash
# Usage: run.sh <worktree> <target-dir> <out.jsonl> [filter]
# Copies zz_diff_probe.rs into <worktree>/aimux-providers/tests and runs it.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
WT="$1"; TD="$2"; OUT="$3"; FILTER="${4:-}"
cp "$HERE/zz_diff_probe.rs" "$WT/aimux-providers/tests/zz_diff_probe.rs"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$TD"
# Dead proxy: any request escaping the scripted transport fails fast instead of reaching the network.
export HTTPS_PROXY=http://127.0.0.1:9 HTTP_PROXY=http://127.0.0.1:9 https_proxy=http://127.0.0.1:9 http_proxy=http://127.0.0.1:9 NO_PROXY= no_proxy=
cd "$WT"
PROBE_OUT="$OUT" PROBE_FILTER="$FILTER" cargo test -p aimux-providers --test zz_diff_probe -- --nocapture
# Diff: python3 -I "$HERE/diff.py" old.jsonl new.jsonl
