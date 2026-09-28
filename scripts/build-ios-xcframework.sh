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
# pub.dev's package limit since 0.5.0). The ios-release profile turns LTO
# off for this build only; bitcode_strip below is belt-and-braces.
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build -p aimux-ffi --profile ios-release --target aarch64-apple-ios
cargo build -p aimux-ffi --profile ios-release --target aarch64-apple-ios-sim

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
  case "$triple" in
    aarch64-apple-ios) slice=ios ;;
    aarch64-apple-ios-sim) slice=sim ;;
  esac
  mkdir -p "$TMP/$slice/aimux_ffi.framework"
  cp "target/$triple/ios-release/libaimux_ffi.a" "$TMP/$slice/aimux_ffi.framework/aimux_ffi"
  # Strip debug + local symbols, keep globals (force_load in aimux.podspec
  # resolves against them). Then drop any __LLVM bitcode sections: Apple
  # tooling ignores them (App Store stopped accepting bitcode with Xcode 14)
  # and on the 0.3.0-era builds they were ~80% of each slice.
  strip -Sx "$TMP/$slice/aimux_ffi.framework/aimux_ffi"
  xcrun bitcode_strip -r -o "$TMP/$slice/aimux_ffi.framework/aimux_ffi" \
    "$TMP/$slice/aimux_ffi.framework/aimux_ffi"
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
echo "xcframework written to $OUT"
