#!/usr/bin/env bash
# Build the aimux-ffi iOS dynamic framework xcframework (cdylib).
#
# Each slice packages target/<triple>/release/libaimux_ffi.dylib (fat-LTO,
# opt-level="z", panic="abort", stripped) as a standard dynamic Apple framework
# (`aimux_ffi.framework`) with install_name `@rpath/aimux_ffi.framework/aimux_ffi`,
# public C header, modulemap, and App Store-compliant Info.plist.
# The xcframework is produced by `xcodebuild -create-xcframework` with slices
# `ios-arm64` and `ios-arm64-simulator`.
#
# Usage: scripts/build-ios-xcframework.sh <output-dir>
set -euo pipefail

OUT="${1:?usage: build-ios-xcframework.sh <output-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version = ' "$ROOT/Cargo.toml" | sed 's/version = "\(.*\)"/\1/')"
cd "$ROOT"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Match aimux.podspec's s.platform = :ios, '12.0' so cc-rs (ring) and rustc
# target the declared minimum iOS version instead of the host runner's SDK.
# Apple Silicon iOS Simulator (arm64-apple-ios-sim) was introduced in iOS 14.0,
# so rustc/ld64 clamp the simulator slice's LC_BUILD_VERSION minos to >= 14.0.
IOS_MIN="${IPHONEOS_DEPLOYMENT_TARGET:-12.0}"
SIM_MIN="$(printf '%s\n14.0\n' "$IOS_MIN" | sort -V | tail -n 1)"
INSTALL_NAME="@rpath/aimux_ffi.framework/aimux_ffi"
# Fat-LTO + -Oz + strip dynamic slices land near ~8-11 MiB; fail loudly on
# any profile/LTO/strip regression well before pub.dev's package limit.
MAX_SLICE_BYTES=20971520

# One cargo invocation builds both slices in parallel. Target-scoped RUSTFLAGS
# apply only to target artifacts (never host build scripts/proc-macros) and
# rustc forwards link-arg only when linking the cdylib.
IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN" \
  CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS:+${CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS} }-C link-arg=-Wl,-install_name,$INSTALL_NAME" \
  CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS:+${CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS} }-C link-arg=-Wl,-install_name,$INSTALL_NAME" \
  cargo build -p aimux-ffi --release \
    --target aarch64-apple-ios --target aarch64-apple-ios-sim

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
  case "$triple" in
    aarch64-apple-ios)
      slice=ios
      supported_platform=iPhoneOS
      macho_platform=ios
      slice_min="$IOS_MIN"
      ;;
    aarch64-apple-ios-sim)
      slice=sim
      supported_platform=iPhoneSimulator
      macho_platform=ios-simulator
      slice_min="$SIM_MIN"
      ;;
  esac

  fw_dir="$TMP/$slice/aimux_ffi.framework"
  fw_bin="$fw_dir/aimux_ffi"
  mkdir -p "$fw_dir/Headers" "$fw_dir/Modules"

  src_dylib="target/$triple/release/libaimux_ffi.dylib"
  test -f "$src_dylib" || {
    echo "error: expected dynamic library not found at $src_dylib" >&2
    exit 1
  }
  cp "$src_dylib" "$fw_bin"
  chmod 755 "$fw_bin"

  # Fallback if caller's global RUSTFLAGS shadowed CARGO_TARGET_*_RUSTFLAGS.
  current_id="$(otool -D "$fw_bin" | tail -n +2 | head -n 1 | xargs)"
  if [ "$current_id" != "$INSTALL_NAME" ]; then
    install_name_tool -id "$INSTALL_NAME" "$fw_bin"
  fi

  cp "$ROOT/aimux-ffi/include/aimux.h" "$fw_dir/Headers/aimux.h"
  cat > "$fw_dir/Modules/module.modulemap" <<'EOF'
