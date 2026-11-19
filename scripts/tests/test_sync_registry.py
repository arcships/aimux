#!/usr/bin/env python3
"""Offline acceptance tests for scripts/sync_registry.py (RFC-0033 §8)."""

from __future__ import annotations

import contextlib
import gzip
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import registry_sanitize as rs  # noqa: E402
import sync_registry as sync  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def _write_json(path: Path, obj: object) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return path


def _run_sync(argv: list[str]):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = sync.main(argv)
    return code, out.getvalue(), err.getvalue()


class SyncRegistryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.packages_rs = self.dir / "default_providers.rs"
        self.packages_rs.write_text('const PACKAGES: [&str; 1] = ["openai"];\n', encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def _fixture(self, rows, up_providers, aliases=None, reconcile=None):
        reg_p = _write_json(self.dir / "registry.json", rows)
        ali_p = _write_json(self.dir / "aliases.json", aliases or {})
        rec_p = _write_json(
            self.dir / "reconcile.json",
            reconcile or {"schema_version": 1, "overrides": [], "entries": {}},
        )
        snap_p = _write_json(
            self.dir / "snap.json",
            {"schema_version": 1, "source_url": "https://models.dev/api.json", "providers": up_providers},
        )
        return reg_p, ali_p, rec_p, snap_p

    def test_repo_offline_check_passes_and_is_deterministic(self):
        out1 = self.dir / "r1.json"
        out2 = self.dir / "r2.json"
        code1, _, err1 = _run_sync(
            [
                "--check",
                "--source",
                "models-dev",
                "--snapshot",
                str(ROOT / "registry" / "models_dev_snapshot.json"),
                "--output",
                str(out1),
            ]
        )
        self.assertEqual(code1, 0, err1)
        code2, _, err2 = _run_sync(
            [
                "--check",
                "--source",
                "models-dev",
                "--snapshot",
                str(ROOT / "registry" / "models_dev_snapshot.json"),
                "--output",
                str(out2),
            ]
        )
        self.assertEqual(code2, 0, err2)
        self.assertEqual(out1.read_bytes(), out2.read_bytes())

    def test_endpoint_normalization_preserves_v1_and_trailing_slash(self):
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "case_port", "display": "C", "base_url": "https://API.Example.COM:443/v1", "env_var": "K_KEY"},
                {"name": "slash_diff", "display": "S", "base_url": "https://api.example.com/v1/", "env_var": "K_KEY"},
                {"name": "v1_diff", "display": "V", "base_url": "https://api.example.com", "env_var": "K_KEY"},
                {
                    "name": "tpl",
                    "display": "T",
                    "base_url": "https://{region}.example.com/v1",
                    "env_var": "K_KEY",
                    "params": [{"name": "region", "env": ["TPL_REGION"]}],
                },
            ],
            up_providers={
                "case_port": {"id": "case_port", "api": "HTTPS://api.example.com/v1", "env": ["K_KEY"]},
                "slash_diff": {"id": "slash_diff", "api": "https://api.example.com/v1", "env": ["K_KEY"]},
                "v1_diff": {"id": "v1_diff", "api": "https://api.example.com/v1", "env": ["K_KEY"]},
                "tpl": {"id": "tpl", "api": "${TPL_HOST}/v1", "env": ["TPL_REGION", "K_KEY"]},
            },
        )
        out = self.dir / "report.json"
        code, _, err = _run_sync(
            [
                "--report",
                "--source",
                "models-dev",
                "--snapshot",
                str(snap_p),
                "--registry",
                str(reg_p),
                "--aliases",
                str(ali_p),
                "--reconcile",
                str(rec_p),
                "--packages-rs",
                str(self.packages_rs),
                "--output",
                str(out),
            ]
        )
        self.assertEqual(code, 0, err)
        report = json.loads(out.read_text())
        diff_ids = [d["id"] for d in report["base_url_diffs"]]
        self.assertEqual(diff_ids, ["slash_diff", "v1_diff"])
        self.assertEqual([d["id"] for d in report["incomparable"]], ["tpl"])
        self.assertEqual(report["counts"]["key_env_diffs"], 0)

    def test_keyless_and_optional_env_plus_no_env_reads(self):
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "local_only", "display": "L", "base_url": "http://127.0.0.1:8080/v1", "auth": "none"},
                {
                    "name": "opt_key",
                    "display": "O",
                    "base_url": "http://127.0.0.1:11434/v1",
                    "auth": "none",
                    "env_var": "OPT_KEY",
                    "base_url_env": "OPT_BASE_URL",
                },
            ],
            up_providers={
                "local_only": {"id": "local_only", "api": "http://127.0.0.1:8080/v1", "env": []},
                "opt_key": {"id": "opt_key", "api": "http://127.0.0.1:11434/v1", "env": ["OPT_KEY"]},
            },
        )
        out = self.dir / "report.json"
        decoy = "super_decoy_secret_value_xyz"
        with mock.patch.dict(os.environ, {"OPT_KEY": decoy, "OPT_BASE_URL": "http://decoy.invalid/secret"}):
            code, stdout, err = _run_sync(
                [
                    "--check",
                    "--source",
                    "models-dev",
                    "--snapshot",
                    str(snap_p),
                    "--registry",
                    str(reg_p),
                    "--aliases",
                    str(ali_p),
                    "--reconcile",
                    str(rec_p),
                    "--packages-rs",
                    str(self.packages_rs),
                    "--output",
                    str(out),
                ]
            )
        self.assertEqual(code, 0, err)
        raw_out = out.read_text() + stdout + err
        self.assertNotIn(decoy, raw_out)
        self.assertNotIn("decoy.invalid", raw_out)

    def test_global_url_redaction_and_different_secret_queries(self):
        sec_a = "sk_secret_alpha_999"
        sec_b = "sk_secret_beta_888"
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "alpha", "display": "A", "base_url": f"https://u:{sec_a}@example.invalid/v1?x={sec_a}#frag1", "env_var": "A_KEY"},
                {"name": "mal", "display": "M", "base_url": f"https://api.example.com/u:s/{sec_a}/v1", "env_var": "M_KEY"},
            ],
            up_providers={
                "alpha": {"id": "alpha", "api": f"https://u:{sec_b}@example.invalid/v1?y={sec_b}#frag2", "env": ["A_KEY"]},
                "mal": {"id": "mal", "api": f"https://other:{sec_b}%2F@example.invalid/v1", "env": ["M_KEY"]},
            },
        )
        out = self.dir / "report.json"
        md_out = self.dir / "report.md"
        code, stdout, err = _run_sync(
            [
                "--report",
                "--source",
                "models-dev",
                "--snapshot",
                str(snap_p),
                "--registry",
                str(reg_p),
                "--aliases",
                str(ali_p),
                "--reconcile",
                str(rec_p),
                "--packages-rs",
                str(self.packages_rs),
                "--output",
                str(out),
                "--markdown-output",
                str(md_out),
            ]
        )
        self.assertEqual(code, 0, err)
        all_text = out.read_text() + md_out.read_text() + stdout + err
        self.assertNotIn(sec_a, all_text)
        self.assertNotIn(sec_b, all_text)
        report = json.loads(out.read_text())
        self.assertEqual(report["input_hash_kind"], "sanitized")
        alpha_diff = [d for d in report["base_url_diffs"] if d["id"] == "alpha"][0]
        # Identical redacted string yet memory comparison still caught different query/creds.
        self.assertEqual(alpha_diff["local"], "https://[REDACTED]@example.invalid/v1?[REDACTED]#[REDACTED]")
        self.assertEqual(alpha_diff["upstream_value"], alpha_diff["local"])
        self.assertEqual(alpha_diff["comparison"], "different")
        self.assertEqual(rs.sanitize_url(f"bad host/{sec_a}")["sanitized"], rs.REDACTED_URL)

    def test_reconcile_override_staleness_and_apply_protection(self):
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "protected_vendor", "display": "P", "base_url": "https://local.example/v1", "env_var": "LOCAL_KEY"},
                {"name": "auto_update", "display": "U", "base_url": "https://old.example/v1", "env_var": "OLD_KEY", "profile": {"max_tokens_key": "max_tokens"}},
            ],
            up_providers={
                "protected_vendor": {"id": "protected_vendor", "api": "https://up.example/v1", "env": ["UP_KEY"]},
                "auto_update": {"id": "auto_update", "api": "https://new.example/v1", "env": ["NEW_KEY"]},
                "new_upstream": {"id": "new_upstream", "api": "https://brandnew.example/v1", "env": ["BRAND_KEY"]},
            },
            reconcile={
                "schema_version": 1,
                "overrides": [
                    {
                        "id": "protected_vendor",
                        "upstream": "models-dev",
                        "field": "base_url",
                        "local_value": "https://local.example/v1",
                        "upstream_value": "https://up.example/v1",
                        "reason": "Keep local URL",
                    }
                ],
                "entries": {
                    "protected_vendor": {"note": "Manual note kept with visible env diff"},
                },
            },
        )
        out = self.dir / "apply.json"
        code, _, err = _run_sync(
            [
                "--apply",
                "--source",
                "models-dev",
                "--snapshot",
                str(snap_p),
                "--registry",
                str(reg_p),
                "--aliases",
                str(ali_p),
                "--reconcile",
                str(rec_p),
                "--packages-rs",
                str(self.packages_rs),
                "--output",
                str(out),
            ]
        )
        self.assertEqual(code, 0, err)
        updated = json.loads(reg_p.read_text())
        self.assertEqual([r["name"] for r in updated], ["protected_vendor", "auto_update"])
        self.assertEqual(updated[0]["base_url"], "https://local.example/v1")
        self.assertEqual(updated[1]["base_url"], "https://new.example/v1")
        self.assertEqual(updated[1]["env_var"], "NEW_KEY")
        self.assertEqual(updated[1]["profile"], {"max_tokens_key": "max_tokens"})

        # Mutate upstream snapshot on protected override -> becomes stale and --check fails!
        snap_data = json.loads(snap_p.read_text())
        snap_data["providers"]["protected_vendor"]["api"] = "https://changed.example/v1"
        _write_json(snap_p, snap_data)
        check_code, _, _ = _run_sync(
            [
                "--check",
                "--source",
                "models-dev",
                "--snapshot",
                str(snap_p),
                "--registry",
                str(reg_p),
                "--aliases",
                str(ali_p),
                "--reconcile",
                str(rec_p),
                "--packages-rs",
                str(self.packages_rs),
                "--output",
                str(out),
            ]
        )
        self.assertEqual(check_code, 1)
        check_report = json.loads(out.read_text())
        self.assertEqual(check_report["counts"]["stale_overrides"], 1)

    def test_schema_and_cli_error_codes(self):
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[{"name": "v", "display": "V", "base_url": "https://a.invalid/v1", "env_var": "V_KEY"}],
            up_providers={"v": {"id": "v", "api": 123, "env": ["V_KEY"]}},
        )
        code, _, _ = _run_sync(
            [
                "--report",
                "--source",
                "models-dev",
                "--snapshot",
                str(snap_p),
                "--registry",
                str(reg_p),
                "--aliases",
                str(ali_p),
                "--reconcile",
                str(rec_p),
                "--packages-rs",
                str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)
        arg_code, _, _ = _run_sync(["--check", "--source", "models-dev", "--fetch"])
        self.assertEqual(arg_code, 2)

    def test_duplicate_ids_and_alias_conflicts_fail_closed(self):
        # Duplicate local registry name
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "dup_p", "display": "D1", "base_url": "https://a.example/v1", "env_var": "A_KEY"},
                {"name": "dup_p", "display": "D2", "base_url": "https://b.example/v1", "env_var": "B_KEY"},
            ],
            up_providers={},
        )
        code, _, err = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)
        self.assertIn("duplicate registry name", err)

        # Duplicate target in aliases (alias conflict)
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[{"name": "v", "display": "V", "base_url": "https://a.example/v1", "env_var": "V_KEY"}],
            up_providers={},
            aliases={"alias-1": "v", "alias-2": "v"},
        )
        code, _, err = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)
        self.assertIn("duplicate alias target", err)

        # Alias source colliding with built-in provider id
        _write_json(ali_p, {"v": "openai"})
        code, _, err = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)
        self.assertIn("collides with built-in provider id", err)

        # Mismatched/duplicate id in models.dev snapshot
        _write_json(ali_p, {})
        _write_json(
            snap_p,
            {
                "schema_version": 1,
                "providers": {
                    "k1": {"id": "dup_up", "api": "https://a.example/v1", "env": ["V_KEY"]},
                },
            },
        )
        code, _, _ = _run_sync(
            [
                "--report", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)

    def test_manual_note_does_not_hide_unacknowledged_drift_and_stale_override_blocks_apply(self):
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "noted", "display": "N", "base_url": "https://local.example/v1", "env_var": "N_KEY"},
                {"name": "kilo_like", "display": "K", "base_url": "https://local-k.example/v1", "env_var": "K_KEY"},
            ],
            up_providers={
                "noted": {"id": "noted", "api": "https://drifted.example/v1", "env": ["N_KEY"]},
                "kilo_like": {"id": "kilo_like", "api": "https://drifted-k.example/v1", "env": ["K_KEY"]},
            },
            reconcile={
                "schema_version": 1,
                "overrides": [
                    {
                        "id": "kilo_like",
                        "upstream": "models-dev",
                        "field": "base_url",
                        "local_value": "https://local-k.example/v1",
                        "upstream_value": "https://old-up-k.example/v1",
                        "reason": "Pin kilo_like URL",
                    }
                ],
                "entries": {"noted": {"note": "Noted local vendor"}},
            },
        )
        out = self.dir / "rep.json"
        md_out = self.dir / "rep.md"
        code, _, _ = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
                "--output", str(out), "--markdown-output", str(md_out),
            ]
        )
        self.assertEqual(code, 1)
        rep = json.loads(out.read_text())
        self.assertEqual([d["id"] for d in rep["base_url_diffs"]], ["kilo_like", "noted"])
        self.assertIn("noted", md_out.read_text())
        self.assertTrue(rep["repo_commit"])

        # --apply must not overwrite kilo_like when its override is stale
        apply_out = self.dir / "apply.json"
        apply_code, _, err = _run_sync(
            [
                "--apply", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs), "--output", str(apply_out),
            ]
        )
        self.assertEqual(apply_code, 0, err)
        updated = {r["name"]: r for r in json.loads(reg_p.read_text())}
        self.assertEqual(updated["kilo_like"]["base_url"], "https://local-k.example/v1")

    def test_sensitive_url_equality_snapshot_redaction_env_set_and_keyless_apply(self):
        secret = "sk_live_secret_987"
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "same_sec", "display": "S", "base_url": f"https://u:{secret}@example.invalid/v1?k=1", "env_var": "S_KEY"},
                {"name": "keyless", "display": "K", "base_url": "http://127.0.0.1:8080/v1", "auth": "none"},
                {"name": "subset_env", "display": "E", "base_url": "https://api.example.com/v1", "env_var": "V_KEY"},
            ],
            up_providers={
                "same_sec": {"id": "same_sec", "api": f"https://u:{secret}@example.invalid/v1?k=1", "env": ["S_KEY"]},
                "keyless": {"id": "keyless", "api": "http://127.0.0.1:8080/v1", "env": ["KEYLESS_OPT_KEY"]},
                "subset_env": {"id": "subset_env", "api": "https://api.example.com/v1", "env": ["V_KEY", "V_ADMIN_KEY"]},
            },
        )
        out = self.dir / "rep.json"
        code, _, _ = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs), "--output", str(out),
            ]
        )
        self.assertEqual(code, 1)
        rep = json.loads(out.read_text())
        self.assertEqual(rep["base_url_diffs"], [])
        self.assertEqual(sorted(d["id"] for d in rep["key_env_diffs"]), ["keyless", "subset_env"])

        # --apply must not add env_var onto an auth:"none" row that had none
        _run_sync(
            [
                "--apply", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs), "--output", str(out),
            ]
        )
        updated = {r["name"]: r for r in json.loads(reg_p.read_text())}
        self.assertNotIn("env_var", updated["keyless"])

        # slim_models_dev_snapshot must never copy raw sensitive api URLs
        slim = sync.slim_models_dev_snapshot(
            {"vendor": {"id": "vendor", "api": f"https://user:{secret}@vendor.invalid/v1?token={secret}"}}
        )
        self.assertNotIn(secret, json.dumps(slim))

    def test_fetch_metadata_body_limit_and_truncated_gzip(self):
        reg_p, ali_p, rec_p, _ = self._fixture(
            rows=[{"name": "v", "display": "V", "base_url": "https://api.example.com/v1", "env_var": "V_KEY"}],
            up_providers={},
        )
        payload = json.dumps({"v": {"id": "v", "api": "https://api.example.com/v1", "env": ["V_KEY"]}}).encode("utf-8")

        class _FakeResp:
            def __init__(self, body: bytes, encoding: str = ""):
                self._bio = io.BytesIO(body)
                self.headers = {"Content-Encoding": encoding} if encoding else {}

            def read(self, n: int = -1) -> bytes:
                return self._bio.read(n)

            def __enter__(self):
                return self

            def __exit__(self, *_a):
                return False

        # Live --fetch populates repo_commit, collected_at, and duration_ms
        out = self.dir / "live.json"
        with mock.patch("urllib.request.urlopen", return_value=_FakeResp(payload)):
            code, _, err = _run_sync(
                [
                    "--report", "--source", "models-dev", "--fetch",
                    "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                    "--packages-rs", str(self.packages_rs), "--output", str(out),
                ]
            )
        self.assertEqual(code, 0, err)
        live = json.loads(out.read_text())
        self.assertTrue(live["repo_commit"])
        self.assertRegex(live["collected_at"], r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
        self.assertIsInstance(live["duration_ms"], int)

        # Oversized body fails cleanly with exit 1
        with mock.patch("urllib.request.urlopen", return_value=_FakeResp(payload)):
            code, _, err = _run_sync(
                [
                    "--report", "--source", "models-dev", "--fetch", "--max-fetch-bytes", "10",
                    "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                    "--packages-rs", str(self.packages_rs),
                ]
            )
        self.assertEqual(code, 1)
        self.assertIn("byte limit", err)

        # Truncated gzip stream raises RegistryInputError (exit 1, no unhandled EOFError)
        gz_trunc = gzip.compress(payload)[:-6]
        with mock.patch("urllib.request.urlopen", return_value=_FakeResp(gz_trunc, encoding="gzip")):
            code, _, err = _run_sync(
                [
                    "--report", "--source", "models-dev", "--fetch",
                    "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                    "--packages-rs", str(self.packages_rs),
                ]
            )
        self.assertEqual(code, 1)
        self.assertIn("fetch failed", err)

        # Malformed/sensitive --fetch-url rejected before Request() without leaking query secret
        sec = "fetch_secret_xyz_42"
        code, stdout, err = _run_sync(
            [
                "--report", "--source", "models-dev", "--fetch",
                "--fetch-url", f"ftp://user:{sec}@bad.invalid/api.json?k={sec}",
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(code, 1)
        self.assertNotIn(sec, stdout + err)

    def test_sol_audit_regressions_apply_canonical_alias_precedence_and_validation(self):
        # 1. --apply writes canonical lowercase https:// (never uppercase HTTPS://),
        #    clears optional env_var on auth:"none" when upstream is [], and respects exact-ID precedence over alias
        reg_p, ali_p, rec_p, snap_p = self._fixture(
            rows=[
                {"name": "upper_v", "display": "U", "base_url": "https://old.example/v1", "env_var": "U_KEY"},
                {"name": "opt_clear", "display": "O", "base_url": "http://127.0.0.1:8080/v1", "auth": "none", "env_var": "OLD_OPT"},
                {"name": "fireworks", "display": "F", "base_url": "https://api.fireworks.ai/v1", "env_var": "FW_KEY"},
            ],
            up_providers={
                "upper_v": {"id": "upper_v", "api": "HTTPS://NEW.EXAMPLE.COM:443/v1", "env": ["U_KEY"]},
                "opt_clear": {"id": "opt_clear", "api": "http://127.0.0.1:8080/v1", "env": []},
                "fireworks": {"id": "fireworks", "api": "https://api.fireworks.ai/v1", "env": ["FW_KEY"]},
                "fireworks-ai": {"id": "fireworks-ai", "api": "https://shadowed-alias.example/v1", "env": ["FW_KEY"]},
            },
            aliases={"fireworks-ai": "fireworks"},
        )
        out = self.dir / "apply.json"
        code, _, err = _run_sync(
            [
                "--apply", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs), "--output", str(out),
            ]
        )
        self.assertEqual(code, 0, err)
        updated = {r["name"]: r for r in json.loads(reg_p.read_text())}
        self.assertEqual(updated["upper_v"]["base_url"], "https://new.example.com/v1")
        self.assertNotIn("env_var", updated["opt_clear"])
        self.assertEqual(updated["fireworks"]["base_url"], "https://api.fireworks.ai/v1")

        # Post-apply --check must now pass cleanly!
        chk_code, _, chk_err = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(chk_code, 0, chk_err)

        # 2. --check failure prints concrete differing preset details to stderr
        _write_json(
            snap_p,
            {"schema_version": 1, "source_url": "https://models.dev/api.json", "providers": {"upper_v": {"id": "upper_v", "api": "https://drift.example/v1", "env": ["U_KEY"]}}},
        )
        fail_code, _, fail_err = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(snap_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(fail_code, 1)
        self.assertIn("upper_v", fail_err)
        self.assertIn("https://drift.example/v1", fail_err)

        # 3. Reconcile source_url with query secret rejected; mixed env_var list rejected; missing reconcile rejected
        bad_rec = _write_json(
            self.dir / "bad_rec.json",
            {
                "schema_version": 1,
                "overrides": [],
                "entries": {"upper_v": {"note": "n", "source_url": "https://docs.example.com/?token=secret_123"}},
            },
        )
        with self.assertRaises(sync.RegistryInputError):
            sync.load_reconcile(bad_rec, {"upper_v"})
        bad_rec2 = _write_json(
            self.dir / "bad_rec2.json",
            {
                "schema_version": 1,
                "overrides": [
                    {"id": "upper_v", "upstream": "models-dev", "field": "env_var", "local_value": "U_KEY", "upstream_value": [1, "U_KEY"], "reason": "r"}
                ],
                "entries": {},
            },
        )
        with self.assertRaises(sync.RegistryInputError):
            sync.load_reconcile(bad_rec2, {"upper_v"})
        with self.assertRaises(sync.RegistryInputError):
            sync.load_reconcile(self.dir / "does_not_exist.json", {"upper_v"})

        # 4. Wrapper validation rejects missing schema_version / bool schema_version, and --write-snapshot validates before writing
        with self.assertRaises(sync.RegistryInputError):
            sync.parse_models_dev({"providers": {"upper_v": {"id": "upper_v"}}})
        with self.assertRaises(sync.RegistryInputError):
            sync.parse_models_dev({"schema_version": True, "providers": {}})
        bad_snap = _write_json(
            self.dir / "bad_snap.json",
            {"schema_version": 1, "source_url": "https://custom.example/api.json", "providers": {"upper_v": {"id": "upper_v", "env": [""]}}},
        )
        target_snap = self.dir / "should_not_exist.json"
        wcode, _, _ = _run_sync(
            [
                "--write-snapshot", str(target_snap), "--snapshot", str(bad_snap),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(wcode, 1)
        self.assertFalse(target_snap.exists())

        # Re-slimming a valid snapshot preserves its original source_url
        good_snap = _write_json(
            self.dir / "good_snap.json",
            {"schema_version": 1, "source_url": "https://custom.example/api.json", "providers": {"upper_v": {"id": "upper_v", "api": "https://new.example.com/v1", "env": ["U_KEY"]}}},
        )
        reslim_p = self.dir / "reslim.json"
        wcode2, _, werr2 = _run_sync(
            [
                "--write-snapshot", str(reslim_p), "--snapshot", str(good_snap),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(wcode2, 0, werr2)
        self.assertEqual(json.loads(reslim_p.read_text())["source_url"], "https://custom.example/api.json")

        # 5. LiteLLM run does not mark models-dev overrides stale and records content_hash
        litellm_dir = self.dir / "litellm_repo"
        trans = litellm_dir / "litellm" / "llms" / "upper_v" / "chat" / "transformation.py"
        trans.parent.mkdir(parents=True, exist_ok=True)
        trans.write_text('DEFAULT_API_BASE = "https://new.example.com/v1"\nkey = os.getenv("U_KEY")\n', encoding="utf-8")
        rec_with_md = _write_json(
            self.dir / "rec_md.json",
            {
                "schema_version": 1,
                "overrides": [
                    {"id": "upper_v", "upstream": "models-dev", "field": "base_url", "local_value": "https://new.example.com/v1", "upstream_value": "https://other.example/v1", "reason": "md only"}
                ],
                "entries": {},
            },
        )
        lm_out = self.dir / "litellm_report.json"
        lm_code, _, lm_err = _run_sync(
            [
                "--report", "--source", "litellm", "--checkout", str(litellm_dir),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_with_md),
                "--packages-rs", str(self.packages_rs), "--output", str(lm_out),
            ]
        )
        self.assertEqual(lm_code, 0, lm_err)
        lm_rep = json.loads(lm_out.read_text())
        self.assertEqual(lm_rep["counts"]["stale_overrides"], 0)
        self.assertTrue(lm_rep["sources"][0]["content_hash"])

        # 6. Duplicate JSON keys & invalid Rust profile/derive fields rejected
        dup_json = self.dir / "dup_aliases.json"
        dup_json.write_text('{"a": "upper_v", "a": "fireworks"}\n', encoding="utf-8")
        with self.assertRaises(sync.RegistryInputError):
            sync.load_aliases(dup_json, {"upper_v", "fireworks"})
        bad_reg = _write_json(
            self.dir / "bad_reg.json",
            [{"name": "bad", "display": "B", "base_url": "https://a.example/v1", "env_var": "B_KEY", "profile": {"structured_output_models": "not-a-list"}}],
        )
        with self.assertRaises(sync.RegistryInputError):
            sync.load_local_registry(bad_reg)
        # Output path or --write-snapshot colliding with registry input rejected (exit 2)
        col_code, _, _ = _run_sync(
            [
                "--report", "--source", "models-dev", "--snapshot", str(good_snap),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs), "--output", str(reg_p),
            ]
        )
        self.assertEqual(col_code, 2)
        ws_col_code, _, _ = _run_sync(
            [
                "--check", "--source", "models-dev", "--snapshot", str(good_snap),
                "--write-snapshot", str(reg_p),
                "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                "--packages-rs", str(self.packages_rs),
            ]
        )
        self.assertEqual(ws_col_code, 2)

        # Duplicate URL-shaped and templated JSON keys redact secrets in errors
        dup_sec = "dup_key_secret_777"
        for raw_key in (f"https://a.invalid/?k={dup_sec}", f"${{ENDPOINT}}/user:{dup_sec}"):
            dup_url_json = self.dir / "dup_url.json"
            dup_url_json.write_text(f'{{"{raw_key}": "x", "{raw_key}": "y"}}\n', encoding="utf-8")
            with self.assertRaises(sync.RegistryInputError) as ctx:
                sync.load_json_file(dup_url_json, "dup")
            self.assertNotIn(dup_sec, str(ctx.exception))

        # Case-insensitive collision check on macOS/Windows for --write-snapshot vs registry / output
        if sys.platform in ("darwin", "win32"):
            upper_ws = reg_p.parent / reg_p.name.upper()
            ci_code, _, _ = _run_sync(
                [
                    "--check", "--source", "models-dev", "--snapshot", str(good_snap),
                    "--write-snapshot", str(upper_ws),
                    "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                    "--packages-rs", str(self.packages_rs),
                ]
            )
            self.assertEqual(ci_code, 2)
            ci_out_code, _, _ = _run_sync(
                [
                    "--report", "--source", "models-dev", "--snapshot", str(good_snap),
                    "--write-snapshot", str(self.dir / "out_pair.json"),
                    "--output", str(self.dir / "OUT_PAIR.JSON"),
                    "--registry", str(reg_p), "--aliases", str(ali_p), "--reconcile", str(rec_p),
                    "--packages-rs", str(self.packages_rs),
                ]
            )
            self.assertEqual(ci_out_code, 2)

        # Local uppercase HTTPS:// rejected to match preset/mod.rs:242; partial wrapper with metadata rejected
        upper_reg = _write_json(
            self.dir / "upper_reg.json",
            [{"name": "up", "display": "U", "base_url": "HTTPS://a.example/v1", "env_var": "U_KEY"}],
        )
        with self.assertRaises(sync.RegistryInputError):
            sync.load_local_registry(upper_reg)
        with self.assertRaises(sync.RegistryInputError):
            sync.parse_models_dev({"providers": {"up": {"id": "up"}}, "metadata": {"x": 1}})

        # _atomic_write_text preserves symlink and file mode (0644)
        import stat

        real_file = self.dir / "real_reg.json"
        real_file.write_text("[]\n", encoding="utf-8")
        os.chmod(real_file, 0o644)
        link_file = self.dir / "link_reg.json"
        os.symlink(real_file, link_file)
        sync._atomic_write_text(link_file, '[{"ok": true}]\n')
        self.assertTrue(link_file.is_symlink())
        self.assertEqual(real_file.read_text(encoding="utf-8"), '[{"ok": true}]\n')
        self.assertEqual(stat.S_IMODE(real_file.stat().st_mode), 0o644)

        # Optional env_var is NOT cleared by --apply when upstream env only contains config_env_names (not explicit [])
        cfg_reg, cfg_ali, cfg_rec, cfg_snap = self._fixture(
            rows=[{"name": "opt_cfg", "display": "O", "base_url": "http://127.0.0.1:8080/v1", "auth": "none", "env_var": "OPT_KEY", "base_url_env": "OPT_BASE"}],
            up_providers={"opt_cfg": {"id": "opt_cfg", "api": "http://127.0.0.1:8080/v1", "env": ["OPT_BASE"]}},
        )
        _run_sync(
            [
                "--apply", "--source", "models-dev", "--snapshot", str(cfg_snap),
                "--registry", str(cfg_reg), "--aliases", str(cfg_ali), "--reconcile", str(cfg_rec),
                "--packages-rs", str(self.packages_rs), "--output", str(self.dir / "cfg_apply.json"),
            ]
        )
        self.assertEqual(json.loads(cfg_reg.read_text())[0].get("env_var"), "OPT_KEY")


if __name__ == "__main__":
    unittest.main(verbosity=2)
