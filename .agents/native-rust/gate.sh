#!/bin/bash
# usage: gate.sh <worktree> <target> <log>
cd "$1" || exit 2
export CARGO_TARGET_DIR="$2" CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=2
{
echo "== fmt"; cargo fmt --all -- --check; echo "rc=$?"
echo "== clippy"; cargo clippy --workspace --all-targets -- -D warnings -A clippy::must_use_candidate 2>&1 | tail -5; echo "rc=${PIPESTATUS[0]}"
echo "== doc"; RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps 2>&1 | tail -3; echo "rc=${PIPESTATUS[0]}"
echo "== test"; cargo test --workspace > "$3.test" 2>&1; echo "rc=$?"
grep '^test result' "$3.test" | awk '{p+=$4; f+=$6; i+=$8} END {print NR" suites, passed "p", failed "f", ignored "i}'
grep -E '^test .* FAILED|panicked' "$3.test" | head
echo "== ts"; python3 scripts/gen_ts_types.py --check 2>&1 | tail -2; echo "rc=$?"
echo "== provdoc"; python3 scripts/gen_providers_doc.py --check 2>&1 | tail -2; echo "rc=$?"
echo "== boundaries"; bash scripts/check_provider_boundaries.sh 2>&1 | tail -3; echo "rc=${PIPESTATUS[0]}"
echo "== status"; git status --short | head
} > "$3" 2>&1
echo done >> "$3"
