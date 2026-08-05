#!/usr/bin/env bash
# local-ci-setup.sh — install the CI toolchain locally, no sudo required.
#
# Installs into ~/.local/opt + symlinks into ~/.local/bin, mirroring the
# versions used by .github/workflows/ci.yml:
#   - Go 1.24.x        (CI: go-version "1.24.x")
#   - Temurin JDK 21   (CI: distribution temurin, java-version 21)
#   - Gradle 8.14.3    (CI: gradle-version "8.14.3")
#   - Flutter stable   (CI: subosito/flutter-action — optional, large)
# Also fixes the common "cargo not in PATH" pitfall by sourcing
# ~/.cargo/env from ~/.bashrc (idempotent).
#
# Usage:
#   scripts/local-ci-setup.sh              # everything except flutter
#   scripts/local-ci-setup.sh --with-flutter
#   scripts/local-ci-setup.sh --skip-go --skip-java --skip-flutter
#   scripts/local-ci-setup.sh --skip-path  # don't touch shell config

set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
OPT="$PREFIX/opt"
BIN="$PREFIX/bin"
mkdir -p "$OPT" "$BIN"

SKIP_GO=0; SKIP_JAVA=0; SKIP_FLUTTER=1; SKIP_PATH=0
for arg in "$@"; do
  case "$arg" in
    --skip-go)    SKIP_GO=1 ;;
    --skip-java)  SKIP_JAVA=1 ;;
    --with-flutter) SKIP_FLUTTER=0 ;;
    --skip-flutter) SKIP_FLUTTER=1 ;;
    --skip-path)  SKIP_PATH=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

log()  { printf '\033[1;34m[setup]\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m[setup]\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m[setup]\033[0m %s\n' "$*" >&2; }

# ── Go 1.24 ────────────────────────────────────────────────────────────────
if [ "$SKIP_GO" -eq 0 ]; then
  if "$BIN/go" version 2>/dev/null | grep -q 'go1\.24'; then
    ok "Go already installed: $("$BIN/go" version)"
  else
    log "Downloading Go 1.24.11..."
    curl -fsSL -o /tmp/go.tar.gz 'https://dl.google.com/go/go1.24.11.linux-amd64.tar.gz'
    rm -rf "$OPT/go" && mkdir -p "$OPT/go"
    tar -C "$OPT/go" -xzf /tmp/go.tar.gz --strip-components=1
    ln -sf "$OPT/go/bin/go" "$BIN/go"
    ln -sf "$OPT/go/bin/gofmt" "$BIN/gofmt"
    ok "Go installed: $("$BIN/go" version)"
  fi
fi

# ── Temurin JDK 21 ─────────────────────────────────────────────────────────
if [ "$SKIP_JAVA" -eq 0 ]; then
  if "$BIN/java" -version 2>&1 | grep -q '21\.'; then
    ok "JDK already installed: $("$BIN/java" -version 2>&1 | head -1)"
  else
    log "Downloading Temurin JDK 21.0.12..."
    curl -fsSL -o /tmp/jdk.tar.gz \
      'https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12%2B8/OpenJDK21U-jdk_x64_linux_hotspot_21.0.12_8.tar.gz'
    rm -rf "$OPT/jdk" && mkdir -p "$OPT/jdk"
    tar -C "$OPT/jdk" -xzf /tmp/jdk.tar.gz --strip-components=1
    ln -sf "$OPT/jdk/bin/java" "$BIN/java"
    ln -sf "$OPT/jdk/bin/javac" "$BIN/javac"
    ln -sf "$OPT/jdk/bin/jar" "$BIN/jar"
    ok "JDK installed: $("$BIN/java" -version 2>&1 | head -1)"
  fi
fi

# ── Gradle 8.14.3 ──────────────────────────────────────────────────────────
if [ "$SKIP_JAVA" -eq 0 ]; then
  if "$BIN/gradle" --version 2>/dev/null | grep -q 'Gradle 8\.14'; then
    ok "Gradle already installed: $("$BIN/gradle" --version | grep Gradle)"
  else
    log "Downloading Gradle 8.14.3..."
    curl -fsSL -o /tmp/gradle.zip 'https://services.gradle.org/distributions/gradle-8.14.3-bin.zip'
    rm -rf "$OPT/gradle" && mkdir -p "$OPT/gradle"
    python3 -c "
import zipfile
zipfile.ZipFile('/tmp/gradle.zip').extractall('$OPT/gradle')
"
    chmod -R +x "$OPT/gradle"/gradle-8.14.3/bin
    ln -sf "$OPT/gradle"/gradle-8.14.3/bin/gradle "$BIN/gradle"
    ok "Gradle installed: $("$BIN/gradle" --version | grep Gradle)"
  fi
fi

# ── Flutter stable (optional, ~1.5 GB) ─────────────────────────────────────
if [ "$SKIP_FLUTTER" -eq 0 ]; then
  if "$BIN/flutter" --version 2>/dev/null | grep -q 'Flutter 3\.44'; then
    ok "Flutter already installed"
  else
    log "Downloading Flutter 3.44.8 stable (large)..."
    curl -fsSL -o /tmp/flutter.tar.xz \
      'https://storage.googleapis.com/flutter_infra_release/releases/stable/linux/flutter_linux_3.44.8-stable.tar.xz'
    rm -rf "$OPT/flutter" && mkdir -p "$OPT/flutter"
    tar -C "$OPT/flutter" -xJf /tmp/flutter.tar.xz --strip-components=1
    ln -sf "$OPT/flutter/bin/flutter" "$BIN/flutter"
    ln -sf "$OPT/flutter/bin/dart" "$BIN/dart"
    ok "Flutter installed: $("$BIN/flutter" --version 2>/dev/null | head -1)"
  fi
fi

# ── cargo PATH fix (idempotent) ────────────────────────────────────────────
if [ "$SKIP_PATH" -eq 0 ]; then
  if [ -f "$HOME/.cargo/env" ]; then
    for rc in "$HOME/.bashrc" "$HOME/.profile"; do
      if [ -f "$rc" ] && ! grep -q '\.cargo/env' "$rc" 2>/dev/null; then
        printf '\n# rustup (cargo/rustc)\n[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"\n' >> "$rc"
        ok "Added ~/.cargo/env to $rc"
      fi
    done
  else
    fail "~/.cargo/env not found — is rustup installed?"
  fi
fi

ok "Done. PATH entries ready in $BIN (add 'export PATH=\"$BIN:\$PATH\"' to your shell if missing)."
