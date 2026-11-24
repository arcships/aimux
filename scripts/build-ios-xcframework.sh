#!/usr/bin/env bash
# Build the aimux-ffi iOS dynamic framework xcframework (cdylib).
#
# Packages target/<triple>/release/libaimux_ffi.dylib (fat-LTO, opt-level="z",
# panic="abort", stripped) as standard dynamic Apple frameworks with
# install_name `@rpath/aimux_ffi.framework/aimux_ffi`, public C header,
# modulemap, and App Store-compliant Info.plist:
#   - ios-arm64: single-arch arm64 device slice (shipped in iOS IPAs)
#   - ios-arm64_x86_64-simulator: universal simulator slice (arm64 + x86_64,
#     required so Xcode generic simulator builds and CocoaPods'
#     [CP] Copy XCFrameworks match cleanly without EXCLUDED_ARCHS hacks)
#
# Usage: scripts/build-ios-xcframework.sh <output-dir>
set -euo pipefail

OUT="${1:?usage: build-ios-xcframework.sh <output-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version = ' "$ROOT/Cargo.toml" | sed 's/version = "\(.*\)"/\1/')"
cd "$ROOT"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# Match aimux.podspec's s.platform = :ios, '12.0' so cc-rs (ring) and rustc
# target the declared minimum iOS version instead of the host runner's SDK.
# Apple Silicon iOS Simulator (arm64-apple-ios-sim) was introduced in iOS 14.0,
# so rustc/ld64 clamp its LC_BUILD_VERSION minos to >= 14.0.
IOS_MIN="${IPHONEOS_DEPLOYMENT_TARGET:-12.0}"
SIM_ARM64_MIN="$(printf '%s\n14.0\n' "$IOS_MIN" | sort -V | tail -n 1)"
INSTALL_NAME="@rpath/aimux_ffi.framework/aimux_ffi"
# Fat-LTO + -Oz + strip single-arch slices land near ~8-11 MiB; fail loudly on
# any profile/LTO/strip regression well before pub.dev's package limit.
MAX_ARCH_BYTES=20971520
MAX_FAT_SIM_BYTES=41943040

# One cargo invocation builds all three slices in parallel. Target-scoped
# RUSTFLAGS apply only to target artifacts (never host build scripts /
# proc-macros) and rustc forwards link-arg only when linking the cdylib.
IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN" \
  CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS:+${CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS} }-C link-arg=-Wl,-install_name,$INSTALL_NAME" \
  CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="${CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS:+${CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS} }-C link-arg=-Wl,-install_name,$INSTALL_NAME" \
  CARGO_TARGET_X86_64_APPLE_IOS_RUSTFLAGS="${CARGO_TARGET_X86_64_APPLE_IOS_RUSTFLAGS:+${CARGO_TARGET_X86_64_APPLE_IOS_RUSTFLAGS} }-C link-arg=-Wl,-install_name,$INSTALL_NAME" \
  cargo build -p aimux-ffi --release \
    --target aarch64-apple-ios \
    --target aarch64-apple-ios-sim \
    --target x86_64-apple-ios

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

prepare_dylib() {
  local triple="$1" arch="$2" macho_platform="$3" min_os="$4" dest="$5"
  local src="target/$triple/release/libaimux_ffi.dylib"
  test -f "$src" || {
    echo "error: expected dynamic library not found at $src" >&2
    exit 1
  }
  cp "$src" "$dest"
  chmod 755 "$dest"
  # Fallback if caller's global RUSTFLAGS shadowed CARGO_TARGET_*_RUSTFLAGS.
  local current_id
  current_id="$(otool -D "$dest" | tail -n +2 | head -n 1 | xargs)"
  if [ "$current_id" != "$INSTALL_NAME" ]; then
    install_name_tool -id "$INSTALL_NAME" "$dest"
  fi
  python3 scripts/check_native_artifact.py \
    --expect-macho-dylib \
    --expect-arch "$arch" \
    --expect-platform "$macho_platform" \
    --expect-min-os "$min_os" \
    --expect-install-name "$INSTALL_NAME" \
    --expect-header-exports \
    --max-bytes "$MAX_ARCH_BYTES" \
    "$dest"
  local bytes
  bytes="$(stat -f%z "$dest")"
  # The byte count is the RFC-0043 measurement of this slice (§3.1 raw_bytes).
  echo "size $triple aimux_ffi.framework/aimux_ffi $bytes bytes"
}

