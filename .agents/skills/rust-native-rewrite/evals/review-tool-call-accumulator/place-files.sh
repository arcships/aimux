#!/usr/bin/env bash
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
# Same input as the rewrite case.
exec "$here/../../../../native-rust/eval-scaffold.sh" "$here/../rewrite-tool-call-accumulator/files"
