#!/usr/bin/env python3
"""Self-test of scripts/stage_release_assets.py on a synthetic download.

    python3 scripts/tests/test_stage_release_assets.py
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
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
    files = [rel for _, rel in stage.ASSETS if rel != stage.XCFRAMEWORK] + XCFRAMEWORK_FILES + NODE_FILES
    for rel in files:
        if rel in skip:
            continue
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rel, encoding="utf-8")


def run_cli(*args: str) -> tuple[int, str, str]:
    stdout, stderr = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        code = stage.main(["stage_release_assets.py", *args])
    return code, stdout.getvalue(), stderr.getvalue()


class StageReleaseAssetsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.src = self.root / "release-assets"
        self.out = self.root / "release-upload"
        os.environ.pop("GITHUB_STEP_SUMMARY", None)
        os.environ.pop("GITHUB_REF", None)
        os.environ.pop("GITHUB_SHA", None)

    def tearDown(self):
        self.tmp.cleanup()

    def test_stages_exactly_the_listed_assets_and_emits_manifest(self):
        download(self.src)
        code, stdout, stderr = run_cli("--source-sha", "a" * 40, str(self.src), str(self.out))
        self.assertEqual(code, 0, stderr)
        expected = sorted([name for name, _ in stage.ASSETS] + [stage.MANIFEST_FILENAME])
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), expected)
        self.assertEqual(len(stage.ASSETS), 21)
        for name, _ in stage.ASSETS:
            self.assertIn(f"size {name} ", stdout)
        m = json.loads((self.out / stage.MANIFEST_FILENAME).read_text(encoding="utf-8"))
        self.assertEqual(m["sourceSha"], "a" * 40)
        self.assertEqual(len(m["assets"]), 21)
        self.assertNotIn("rustcVersion", m)

    def test_each_android_abi_keeps_its_own_library(self):
        download(self.src)
        run_cli("--source-sha", "a" * 40, str(self.src), str(self.out))
        for abi in stage.ANDROID_ABIS:
            self.assertEqual(
                (self.out / f"libaimux_ffi-android-{abi}.so").read_text(encoding="utf-8"),
                f"android/{abi}/libaimux_ffi.so",
            )

    def test_xcframework_zip_is_deterministic_and_compressed(self):
        download(self.src)
        out2 = self.root / "release-upload-2"
        self.assertEqual(run_cli("--source-sha", "a" * 40, str(self.src), str(self.out))[0], 0)
        self.assertEqual(run_cli("--source-sha", "a" * 40, str(self.src), str(out2))[0], 0)
        z1 = (self.out / "aimux_ffi.xcframework.zip").read_bytes()
        z2 = (out2 / "aimux_ffi.xcframework.zip").read_bytes()
        self.assertEqual(hashlib.sha256(z1).hexdigest(), hashlib.sha256(z2).hexdigest())
        with zipfile.ZipFile(self.out / "aimux_ffi.xcframework.zip") as z:
            infos = [zi for zi in z.infolist() if not zi.is_dir()]
            self.assertEqual(sorted(zi.filename for zi in infos), sorted(XCFRAMEWORK_FILES))
            for zi in infos:
                self.assertEqual(zi.compress_type, zipfile.ZIP_DEFLATED)

    def test_files_off_the_list_are_left_out_and_missing_fails_without_manifest(self):
        download(self.src)
        code, stdout, _ = run_cli("--source-sha", "a" * 40, str(self.src), str(self.out))
        self.assertEqual(code, 0)
        for rel in NODE_FILES:
            self.assertFalse((self.out / rel).exists())
            self.assertIn(f"not on the release list, left out: {rel}", stdout)

        src_missing = self.root / "missing-src"
        download(src_missing, skip={"android/armeabi-v7a/libaimux_ffi.so", "aimux.h"})
        code, _, stderr = run_cli("--source-sha", "a" * 40, str(src_missing), str(self.out))
        self.assertEqual(code, 1)
        self.assertIn("missing from the downloaded artifacts: android/armeabi-v7a/libaimux_ffi.so", stderr)
        self.assertFalse((self.out / stage.MANIFEST_FILENAME).exists())

        # Directory in place of a file asset (e.g. Windows DLL) must fail closed.
        src_dir_dll = self.root / "dir-dll-src"
        download(src_dir_dll, skip={"aimux_ffi-windows-x64.dll"})
        (src_dir_dll / "aimux_ffi-windows-x64.dll").mkdir()
        (src_dir_dll / "aimux_ffi-windows-x64.dll" / "fake.txt").write_text("oops", encoding="utf-8")
        code, _, stderr = run_cli("--source-sha", "a" * 40, str(src_dir_dll), str(self.root / "dir-dll-out"))
        self.assertEqual(code, 1)
        self.assertIn("expected regular file", stderr)

    def test_job_summary_and_tag_validation(self):
        download(self.src)
        summary = self.root / "summary.md"
        os.environ["GITHUB_STEP_SUMMARY"] = str(summary)
        try:
            code, _, err = run_cli(
                "--source-sha", "a" * 40, "--rustc-version", "rustc 1.99.1 (58cf8f9c3 2026-10-13)", str(self.src), str(self.out)
            )
        finally:
            os.environ.pop("GITHUB_STEP_SUMMARY", None)
        self.assertEqual(code, 0, err)
        text = summary.read_text(encoding="utf-8")
        self.assertIn("| Asset | Bytes | SHA-256 |", text)
        self.assertIn("rustc `rustc 1.99.1 (58cf8f9c3 2026-10-13)`", text)

        code, _, err = run_cli("--source-sha", "a" * 40, "--tag", "v9.9.9", str(self.src), str(self.out))
        self.assertEqual(code, 1)
        self.assertIn("disagrees with tag", err)

    def test_rustc_version_dir_provenance_and_verify_mode(self):
        download(self.src)
        prov = self.root / "rustc-provenance"
        prov.mkdir()
        ver_a = "rustc 1.99.1 (58cf8f9c3 2026-10-13)"
        ver_b = "rustc 1.100.0 (112233445 2026-11-19)"
        for builder in stage.RELEASE_RUSTC_BUILDERS:
            eol = "\r\n" if "windows" in builder else "\n"
            (prov / f"{builder}.txt").write_bytes((ver_a + eol).encode("utf-8"))

        sha = "b" * 64
        ws_ver = stage.parse_workspace_version(Path(stage.__file__).resolve().parents[1] / "Cargo.toml")
        code, _, err = run_cli(
            "--source-sha",
            sha,
            "--tag",
            f"v{ws_ver}",
            "--rustc-version-dir",
            str(prov),
            "--expected-rustc-builders",
            "release",
            "--require-rustc-version",
            str(self.src),
            str(self.out),
        )
        self.assertEqual(code, 0, err)
        m = json.loads((self.out / stage.MANIFEST_FILENAME).read_text(encoding="utf-8"))
        self.assertEqual(m["rustcVersion"], ver_a)
        self.assertEqual(set(m["rustcVersions"]), set(stage.RELEASE_RUSTC_BUILDERS))

        # --verify passes on staged bundle
        code, _, err = run_cli(
            "--verify",
            "--source-sha",
            sha,
            "--tag",
            f"v{ws_ver}",
            "--expected-rustc-builders",
            "release",
            "--require-rustc-version",
            str(self.out),
        )
        self.assertEqual(code, 0, err)

        # Tampering rustcVersion or version double-v or extra file fails --verify
        manifest_path = self.out / stage.MANIFEST_FILENAME
        orig = manifest_path.read_text(encoding="utf-8")
        try:
            manifest_path.write_text(json.dumps(dict(m, rustcVersion=ver_b)), encoding="utf-8")
            self.assertEqual(run_cli("--verify", "--require-rustc-version", "--expected-rustc-builders", "release", str(self.out))[0], 1)

            manifest_path.write_text(json.dumps(dict(m, releaseVersion=f"vv{ws_ver}")), encoding="utf-8")
            self.assertEqual(run_cli("--verify", "--tag", f"v{ws_ver}", str(self.out))[0], 1)
        finally:
            manifest_path.write_text(orig, encoding="utf-8")

        # Heterogeneous builder versions emit sorted '; '-joined summary
        (prov / "flutter-ios.txt").write_text(ver_b + "\n", encoding="utf-8")
        out_mix = self.root / "staged-mixed"
        self.assertEqual(
            run_cli(
                "--source-sha",
                sha,
                "--rustc-version-dir",
                str(prov),
                "--expected-rustc-builders",
                "release",
                "--require-rustc-version",
                str(self.src),
                str(out_mix),
            )[0],
            0,
        )
        m_mix = json.loads((out_mix / stage.MANIFEST_FILENAME).read_text(encoding="utf-8"))
        self.assertEqual(m_mix["rustcVersion"], "; ".join(sorted([ver_a, ver_b])))

        # Missing builder fails closed
        (prov / "flutter-ios.txt").unlink()
        out_bad = self.root / "staged-bad"
        code, _, err = run_cli(
            "--source-sha",
            sha,
            "--rustc-version-dir",
            str(prov),
            "--expected-rustc-builders",
            "release",
            "--require-rustc-version",
            str(self.src),
            str(out_bad),
        )
        self.assertEqual(code, 1)
        self.assertIn("missing=['flutter-ios']", err)
        self.assertFalse((out_bad / stage.MANIFEST_FILENAME).exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
