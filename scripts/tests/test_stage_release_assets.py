#!/usr/bin/env python3
"""Self-test of scripts/stage_release_assets.py on a synthetic download.

    python3 scripts/tests/test_stage_release_assets.py

The download is laid out the way `actions/download-artifact` with
`merge-multiple: true` leaves the release run's artifacts: the Go archives,
shared libraries, header and console binaries at the top, the Android
libraries under `android/<abi>/`, the xcframework as a directory, and the
Node binaries of a Node build that finished first.
"""

import contextlib
import io
import os
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import stage_release_assets as stage  # noqa: E402

XCFRAMEWORK_FILES = [
    "aimux_ffi.xcframework/Info.plist",
    "aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi",
    "aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/Info.plist",
    "aimux_ffi.xcframework/ios-arm64-simulator/aimux_ffi.framework/aimux_ffi",
    "aimux_ffi.xcframework/ios-arm64-simulator/aimux_ffi.framework/Info.plist",
]
NODE_FILES = ["aimux.linux-x64-gnu.node", "aimux.darwin-arm64.node"]


def download(root: Path, skip=()):
    """Write the synthetic download under `root`; each file holds its own
    path, so a test can tell which source an asset came from."""
    files = [rel for _, rel in stage.ASSETS if rel != stage.XCFRAMEWORK] + XCFRAMEWORK_FILES + NODE_FILES
    for rel in files:
        if rel in skip:
            continue
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rel)


def run(src: Path, out: Path):
    stdout, stderr = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        code = stage.main(["stage_release_assets.py", str(src), str(out)])
    return code, stdout.getvalue(), stderr.getvalue()


class StageReleaseAssetsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.src = Path(self.tmp.name, "release-assets")
        self.out = Path(self.tmp.name, "release-upload")
        os.environ.pop("GITHUB_STEP_SUMMARY", None)

    def tearDown(self):
        self.tmp.cleanup()

    def test_stages_exactly_the_listed_assets(self):
        download(self.src)
        code, stdout, _ = run(self.src, self.out)
        self.assertEqual(code, 0)
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), sorted(name for name, _ in stage.ASSETS))
        self.assertEqual(len(stage.ASSETS), 21)
        for name, _ in stage.ASSETS:
            self.assertIn(f"size {name} ", stdout)

    def test_each_android_abi_keeps_its_own_library(self):
        download(self.src)
        run(self.src, self.out)
        for abi in stage.ANDROID_ABIS:
            self.assertEqual(
                (self.out / f"libaimux_ffi-android-{abi}.so").read_text(),
                f"android/{abi}/libaimux_ffi.so",
            )

    def test_the_xcframework_ships_as_one_zip_with_both_slices(self):
        download(self.src)
        run(self.src, self.out)
        with zipfile.ZipFile(self.out / "aimux_ffi.xcframework.zip") as z:
            names = [n for n in z.namelist() if not n.endswith("/")]
            self.assertEqual(sorted(names), sorted(XCFRAMEWORK_FILES))
            self.assertEqual(z.read(XCFRAMEWORK_FILES[1]).decode(), XCFRAMEWORK_FILES[1])

    def test_the_header_the_go_downloader_fetches_is_staged(self):
        download(self.src)
        run(self.src, self.out)
        self.assertEqual((self.out / "aimux.h").read_text(), "aimux.h")

    def test_files_off_the_list_are_left_out_and_named(self):
        download(self.src)
        code, stdout, _ = run(self.src, self.out)
        self.assertEqual(code, 0)
        for rel in NODE_FILES:
            self.assertFalse((self.out / rel).exists())
            self.assertIn(f"not on the release list, left out: {rel}", stdout)

    def test_a_missing_asset_fails_and_names_it(self):
        download(self.src, skip={"android/armeabi-v7a/libaimux_ffi.so", "aimux.h"})
        code, _, stderr = run(self.src, self.out)
        self.assertEqual(code, 1)
        self.assertIn("missing from the downloaded artifacts: android/armeabi-v7a/libaimux_ffi.so", stderr)
        self.assertIn("missing from the downloaded artifacts: aimux.h", stderr)

    def test_the_job_summary_gets_the_size_table(self):
        download(self.src)
        summary = Path(self.tmp.name, "summary.md")
        os.environ["GITHUB_STEP_SUMMARY"] = str(summary)
        try:
            run(self.src, self.out)
        finally:
            os.environ.pop("GITHUB_STEP_SUMMARY")
        text = summary.read_text()
        self.assertIn("| Asset | Bytes |", text)
        self.assertIn("| `libaimux_ffi-android-x86_64.so` |", text)


if __name__ == "__main__":
    unittest.main()
