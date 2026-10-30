#!/usr/bin/env bash
# Shared scaffold for the skills' `claude plugin eval` cases.
#
#   eval-scaffold.sh <case files dir>
#
# Runs in the empty eval workspace, as the person running the eval. Copies
# the case's crate and this profile in, writes `.eval/cargo` and fetches the
# crate's dependencies with it. The run gets a temporary HOME, so a bare
# `cargo` cannot find this machine's rustup toolchain; the wrapper points at
# it and builds offline.
set -eu
files="$1"
profile_dir="$(cd "$(dirname "$0")" && pwd)"

# The scaffold commits whatever is in the workspace as the baseline, so it
# must start in the empty directory the eval run created.
if [ -n "$(ls -A .)" ]; then
  echo "eval-scaffold.sh: $(pwd) is not empty; run it only from a case's place-files.sh" >&2
  exit 1
fi

cp -R "$files/." .
mkdir -p .agents/native-rust
cp "$profile_dir/PROFILE.md" .agents/native-rust/PROFILE.md

# HOME may already be temporary here, so find the toolchain from the cargo
# on PATH: rustup's proxy lives in $CARGO_HOME/bin, $RUSTUP_HOME beside it.
cargo_bin="$(command -v cargo)"
cargo_home="${CARGO_HOME:-$(cd "$(dirname "$cargo_bin")/.." && pwd)}"
rustup_home="${RUSTUP_HOME:-$(dirname "$cargo_home")/.rustup}"

mkdir -p .eval
cat > .eval/cargo <<WRAPPER
#!/usr/bin/env bash
export RUSTUP_HOME="$rustup_home" CARGO_HOME="$cargo_home"
export CARGO_NET_OFFLINE="\${CARGO_NET_OFFLINE:-true}"
export CARGO_TARGET_DIR="\${CARGO_TARGET_DIR:-\$PWD/target}"
exec "$cargo_home/bin/cargo" "\$@"
WRAPPER
chmod +x .eval/cargo
CARGO_NET_OFFLINE=false .eval/cargo fetch --quiet

git init -q
git config user.name eval
git config user.email eval@example.invalid
printf '.eval/\ntarget/\n' > .git/info/exclude
git add -A
git commit -qm baseline
