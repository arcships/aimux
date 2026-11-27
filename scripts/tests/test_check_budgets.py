#!/usr/bin/env python3
"""Unit tests for scripts/quality/check_budgets.py and quality/size-budgets.json."""

from __future__ import annotations

import copy
import datetime as dt
import io
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "quality"))
import check_budgets as cb  # noqa: E402


def _make_zip(path: Path, entries: dict[str, bytes], *, symlink_name: str | None = None) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
        if symlink_name:
            zi = zipfile.ZipInfo(symlink_name)
            zi.external_attr = (0o120777 << 16)
            zf.writestr(zi, b"/etc/passwd")
    return path


def _make_tar_gz(path: Path, entries: dict[str, bytes], *, symlink_name: str | None = None) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path, "w:gz") as tf:
        for name, data in entries.items():
            ti = tarfile.TarInfo(name=name)
            ti.size = len(data)
            tf.addfile(ti, io.BytesIO(data))
        if symlink_name:
            si = tarfile.TarInfo(name=symlink_name)
            si.type = tarfile.SYMTYPE
            si.linkname = "/etc/passwd"
            tf.addfile(si)
    return path


class CheckBudgetsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.raw_cfg = cb.load_json_strict(cb.DEFAULT_BUDGETS)
        self.cfg = cb.validate_config(self.raw_cfg)

    def test_repo_config_valid_and_excludes_checksum_signature_globs(self) -> None:
        all_globs = [g for r in self.cfg["rules"] for g in r["globs"]]
        for bad_ext in (".md5", ".sha1", ".sha256", ".sha512", ".asc"):
            self.assertFalse(any(bad_ext in g for g in all_globs), f"unexpected {bad_ext} in {all_globs}")

        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = cb.main(["--validate-only"])
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(buf.getvalue())["status"], "PASS")

    def test_rule_precedence_for_android_jvm_and_npm(self) -> None:
        rules = self.cfg["rules"]
        self.assertEqual(cb._match_rule("libaimux_ffi-android-arm64-v8a.so", rules)["category"], "android_arm64")
        self.assertEqual(cb._match_rule("libaimux_ffi-linux-x64.so", rules)["category"], "cdylib")
        self.assertEqual(cb._match_rule("aimux-java-0.6.1-sources.jar", rules)["category"], "jvm_sources_jar")
        self.assertEqual(cb._match_rule("aimux-java-0.6.1-javadoc.jar", rules)["category"], "jvm_javadoc_jar")
        self.assertEqual(cb._match_rule("aimux-java-0.6.1.jar", rules)["category"], "jvm_main_jar")
        self.assertEqual(cb._match_rule("arcships-aimux-0.6.1.tgz", rules)["category"], "npm_main")
        self.assertEqual(cb._match_rule("arcships-aimux-linux-x64-gnu-0.6.1.tgz", rules)["category"], "npm_platform")
        staticlib_spec = self.cfg["rules_by_cat"]["staticlib"]["metrics"]["raw_bytes"]
        self.assertEqual(staticlib_spec, {"ceiling": 75_000_000, "target": 70_000_000})

    def test_ceiling_boundaries_target_warning_and_compare_delta(self) -> None:
        small_cfg = cb.validate_config(
            {
                "schema_version": 1,
                "rules": [
                    {
                        "category": "sample",
                        "globs": ["sample-*.bin"],
                        "raw_bytes": {"ceiling": 100, "target": 60},
                    }
                ],
                "aggregates": [],
                "exceptions": [],
            }
        )
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            f_ok = root / "sample-ok.bin"
            f_ok.write_bytes(b"a" * 50)
            rep_ok, rc_ok = cb.evaluate(small_cfg, [f_ok])
            self.assertEqual(rc_ok, 0)
            self.assertEqual(rep_ok["warning_count"], 0)

            f_warn = root / "sample-ok.bin"
            f_warn.write_bytes(b"a" * 100)  # exactly at ceiling C, above target 60
            rep_warn, rc_warn = cb.evaluate(small_cfg, [f_warn], compare_report=rep_ok)
            self.assertEqual(rc_warn, 0)
            self.assertEqual(rep_warn["warning_count"], 1)
            self.assertEqual(rep_warn["results"][0]["delta_vs_base_bytes"], 50)

            f_over = root / "sample-over.bin"
            f_over.write_bytes(b"a" * 101)  # C + 1 fails
            rep_over, rc_over = cb.evaluate(small_cfg, [f_over])
            self.assertEqual(rc_over, 1)
            self.assertEqual(rep_over["status"], "FAIL")

    def test_android_total_aggregate_and_ios_xcframework_slices(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            ad = root / "android"
            for abi, sz in (("arm64-v8a", 12_950_000), ("armeabi-v7a", 8_950_000), ("x86_64", 16_950_000)):
                p = ad / abi / "libaimux_ffi.so"
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(b"\x00" * sz)
            # Each ABI is under its cap (13M/9M/17M), but sum is 38.85M > 38.6M!
            rep, rc = cb.evaluate(self.cfg, [ad])
            self.assertEqual(rc, 1)
            agg_rows = [r for r in rep["results"] if r["category"] == "android_total"]
            self.assertEqual(len(agg_rows), 1)
            self.assertEqual(agg_rows[0]["status"], "FAIL")

            xc = root / "aimux_ffi.xcframework"
            (xc / "Info.plist").parent.mkdir(parents=True, exist_ok=True)
            (xc / "Info.plist").write_text("<plist/>", encoding="utf-8")
            for sl in ("ios-arm64", "ios-arm64_x86_64-simulator"):
                fw = xc / sl / "aimux_ffi.framework"
                fw.mkdir(parents=True, exist_ok=True)
                (fw / "aimux_ffi").write_bytes(b"ok")
            rep_xc, rc_xc = cb.evaluate(self.cfg, [xc])
            self.assertEqual(rc_xc, 0)
            self.assertEqual(sum(1 for r in rep_xc["results"] if r["category"] == "ios_slice"), 2)

    def test_archive_safety_and_wheel_native_checks(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            # Zip traversal
            bad_zip = _make_zip(root / "aimux-java-0.6.1.jar", {"../evil.txt": b"x"})
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [bad_zip])
            # Zip symlink
            sym_zip = _make_zip(root / "aimux-kotlin-0.6.1.jar", {"ok.class": b"x"}, symlink_name="link")
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [sym_zip])
            # Tar traversal & symlink
            bad_tar = _make_tar_gz(root / "aimux-core-0.6.1.crate", {"../../evil": b"x"})
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [bad_tar])
            sym_tar = _make_tar_gz(root / "aimux-stream-0.6.1.crate", {"a/Cargo.toml": b"x"}, symlink_name="a/link")
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [sym_tar])
            # Wheel without native extension fails; wheel with .so passes
            whl_no_native = _make_zip(root / "arcships_aimux-0.6.1-cp39-abi3-manylinux_2_17_x86_64.whl", {"aimux/__init__.py": b""})
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [whl_no_native])
            whl_ok = _make_zip(
                root / "arcships_aimux-0.6.1-cp39-abi3-win_amd64.whl",
                {"aimux/__init__.py": b"", "aimux/_aimux.pyd": b"MZ" + b"\x00" * 100},
            )
            rep_whl, rc_whl = cb.evaluate(self.cfg, [whl_ok])
            self.assertEqual(rc_whl, 0)
            self.assertEqual({r["metric"] for r in rep_whl["results"]}, {"raw_bytes", "packed_bytes", "unpacked_bytes"})

            # Corrupt ZIP CRC payload flipped -> rejected
            corrupt_zip = root / "aimux-java-0.6.1-corrupt.jar"
            with zipfile.ZipFile(corrupt_zip, "w", compression=zipfile.ZIP_STORED) as zf:
                zf.writestr("ok.txt", b"HELLO_WORLD")
            raw_z = bytearray(corrupt_zip.read_bytes())
            idx = raw_z.index(b"HELLO_WORLD")
            raw_z[idx] ^= 0xFF
            corrupt_zip.write_bytes(bytes(raw_z))
            with self.assertRaises(cb.BudgetError):
                cb.inspect_zip(corrupt_zip)

            # ZIP directory/file collision (direct and nested x vs x/y/) -> rejected
            coll_zip = root / "aimux-java-0.6.1-coll.jar"
            with zipfile.ZipFile(coll_zip, "w") as zf:
                zf.writestr("foo", b"file")
                zf.writestr("foo/", b"")
            with self.assertRaises(cb.BudgetError):
                cb.inspect_zip(coll_zip)
            nested_coll_zip = root / "aimux-java-0.6.1-nested-coll.jar"
            with zipfile.ZipFile(nested_coll_zip, "w") as zf:
                zf.writestr("x", b"file")
                zf.writestr("x/y/", b"")
            with self.assertRaises(cb.BudgetError):
                cb.inspect_zip(nested_coll_zip)

            # TAR nested file/dir collision (x vs x/y/) -> rejected
            nested_coll_tar = root / "aimux-core-0.6.1-nested-coll.crate"
            with tarfile.open(nested_coll_tar, "w:gz") as tf:
                finfo = tarfile.TarInfo("x"); finfo.size = 4
                tf.addfile(finfo, io.BytesIO(b"1234"))
                dinfo = tarfile.TarInfo("x/y"); dinfo.type = tarfile.DIRTYPE
                tf.addfile(dinfo)
            with self.assertRaises(cb.BudgetError):
                cb.inspect_tar(nested_coll_tar)

            # Corrupt second tar header -> rejected
            import gzip
            raw_t = io.BytesIO()
            with tarfile.open(fileobj=raw_t, mode="w") as tf:
                for n in ("a.txt", "b.txt"):
                    ti = tarfile.TarInfo(n); ti.size = 4
                    tf.addfile(ti, io.BytesIO(b"1234"))
            tb = bytearray(raw_t.getvalue())
            tb[1024 + 10] ^= 0xFF
            corrupt_tar = root / "aimux-core-0.6.1-corrupt.crate"
            corrupt_tar.write_bytes(gzip.compress(bytes(tb)))
            with self.assertRaises(cb.BudgetError):
                cb.inspect_tar(corrupt_tar)

            # Truncated tar (missing end-of-archive zero blocks) -> rejected
            trunc_tar = root / "aimux-core-0.6.1-trunc.crate"
            trunc_tar.write_bytes(gzip.compress(raw_t.getvalue()[:2048]))
            with self.assertRaises(cb.BudgetError):
                cb.inspect_tar(trunc_tar)

            # xcframework zip missing Info.plist or empty slices -> rejected
            bad_xc_zip = _make_zip(
                root / "aimux_ffi.xcframework.zip",
                {
                    "aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi": b"",
                    "aimux_ffi.xcframework/ios-arm64_x86_64-simulator/aimux_ffi.framework/aimux_ffi": b"",
                },
            )
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [bad_xc_zip])

            # xcframework zip with Unix S_IFDIR entries pretending to be Info.plist & slices -> rejected
            dir_spoof_zip = root / "aimux_ffi.xcframework.zip"
            with zipfile.ZipFile(dir_spoof_zip, "w") as zf:
                for name in (
                    "aimux_ffi.xcframework/Info.plist",
                    "aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi",
                    "aimux_ffi.xcframework/ios-arm64_x86_64-simulator/aimux_ffi.framework/aimux_ffi",
                ):
                    zi = zipfile.ZipInfo(name)
                    zi.external_attr = (0o040755 << 16)
                    zf.writestr(zi, b"fake")
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [dir_spoof_zip])

            # ZIP special mode (FIFO) disguised with trailing '/' or DOS dir bit 0x10 -> rejected
            for idx, (fname, ext_attr) in enumerate([
                ("fifo/", (0o010644 << 16)),
                ("fifo_dos", (0o010644 << 16) | 0x10),
                ("reg_as_dir/", (0o100644 << 16)),
            ]):
                sp_zip = root / f"aimux-java-0.6.1-special-{idx}.jar"
                with zipfile.ZipFile(sp_zip, "w") as zf:
                    zf.writestr("ok.class", b"ok")
                    zi = zipfile.ZipInfo(fname)
                    zi.external_attr = ext_attr
                    zf.writestr(zi, b"")
                with self.assertRaises(cb.BudgetError):
                    cb.inspect_zip(sp_zip)

    def test_flutter_pub_archive_exact_ceilings(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            pkg_tgz = _make_tar_gz(
                root / "aimux-0.6.1.tar.gz",
                {"pubspec.yaml": b"name: aimux\nversion: 0.6.1\n", "lib/aimux.dart": b"void main() {}\n"},
            )
            rep, rc = cb.evaluate(self.cfg, [pkg_tgz])
            self.assertEqual(rc, 0)
            self.assertEqual({r["category"] for r in rep["results"]}, {"flutter_pub"})
            self.assertEqual({r["metric"] for r in rep["results"]}, {"packed_bytes", "unpacked_bytes"})

    def test_exceptions_expiry_and_commit_bound_version_scope(self) -> None:
        now = dt.datetime(2026, 11, 20, 0, 0, tzinfo=dt.timezone.utc)
        head_sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, text=True).strip()
        tree_sha = subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=REPO_ROOT, text=True).strip()
        resolved_sha, head_ver = cb._resolve_commit_info(head_sha)
        self.assertEqual(resolved_sha, head_sha)
        self.assertIsNotNone(head_ver)

        base = {
            "schema_version": 1,
            "rules": [{"category": "sample", "globs": ["sample.bin"], "raw_bytes": {"ceiling": 100}}],
            "aggregates": [],
            "exceptions": [
                {
                    "category": "sample",
                    "metric": "raw_bytes",
                    "temp_cap_bytes": 200,
                    "reason": "short-term override",
                    "issue": "#519",
                    "approver": "eric",
                    "expires_at": "2026-11-30T00:00:00Z",
                    "commit_or_version": f"v{head_ver}",
                }
            ],
        }
        cfg = cb.validate_config(base, now=now)
        with tempfile.TemporaryDirectory() as td:
            f = Path(td) / "sample.bin"
            f.write_bytes(b"x" * 150)
            # Real HEAD commit whose Cargo.toml matches v{head_ver} passes with exception.
            rep_ok, rc_ok = cb.evaluate(cfg, [f], source_sha=head_sha)
            self.assertEqual(rc_ok, 0)
            self.assertEqual(rep_ok["status"], "PASS_WITH_EXCEPTION")
            # Tree SHA or unknown SHA must NOT receive version-scoped exception.
            _, rc_tree = cb.evaluate(cfg, [f], source_sha=tree_sha)
            self.assertEqual(rc_tree, 1)
            _, rc_unknown = cb.evaluate(cfg, [f], source_sha="deadbeef" * 5)
            self.assertEqual(rc_unknown, 1)

            # Commit-prefix exception must NOT match a non-commit --source-sha string.
            sha_exc = copy.deepcopy(base)
            sha_exc["exceptions"][0]["commit_or_version"] = "deadbee"
            sha_cfg = cb.validate_config(sha_exc, now=now)
            _, rc_fake = cb.evaluate(sha_cfg, [f], source_sha="deadbee-not-a-commit")
            self.assertEqual(rc_fake, 1)

        # DST-crossing timezone-aware `now` must still enforce exact 14*86400 UTC seconds.
        from zoneinfo import ZoneInfo
        ny_now = dt.datetime(2026, 10, 25, 12, 0, tzinfo=ZoneInfo("America/New_York"))  # UTC 16:00, crosses Nov 1 fall-back DST
        dst_over = copy.deepcopy(base)
        dst_over["exceptions"][0]["expires_at"] = "2026-11-08T16:30:00Z"  # 14 days + 30 min elapsed UTC
        with self.assertRaises(cb.BudgetError):
            cb.validate_config(dst_over, now=ny_now)

        # Expired exception fails config validation.
        expired = copy.deepcopy(base)
        expired["exceptions"][0]["expires_at"] = "2026-11-19T00:00:00Z"
        with self.assertRaises(cb.BudgetError):
            cb.validate_config(expired, now=now)

        # Missing commit_or_version or lifetime > 14 days fails config validation.
        unscoped = copy.deepcopy(base)
        unscoped["exceptions"][0].pop("commit_or_version")
        with self.assertRaises(cb.BudgetError):
            cb.validate_config(unscoped, now=now)
        too_long = copy.deepcopy(base)
        too_long["exceptions"][0]["expires_at"] = "2099-01-01T00:00:00Z"
        with self.assertRaises(cb.BudgetError):
            cb.validate_config(too_long, now=now)

    def test_zero_matched_and_unmatched_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            stray = root / "unknown.xyz"
            stray.write_bytes(b"123")
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [stray])
            with self.assertRaises(cb.BudgetError):
                cb.evaluate(self.cfg, [stray], allow_unmatched=True)
            err = io.StringIO()
            with redirect_stderr(err):
                rc = cb.main([str(root / "empty")])
            self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