write_framework_metadata() {
  local fw_dir="$1" supported_platform="$2" min_os="$3"
  mkdir -p "$fw_dir/Headers" "$fw_dir/Modules"
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
  <key>MinimumOSVersion</key><string>$min_os</string>
  <key>UIDeviceFamily</key>
  <array>
    <integer>1</integer>
    <integer>2</integer>
  </array>
</dict></plist>
EOF
}

mkdir -p "$TMP/ios/aimux_ffi.framework" "$TMP/sim/aimux_ffi.framework" "$TMP/sim-archs"

prepare_dylib aarch64-apple-ios     arm64  ios           "$IOS_MIN"       "$TMP/ios/aimux_ffi.framework/aimux_ffi"
prepare_dylib aarch64-apple-ios-sim arm64  ios-simulator "$SIM_ARM64_MIN" "$TMP/sim-archs/aimux_ffi-arm64"
prepare_dylib x86_64-apple-ios      x86_64 ios-simulator "$IOS_MIN"       "$TMP/sim-archs/aimux_ffi-x86_64"

lipo -create \
  "$TMP/sim-archs/aimux_ffi-arm64" \
  "$TMP/sim-archs/aimux_ffi-x86_64" \
  -output "$TMP/sim/aimux_ffi.framework/aimux_ffi"
chmod 755 "$TMP/sim/aimux_ffi.framework/aimux_ffi"

write_framework_metadata "$TMP/ios/aimux_ffi.framework" iPhoneOS        "$IOS_MIN"
write_framework_metadata "$TMP/sim/aimux_ffi.framework" iPhoneSimulator "$IOS_MIN"

rm -rf "$OUT"
xcodebuild -create-xcframework \
  -framework "$TMP/ios/aimux_ffi.framework" \
  -framework "$TMP/sim/aimux_ffi.framework" \
  -output "$OUT" >/dev/null

# Verify expected slice layout inside the packaged xcframework.
for slice_dir in ios-arm64 ios-arm64_x86_64-simulator; do
  test -f "$OUT/$slice_dir/aimux_ffi.framework/aimux_ffi"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Info.plist"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Headers/aimux.h"
  test -f "$OUT/$slice_dir/aimux_ffi.framework/Modules/module.modulemap"
done

python3 scripts/check_native_artifact.py \
  --expect-macho-dylib \
  --expect-arch arm64 \
  --expect-platform ios \
  --expect-min-os "$IOS_MIN" \
  --expect-install-name "$INSTALL_NAME" \
  --expect-header-exports \
  --max-bytes "$MAX_ARCH_BYTES" \
  "$OUT/ios-arm64/aimux_ffi.framework/aimux_ffi"

python3 scripts/check_native_artifact.py \
  --expect-macho-dylib \
  --expect-arch arm64,x86_64 \
  --expect-platform ios-simulator \
  --expect-install-name "$INSTALL_NAME" \
  --expect-header-exports \
  --max-bytes "$MAX_FAT_SIM_BYTES" \
  "$OUT/ios-arm64_x86_64-simulator/aimux_ffi.framework/aimux_ffi"

# Dynamic link-smoke all packaged architectures (device arm64 + simulator arm64
# and x86_64) against primitives.c before shipping.
for entry in "iphoneos:ios-arm64:arm64:-miphoneos-version-min=$IOS_MIN" \
             "iphonesimulator:ios-arm64_x86_64-simulator:arm64:-mios-simulator-version-min=$SIM_ARM64_MIN" \
             "iphonesimulator:ios-arm64_x86_64-simulator:x86_64:-mios-simulator-version-min=$IOS_MIN"; do
  IFS=: read -r sdk slice_dir arch min_flag <<< "$entry"
  sdk_path="$(xcrun --sdk "$sdk" --show-sdk-path)"
  smoke_out="$TMP/smoke-$slice_dir-$arch"

  xcrun --sdk "$sdk" clang -isysroot "$sdk_path" -arch "$arch" "$min_flag" \
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
