#!/usr/bin/env python3
"""Stage the GitHub Release assets by their list (#273).

    stage_release_assets.py <downloaded-artifacts-dir> <out-dir>

The release job downloads every artifact of the run into one directory
(`actions/download-artifact` with `merge-multiple: true`). Uploading that
directory with a `**` glob shipped whatever happened to be there, under the
base name of each file: in v0.5.0 the three Android ABIs, all named
`libaimux_ffi.so`, overwrote each other into one asset of unknown ABI, the iOS
xcframework went up as two loose files (`aimux_ffi`, `Info.plist`), `aimux.h`
was missing (the Go binding's `go generate` downloads it), and the Node
binaries went up only because the Node build had finished before the
download.

This script copies exactly the assets in `ASSETS` from the download into
<out-dir>, under the names the release publishes, and fails if any of them is
missing. The Android libraries get their ABI in the name, and the
xcframework is zipped as one asset. Files of the download that are not on the
list (the Node `.node` binaries, published to npm) are listed and left out.
Every staged asset's byte size is printed, and written as a table to the job
summary when `GITHUB_STEP_SUMMARY` is set: the release-time measurement of
RFC-0043 §3.1.
"""

import os
import shutil
import sys
from pathlib import Path

GO_TARGETS = [
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-gnu",
]
CONSOLE_TOOLS = ["aimux-web", "aimux-cli", "aimux-replay"]
CONSOLE_SUFFIXES = ["-linux-x64", "-macos-arm64", "-windows-x64.exe"]
ANDROID_ABIS = ["arm64-v8a", "armeabi-v7a", "x86_64"]
XCFRAMEWORK = "aimux_ffi.xcframework"

# (published name, path in the download). A path that names a directory is
# zipped, with the directory as the archive's top-level entry.
ASSETS = (
    # go-build: the Go binding's static archives (bindings/go/cmd/ffidownload).
    [(f"libaimux_ffi-{t}.a", f"libaimux_ffi-{t}.a") for t in GO_TARGETS]
    # ffi-build: the C ABI shared libraries and their header.
    + [
        ("libaimux_ffi-linux-x64.so", "libaimux_ffi-linux-x64.so"),
        ("libaimux_ffi-macos-arm64.dylib", "libaimux_ffi-macos-arm64.dylib"),
        ("aimux_ffi-windows-x64.dll", "aimux_ffi-windows-x64.dll"),
        ("aimux.h", "aimux.h"),
    ]
    # console-build: the console and the CLI tools.
    + [(f"{tool}{s}", f"{tool}{s}") for tool in CONSOLE_TOOLS for s in CONSOLE_SUFFIXES]
    # flutter-ffi-mobile: one library per Android ABI, and the iOS xcframework.
    + [(f"libaimux_ffi-android-{abi}.so", f"android/{abi}/libaimux_ffi.so") for abi in ANDROID_ABIS]
    + [(f"{XCFRAMEWORK}.zip", XCFRAMEWORK)]
)


def stage(src: Path, out: Path):
    """Copy the listed assets from `src` to `out`. Returns (staged, missing,
    left_out): staged is [(name, bytes)], missing the download paths that are
    not there, left_out the downloaded files that are on no line of the list."""
    out.mkdir(parents=True, exist_ok=True)
    staged, missing, used = [], [], set()
    for name, rel in ASSETS:
        path = src / rel
        if path.is_dir():
            archive = shutil.make_archive(str(out / name[: -len(".zip")]), "zip", root_dir=src, base_dir=rel)
            used.update(p for p in path.rglob("*") if p.is_file())
            staged.append((name, Path(archive).stat().st_size))
        elif path.is_file():
            shutil.copyfile(path, out / name)
            used.add(path)
            staged.append((name, path.stat().st_size))
        else:
            missing.append(rel)
    left_out = sorted(str(p.relative_to(src)) for p in src.rglob("*") if p.is_file() and p not in used)
    return staged, missing, left_out


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    src, out = Path(argv[1]), Path(argv[2])
    staged, missing, left_out = stage(src, out)
    for name, size in staged:
        print(f"size {name} {size} bytes")
    for rel in left_out:
        print(f"not on the release list, left out: {rel}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("### Release assets\n\n| Asset | Bytes |\n|---|---:|\n")
            f.writelines(f"| `{name}` | {size:,} |\n" for name, size in staged)
            if missing:
                f.write("\nMissing: " + ", ".join(f"`{rel}`" for rel in missing) + "\n")
    if missing:
        for rel in missing:
            print(f"missing from the downloaded artifacts: {rel}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
