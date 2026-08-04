#!/usr/bin/env bash
# Copy the selected aimux_ffi.xcframework slice into CocoaPods'
# XCFrameworkIntermediates directory so the app link can find
# aimux_ffi.framework (FRAMEWORK_SEARCH_PATHS already points there via
# PODS_XCFRAMEWORKS_BUILD_DIR/aimux).
#
# Runs as the podspec's script_phase (pod target build, before compile) —
# CocoaPods' own xcframework script does not get mounted in Flutter
# projects (base configuration conflict), so we do it ourselves.
set -euo pipefail

SRC="${PODS_TARGET_SRCROOT}/aimux_ffi.xcframework"
DEST="${PODS_XCFRAMEWORKS_BUILD_DIR}/aimux"

if [[ "${SDK_NAME:-}" == *simulator* ]]; then
  SLICE="ios-arm64-simulator"
else
  SLICE="ios-arm64"
fi

test -d "$SRC/$SLICE/aimux_ffi.framework" || {
  echo "error: xcframework slice $SLICE not found in $SRC" >&2
  exit 1
}

mkdir -p "$DEST"
rm -rf "$DEST/aimux_ffi.framework"
cp -R "$SRC/$SLICE/aimux_ffi.framework" "$DEST/"
echo "embedded aimux_ffi.framework ($SLICE) -> $DEST"
