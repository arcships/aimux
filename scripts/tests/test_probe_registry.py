#!/usr/bin/env python3
"""Offline SSRF, redirect, classification, and budget tests for scripts/probe_registry.py (RFC-0033 §8)."""

from __future__ import annotations

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import probe_registry as probe  # noqa: E402
import registry_sanitize as rs  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def _entry(pid: str, origin: str = "https://api.example.com", base_path: str = "/v1") -> dict:
    return {
        "id": pid,
        "origin": origin,
        "base_path": base_path,
        "source": f"test#{pid}",
    }


class ProbeRegistryTest(unittest.TestCase):
    def test_repo_allowlist_and_registry_stay_valid_offline(self):
        entries = probe.load_allowlist(ROOT / "registry" / "probe_allowlist.json")
        reg = probe.load_registry_urls(ROOT / "aimux-providers" / "src" / "provider_registry.json")
        self.assertGreater(len(entries), 200)
        for e in entries:
            self.assertIn(e["id"], reg)
            info = rs.sanitize_url(reg[e["id"]])
            self.assertEqual(info["kind"], "static")
            self.assertEqual(info["origin"], e["origin"])
            self.assertEqual(info["path"] or "", e["base_path"])

    def test_blocks_private_loopback_metadata_ipv6_mapped_and_mixed_dns(self):
        cases = [
            ("local_name", "https://localhost", lambda h, p: ["93.184.216.34"], "blocked-non-public-ip"),
            ("internal_name", "https://svc.internal", lambda h, p: ["93.184.216.34"], "blocked-non-public-ip"),
            ("loopback_v4", "https://api.example.com", lambda h, p: ["127.0.0.1"], "blocked-non-public-ip"),
            ("private_10", "https://api.example.com", lambda h, p: ["10.0.0.5"], "blocked-non-public-ip"),
            ("metadata", "https://api.example.com", lambda h, p: ["169.254.169.254"], "blocked-non-public-ip"),
            ("cgnat", "https://api.example.com", lambda h, p: ["100.64.1.1"], "blocked-non-public-ip"),
            ("loopback_v6", "https://api.example.com", lambda h, p: ["::1"], "blocked-non-public-ip"),
            ("link_v6", "https://api.example.com", lambda h, p: ["fe80::1"], "blocked-non-public-ip"),
            ("ula_v6", "https://api.example.com", lambda h, p: ["fd12:3456::1"], "blocked-non-public-ip"),
            ("mapped_v4", "https://api.example.com", lambda h, p: ["::ffff:127.0.0.1"], "blocked-non-public-ip"),
            ("mixed_dns", "https://api.example.com", lambda h, p: ["93.184.216.34", "192.168.1.1"], "blocked-non-public-ip"),
            ("literal_meta", "http://169.254.169.254", lambda h, p: ["169.254.169.254"], "blocked-non-public-ip"),
            ("sensitive_q", "https://api.example.com?k=secret", lambda h, p: ["93.184.216.34"], "blocked-sensitive-url"),
            ("sensitive_u", "https://user:pass@api.example.com", lambda h, p: ["93.184.216.34"], "blocked-sensitive-url"),
        ]
        called = False

        def forbid_http(*_args, **_kwargs):
            nonlocal called
            called = True
            return 200, {}

        for pid, origin, resolver, expected in cases:
            report = probe.run_probe(
                [_entry(pid, origin=origin)],
                registry_urls={},
                resolver=resolver,
                http_exchange=forbid_http,
            )
            self.assertEqual(report["results"][0]["outcome"], expected, pid)
        self.assertFalse(called, "blocked targets must never dial network")

    def test_unapproved_registry_drift_blocks_without_network(self):
        called = False

        def forbid_http(*_args, **_kwargs):
            nonlocal called
            called = True
            return 200, {}

        report = probe.run_probe(
            [_entry("drifted", origin="https://api.example.com", base_path="/v1")],
            registry_urls={"drifted": "https://other.example.com/v1"},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=forbid_http,
        )
        self.assertEqual(report["results"][0]["outcome"], "blocked-unapproved")
        self.assertFalse(called)

    def test_redirect_security_cross_origin_downgrade_rebinding_loop_and_redaction(self):
        # Cross-origin blocked, https->http downgrade blocked, sensitive Location redacted,
        # DNS rebinding across hops blocked, loop blocked.
        scenarios = [
            ("cross", {"Location": "https://evil.example.net/v1/models"}, lambda h, p: ["93.184.216.34"], "blocked-cross-origin-redirect"),
            ("downgrade", {"Location": "http://api.example.com/v1/models/"}, lambda h, p: ["93.184.216.34"], "blocked-https-downgrade"),
            ("leak_loc", {"Location": "https://api.example.com/v1/models/?token=leak_me_not"}, lambda h, p: ["93.184.216.34"], "blocked-sensitive-url"),
            ("loop", {"Location": "/v1/models"}, lambda h, p: ["93.184.216.34"], "blocked-redirect-loop"),
        ]
        for pid, hdrs, resolver, expected in scenarios:
            report = probe.run_probe(
                [_entry(pid)],
                registry_urls={},
                resolver=resolver,
                http_exchange=lambda *_a: (302, {"location": hdrs["Location"]}),
                max_redirects=3,
            )
            res = report["results"][0]
            self.assertEqual(res["outcome"], expected, pid)
            self.assertNotIn("leak_me_not", json.dumps(report))

        # DNS rebinding: first resolution public, second hop resolution returns private IP
        hops = iter([["93.184.216.34"], ["10.0.0.1"]])

        def rebind_http(_ip, _p, _s, _h, path, *_rest):
            if path == "/v1/models":
                return 302, {"location": "/v1/models/v2"}
            raise AssertionError("rebound private hop must not dial HTTP")

        reb_report = probe.run_probe(
            [_entry("rebind")],
            registry_urls={},
            resolver=lambda h, p: next(hops),
            http_exchange=rebind_http,
            max_redirects=3,
        )
        self.assertEqual(reb_report["results"][0]["outcome"], "blocked-non-public-ip")

        # max_redirects=0 returns redirect-unverified without following
        zero_report = probe.run_probe(
            [_entry("noredir")],
            registry_urls={},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=lambda *_a: (302, {"location": "/v1/models/"}),
            max_redirects=0,
        )
        self.assertEqual(zero_report["results"][0]["outcome"], "redirect-unverified")

    def test_http_status_classifications_fixed_headers_and_limits(self):
        statuses = [
            (200, "reachable"),
            (401, "reachable-auth-or-policy"),
            (403, "reachable-auth-or-policy"),
            (404, "reachable-path-unsupported"),
            (405, "reachable-path-unsupported"),
            (429, "reachable-rate-limited"),
            (400, "reachable-client-error"),
            (503, "server-error"),
        ]
        seen_headers: list[dict[str, str]] = []

        def status_http(status):
            def _fn(_ip, _port, _scheme, _host, _path, headers, *_rest):
                seen_headers.append(headers)
                return status, {}

            return _fn

        with mock.patch.dict(os.environ, {"OPENAI_API_KEY": "sk-secret-env", "HTTPS_PROXY": "http://127.0.0.1:9"}):
            for st, expected in statuses:
                report = probe.run_probe(
                    [_entry(f"p{st}")],
                    registry_urls={},
                    resolver=lambda h, p: ["93.184.216.34"],
                    http_exchange=status_http(st),
                )
                self.assertEqual(report["results"][0]["outcome"], expected)
        for hdrs in seen_headers:
            lowered = {k.lower(): v for k, v in hdrs.items()}
            self.assertNotIn("authorization", lowered)
            self.assertEqual(lowered["user-agent"], "aimux-registry-probe/1.0")

        # Timeout & limit-exceeded
        t_rep = probe.run_probe(
            [_entry("t1")],
            registry_urls={},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=lambda *_a: (_ for _ in ()).throw(TimeoutError()),
        )
        self.assertEqual(t_rep["results"][0]["outcome"], "timeout")

        l_rep = probe.run_probe(
            [_entry("l1")],
            registry_urls={},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=lambda *_a: (_ for _ in ()).throw(probe.LimitExceededError("big")),
        )
        self.assertEqual(l_rep["results"][0]["outcome"], "limit-exceeded")

    def test_markdown_escapes_mentions_and_html(self):
        report = {
            "total": 1,
            "input_hash_kind": "original",
            "input_hash": "a" * 64,
            "counts": {"connect-error": 1},
            "results": [
                {
                    "id": "@team<script>alert(1)</script>",
                    "target": "https://api.example.com/v1/models",
                    "outcome": "connect-error",
                    "http_status": None,
                }
            ],
        }
        md = probe.render_markdown(report)
        self.assertNotIn("@team", md)
        self.assertNotIn("<script>", md)
        self.assertIn("&#64;team", md)

    def test_cli_rejects_invalid_budgets(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "allow.json"
            p.write_text('{"schema_version":1,"entries":[]}\n', encoding="utf-8")
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = probe.main(["--allowlist", str(p), "--timeout", "0"])
            self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
