#!/usr/bin/env bash
# Build the aimux-ffi iOS static framework xcframework.
#
# Framework-type slices (not bare .a) so Xcode/SPM and CocoaPods link the
# Rust static library reliably. The xcframework is produced by
# xcodebuild -create-xcframework (hand-written Info.plists are rejected by
# Xcode's framework resolution). Slice names follow xcodebuild's convention
# (ios-arm64 / ios-arm64-simulator) — ios/aimux.podspec's force_load paths
# depend on them.
#
# Usage: scripts/build-ios-xcframework.sh <output-dir>
set -euo pipefail

OUT="${1:?usage: build-ios-xcframework.sh <output-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')"
cd "$ROOT"

# A staticlib is never linked here — the consumer's Xcode link step runs
# after LTO stops mattering — so the workspace's fat-LTO only bloats the
# archive: cargo passes -C embed-bitcode=yes and every codegen unit carries
# its full LLVM bitcode (~80% of each slice, ~256MB uncompressed past
# pub.dev's package limit since 0.5.0). The staticlib-release profile turns
# LTO off for this build only; the bitcode check and the size gate below
# fail loudly if that regresses. rustup's own sysroot objects still carry
# their bitcode (~9.7 MB per slice, RFC-0043 §15.7); rust-objcopy from the
# llvm-tools component strips those sections before the slice is packed.
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
rustup component add llvm-tools
# Match aimux.podspec's s.platform = :ios, '12.0' so cc-rs (ring) compiles for
# the declared minimum iOS version instead of the host runner's latest SDK.
IOS_MIN="${IPHONEOS_DEPLOYMENT_TARGET:-12.0}"
# One cargo invocation builds both slices in parallel (each crate is one
# codegen unit, so two sequential builds leave most cores idle).
IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN" \
  cargo build -p aimux-ffi --profile staticlib-release \
    --target aarch64-apple-ios --target aarch64-apple-ios-sim

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
  case "$triple" in
    aarch64-apple-ios) slice=ios ;;
    aarch64-apple-ios-sim) slice=sim ;;
  esac
  mkdir -p "$TMP/$slice/aimux_ffi.framework"
  python3 scripts/check_native_artifact.py --strip-bitcode "target/$triple/staticlib-release/libaimux_ffi.a"
  # Merge into one object rooted at the C API (+ _rust_eh_personality) and
  # strip debug + local symbols; global C ABI symbols stay for aimux.podspec's
  # force_load.
  python3 scripts/merge_archive.py "target/$triple/staticlib-release/libaimux_ffi.a" \
    --out "$TMP/$slice/aimux_ffi.framework/aimux_ffi"
  # Fail loudly on a size regression instead of letting a fat slice slip
  # through and blow pub.dev's 256MiB package limit again: a healthy slice
  # is ~15-30MB; an LTO/bitcode regression lands near 128MB.
  SLICE_BYTES=$(stat -f%z "$TMP/$slice/aimux_ffi.framework/aimux_ffi")
  # The byte count is the RFC-0043 measurement of this slice (§3.1 raw_bytes).
  echo "size $triple aimux_ffi.framework/aimux_ffi $SLICE_BYTES bytes"
  if [ "$SLICE_BYTES" -gt 67108864 ]; then
    echo "slice $triple is $SLICE_BYTES bytes (> 64MiB) — bitcode/LTO regression?" >&2
    exit 1
  fi
  cat > "$TMP/$slice/aimux_ffi.framework/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>aimux_ffi</string>
  <key>CFBundleIdentifier</key><string>ai.arcships.aimux.aimux_ffi</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>aimux_ffi</string>
  <key>CFBundlePackageType</key><string>FMWK</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>
EOF
done

rm -rf "$OUT"
xcodebuild -create-xcframework \
  -framework "$TMP/ios/aimux_ffi.framework" \
  -framework "$TMP/sim/aimux_ffi.framework" \
  -output "$OUT" >/dev/null

# Slice layout must match the podspec force_load paths.
test -f "$OUT/ios-arm64/aimux_ffi.framework/aimux_ffi"
test -f "$OUT/ios-arm64-simulator/aimux_ffi.framework/aimux_ffi"

# Link-smoke both packaged slices (RFC-0043 §5.3: CI's Flutter example only
# final-links the simulator slice, so verify device + simulator relocations and
# all 14 exported C ABI symbols against the packaged binaries before shipping).
EXPORTS="$(python3 -c 'import sys, pathlib; sys.path.insert(0, "scripts"); import merge_archive; print(" ".join(sorted(merge_archive.header_exports(pathlib.Path(".")))))')"
for entry in "iphoneos:ios-arm64:-miphoneos-version-min=$IOS_MIN" \
             "iphonesimulator:ios-arm64-simulator:-mios-simulator-version-min=$IOS_MIN"; do
  IFS=: read -r sdk slice_dir min_flag <<< "$entry"
  sdk_path="$(xcrun --sdk "$sdk" --show-sdk-path)"
  slice_bin="$OUT/$slice_dir/aimux_ffi.framework/aimux_ffi"
  smoke_out="$TMP/smoke-$slice_dir"
  xcrun --sdk "$sdk" clang -isysroot "$sdk_path" -arch arm64 "$min_flag" \
    -std=c11 -Wall -Wextra -Werror -Oz -Wl,-dead_strip \
    -I"$ROOT/aimux-ffi/include" -I"$ROOT/aimux-ffi/tests/c" \
    "$ROOT/aimux-ffi/tests/c/primitives.c" \
    -Xlinker -force_load -Xlinker "$slice_bin" -framework CoreFoundation -o "$smoke_out"
  nm_syms="$(nm -gU "$smoke_out" | awk '{print $NF}')"
  for sym in $EXPORTS; do
    grep -qx "_$sym" <<< "$nm_syms" || {
      echo "FAIL: defined symbol _$sym missing from $slice_dir link smoke" >&2
      exit 1
    }
  done
done
echo "xcframework written to $OUT"
