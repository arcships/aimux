#!/usr/bin/env bash
# Build the aimux-ffi iOS static framework xcframework.
#
# Framework-type slices (not bare .a) so Xcode/SPM and CocoaPods link the
# Rust static library reliably. Slice names follow xcodebuild's convention
# (ios-arm64 / ios-arm64-simulator) — ios/aimux.podspec's force_load paths
# depend on them.
#
# Usage: scripts/build-ios-xcframework.sh <output-dir>
set -euo pipefail

OUT="${1:?usage: build-ios-xcframework.sh <output-dir>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build -p aimux-ffi --release --target aarch64-apple-ios
cargo build -p aimux-ffi --release --target aarch64-apple-ios-sim

rm -rf "$OUT"
for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
  case "$triple" in
    aarch64-apple-ios) slice=ios-arm64 ;;
    aarch64-apple-ios-sim) slice=ios-arm64-simulator ;;
  esac
  mkdir -p "$OUT/$slice/aimux_ffi.framework"
  cp "target/$triple/release/libaimux_ffi.a" "$OUT/$slice/aimux_ffi.framework/aimux_ffi"
  cat > "$OUT/$slice/aimux_ffi.framework/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>ai.arcships.aimux.aimux_ffi</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>aimux_ffi</string>
  <key>CFBundleExecutable</key><string>aimux_ffi</string>
  <key>CFBundlePackageType</key><string>FMWK</string>
  <key>CFBundleVersion</key><string>1</string>
</dict></plist>
EOF
done

cat > "$OUT/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundlePackageType</key><string>XFWK</string>
  <key>XCFrameworkFormatVersion</key><string>1.0</string>
  <key>AvailableLibraries</key><array>
    <dict>
      <key>LibraryIdentifier</key><string>ios-arm64</string>
      <key>LibraryPath</key><string>aimux_ffi.framework</string>
      <key>SupportedArchitectures</key><array><string>arm64</string></array>
      <key>SupportedPlatform</key><string>ios</string>
    </dict>
    <dict>
      <key>LibraryIdentifier</key><string>ios-arm64-simulator</string>
      <key>LibraryPath</key><string>aimux_ffi.framework</string>
      <key>SupportedArchitectures</key><array><string>arm64</string></array>
      <key>SupportedPlatform</key><string>ios</string>
      <key>SupportedPlatformVariant</key><string>simulator</string>
    </dict>
  </array>
</dict></plist>
EOF

test -f "$OUT/ios-arm64/aimux_ffi.framework/aimux_ffi"
test -f "$OUT/ios-arm64-simulator/aimux_ffi.framework/aimux_ffi"
echo "xcframework written to $OUT"
