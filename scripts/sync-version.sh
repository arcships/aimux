#!/usr/bin/env bash
# Sync the workspace version (single source of truth: Cargo.toml) to all
# language packages before a release. Run from the repo root:
#
#   ./scripts/sync-version.sh
#
# Go needs no version bump: the module version is the git tag.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
if [[ -z "$VERSION" ]]; then
  echo "ERROR: could not read version from Cargo.toml" >&2
  exit 1
fi
echo "Workspace version: $VERSION"

# Node
node -e "
const fs = require('fs');
const p = JSON.parse(fs.readFileSync('bindings/node/package.json', 'utf8'));
p.version = '$VERSION';
fs.writeFileSync('bindings/node/package.json', JSON.stringify(p, null, 2) + '\n');
console.log('  Node    -> bindings/node/package.json');
"

# Python
sed -i -E "s/^version = .*/version = \"$VERSION\"/" bindings/python/pyproject.toml
echo "  Python  -> bindings/python/pyproject.toml"

# Flutter
sed -i -E "s/^version: .*/version: $VERSION/" bindings/flutter/pubspec.yaml
echo "  Flutter -> bindings/flutter/pubspec.yaml"

# Kotlin
sed -i -E "s/^version = .*/version = \"$VERSION\"/" bindings/kotlin/build.gradle.kts
echo "  Kotlin  -> bindings/kotlin/build.gradle.kts"

# Java
sed -i -E "s/^version = .*/version = \"$VERSION\"/" bindings/java/build.gradle.kts
echo "  Java    -> bindings/java/build.gradle.kts"

echo "Done. Go is driven by the git tag — no change needed."
