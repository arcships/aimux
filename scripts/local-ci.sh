#!/usr/bin/env bash
# local-ci.sh — run the CI-equivalent quality gates locally, in one command.
#
# Mirrors .github/workflows/ci.yml (host jobs only; cross-platform matrix
# targets and Flutter example builds stay in CI):
#   rust      cargo fmt + clippy -D warnings + cargo test --workspace
#   contract  ProviderName drift + Rust contract tests + Node run-node.ts
#   python    maturin develop --release + pytest (bindings/python venv)
#   node      npm ci + napi build + npm test + tsc typecheck
#   go        go vet + go test (needs local-ci-setup.sh)
#   java      gradle test (needs local-ci-setup.sh; LD_LIBRARY_PATH set)
#   flutter   flutter test + analyze (needs --with-flutter setup)
#
# Usage:
#   scripts/local-ci.sh                 # everything installed
#   scripts/local-ci.sh --only=rust,contract
#   scripts/local-ci.sh --skip=node,flutter
#   scripts/local-ci.sh --quick         # rust + contract only
#
# Exit code 0 = all gates green. Stops at the first failing gate.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

ONLY=""; SKIP=""; QUICK=0
for arg in "$@"; do
  case "$arg" in
    --only=*)   ONLY="${arg#--only=}" ;;
    --skip=*)   SKIP="${arg#--skip=}" ;;
    --quick)    QUICK=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done
[ "$QUICK" -eq 1 ] && { ONLY="${ONLY:-rust,contract}"; }

want() { # want <stage>
  local stage="$1"
  if [ -n "$ONLY" ]; then
    case ",$ONLY," in *",$stage,"*) return 0 ;; *) return 1 ;; esac
  fi
  case ",$SKIP," in *",$stage,"*) return 1 ;; *) return 0 ;; esac
}

PASS=0; FAIL=0
pass() { PASS=$((PASS+1)); printf '\033[1;32m✔ %s\033[0m\n' "$1"; }
fail() { FAIL=$((FAIL+1)); printf '\033[1;31m✘ %s\033[0m\n' "$1"; }

section() { printf '\n\033[1;36m══ %s ══\033[0m\n' "$1"; }
need() { # need <cmd> <hint>
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "$2 (missing $1 — run scripts/local-ci-setup.sh)"
    return 1
  fi
  return 0
}

cd "$REPO"

# ── rust ───────────────────────────────────────────────────────────────────
if want rust; then
  section "rust: fmt + clippy + workspace tests"
  if cargo fmt --all -- --check \
     && cargo clippy --workspace --all-targets -- -D warnings \
     && cargo test --workspace --quiet; then
    pass "rust"
  else
    fail "rust"
  fi
fi

# ── contract ───────────────────────────────────────────────────────────────
if want contract; then
  section "contract: ProviderName drift + Rust/Node contract tests"
  if python3 scripts/gen_provider_names.py --check \
     && cargo test --test contract_test -p aimux-core --quiet \
     && node --experimental-strip-types contract-tests/run-node.ts | grep -q '0 failed'; then
    pass "contract"
  else
    fail "contract"
  fi
fi

# ── python ─────────────────────────────────────────────────────────────────
if want python; then
  section "python: maturin develop + pytest"
  if need maturin "python" \
     && (cd bindings/python && maturin develop --release --quiet \
         && .venv/bin/python -m pytest tests/ -q --tb=short); then
    pass "python"
  else
    fail "python"
  fi
fi

# ── node ───────────────────────────────────────────────────────────────────
if want node; then
  section "node: npm ci + napi build + ava + tsc"
  if need npm "node" \
     && (cd bindings/node && npm ci --no-audit --no-fund --silent \
         && npm run build --silent \
         && npm test 2>&1 | grep -q 'tests passed' \
         && npm run build:typed --silent); then
    pass "node"
  else
    fail "node"
  fi
fi

# ── go ─────────────────────────────────────────────────────────────────────
if want go; then
  section "go: vet + test (CGO_LDFLAGS -> target/release)"
  if need go "go" \
     && cargo build -p aimux-ffi --release --quiet \
     && (cd bindings/go && export CGO_LDFLAGS="-L$REPO/target/release" \
         && go vet ./... \
         && go test ./...); then
    pass "go"
  else
    fail "go"
  fi
fi

# ── java ───────────────────────────────────────────────────────────────────
if want java; then
  section "java: gradle test (LD_LIBRARY_PATH -> target/release)"
  if need gradle "java" && need java "java" \
     && cargo build -p aimux-ffi --release --quiet \
     && (cd bindings/java && LD_LIBRARY_PATH="$REPO/target/release" gradle test --quiet); then
    pass "java"
  else
    fail "java"
  fi
fi

# ── flutter ────────────────────────────────────────────────────────────────
if want flutter; then
  section "flutter: test + analyze"
  if need flutter "flutter" \
     && cargo build -p aimux-ffi --release --quiet \
     && (cd bindings/flutter && export LD_LIBRARY_PATH="$REPO/target/release" \
         && flutter pub get \
         && flutter test \
         && flutter analyze); then
    pass "flutter"
  else
    fail "flutter"
  fi
fi

# ── summary ────────────────────────────────────────────────────────────────
printf '\n\033[1;36m══════════════════════════════════\033[0m\n'
printf 'local-ci: %s passed, %s failed\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] && printf '\033[1;32mALL GATES GREEN\033[0m\n' || exit 1
