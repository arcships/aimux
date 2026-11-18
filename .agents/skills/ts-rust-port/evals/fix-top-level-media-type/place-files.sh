#!/usr/bin/env bash
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
# Shared setup: copies the crate and the profile, writes .eval/cargo.
exec "$here/../../../../native-rust/eval-scaffold.sh" "$here/files"