framework module aimux_ffi {
    umbrella header "aimux.h"
    export *
    module * { export * }
}
EOF

  cat > "$fw_dir/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>aimux_ffi</string>
  <key>CFBundleIdentifier</key><string>ai.arcships.aimux.aimux-ffi</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>aimux_ffi</string>
  <key>CFBundlePackageType</key><string>FMWK</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleSupportedPlatforms</key>
  <array>
    <string>$supported_platform</string>
  </array>
  <key>CFBundleVersion</key><string>1</string>
  <key>MinimumOSVersion</key><string>$slice_min</string>
  <key>UIDeviceFamily</key>
  <array>
    <integer>1</integer>
    <integer>2</integer>
  </array>
</dict></plist>
EOF

  python3 scripts/check_native_artifact.py \
    --expect-macho-dylib \
    --expect-arch arm64 \
    --expect-platform "$macho_platform" \
    --expect-min-os "$slice_min" \
    --expect-install-name "$INSTALL_NAME" \
    --expect-header-exports \
    --max-bytes "$MAX_SLICE_BYTES" \
    "$fw_bin"

  SLICE_BYTES=$(stat -f%z "$fw_bin")
  # The byte count is the RFC-0043 measurement of this slice (§3.1 raw_bytes).
  echo "size $triple aimux_ffi.framework/aimux_ffi $SLICE_BYTES bytes"
done

rm -rf "$OUT"
xcodebuild -create-xcframework \
  -framework "$TMP/ios/aimux_ffi.framework" \
  -framework "$TMP/sim/aimux_ffi.framework" \
  -output "$OUT" >/dev/null

# Verify expected slice layout inside the packaged xcframework.
for slice_dir in ios-arm64 ios-arm64-simulator; do
  test -f "$OUT/$slice_dir/aimux_ffi.framework/aimux_ffi"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Info.plist"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Headers/aimux.h"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Modules/module.modulemap"
done

# Dynamic link-smoke both packaged slices: CI's Flutter example only links the
# simulator slice, so verify device + simulator dynamic linkage, LC_LOAD_DYLIB
# install_name, and all 14 exported C ABI symbols before shipping.
for entry in "iphoneos:ios-arm64:ios:$IOS_MIN:-miphoneos-version-min=$IOS_MIN" \
             "iphonesimulator:ios-arm64-simulator:ios-simulator:$SIM_MIN:-mios-simulator-version-min=$SIM_MIN"; do
  IFS=: read -r sdk slice_dir macho_platform slice_min min_flag <<< "$entry"
  sdk_path="$(xcrun --sdk "$sdk" --show-sdk-path)"
  slice_bin="$OUT/$slice_dir/aimux_ffi.framework/aimux_ffi"
  smoke_out="$TMP/smoke-$slice_dir"

  python3 scripts/check_native_artifact.py \
    --expect-macho-dylib \
    --expect-arch arm64 \
    --expect-platform "$macho_platform" \
    --expect-min-os "$slice_min" \
    --expect-install-name "$INSTALL_NAME" \
    --expect-header-exports \
    --max-bytes "$MAX_SLICE_BYTES" \
    "$slice_bin"

  xcrun --sdk "$sdk" clang -isysroot "$sdk_path" -arch arm64 "$min_flag" \
    -std=c11 -Wall -Wextra -Werror -Oz -Wl,-dead_strip \
    -I"$ROOT/aimux-ffi/include" -I"$ROOT/aimux-ffi/tests/c" \
    "$ROOT/aimux-ffi/tests/c/primitives.c" \
    -F "$OUT/$slice_dir" -framework aimux_ffi \
    -Wl,-rpath,@executable_path/Frameworks \
    -framework CoreFoundation -o "$smoke_out"

  otool -L "$smoke_out" | grep -Fq "$INSTALL_NAME" || {
    echo "FAIL: $smoke_out does not dynamically link $INSTALL_NAME" >&2
    otool -L "$smoke_out" >&2
    exit 1
  }
done

echo "xcframework written to $OUT"
