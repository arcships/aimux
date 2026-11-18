#!/usr/bin/env python3
"""Offline SSRF, redirect, classification, and budget tests for scripts/probe_registry.py (RFC-0033 §8)."""

from __future__ import annotations

import contextlib
import io
import json
import os
import socket
import ssl
import sys
import tempfile
import threading
import time
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
        arg_out, arg_err = io.StringIO(), io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "allow.json"
            p.write_text('{"schema_version":1,"entries":[]}\n', encoding="utf-8")
            with contextlib.redirect_stdout(arg_out), contextlib.redirect_stderr(arg_err):
                code = probe.main(["--allowlist", str(p), "--timeout", "0"])
            self.assertEqual(code, 2)

    def test_dns_tls_connect_timeout_and_duplicate_allowlist_fixtures(self):
        # dns-error on empty resolution and OSError
        for res_fn in (lambda h, p: [], lambda h, p: (_ for _ in ()).throw(OSError("dns down"))):
            rep = probe.run_probe([_entry("d1")], registry_urls={}, resolver=res_fn, http_exchange=lambda *_a: (200, {}))
            self.assertEqual(rep["results"][0]["outcome"], "dns-error")

        # tls-error and connect-error (including socket.timeout -> timeout on Python <3.10)
        for exc, expected in (
            (probe.ProbeTlsError("cert"), "tls-error"),
            (probe.ProbeConnectError("reset"), "connect-error"),
            (OSError("refused"), "connect-error"),
            (socket.timeout("sock timeout"), "timeout"),
        ):
            rep = probe.run_probe(
                [_entry("e1")],
                registry_urls={},
                resolver=lambda h, p: ["93.184.216.34"],
                http_exchange=lambda *_a, _e=exc: (_ for _ in ()).throw(_e),
            )
            self.assertEqual(rep["results"][0]["outcome"], expected)

        # _default_http_exchange maps ssl.SSLError to ProbeTlsError
        fake_sock = mock.MagicMock()
        fake_ctx = mock.MagicMock()
        fake_ctx.wrap_socket.side_effect = ssl.SSLError("bad cert")
        with mock.patch("socket.create_connection", return_value=fake_sock), mock.patch(
            "ssl.create_default_context", return_value=fake_ctx
        ):
            with self.assertRaises(probe.ProbeTlsError):
                probe._default_http_exchange("93.184.216.34", 443, "https", "api.example.com", "/v1/models", {}, 1.0, 1024, 1024)

        # Duplicate allowlist IDs fail input validation (exit 1)
        with tempfile.TemporaryDirectory() as tmp:
            dup_p = Path(tmp) / "dup_allow.json"
            dup_p.write_text(
                json.dumps({"schema_version": 1, "entries": [_entry("dup"), _entry("dup")]}),
                encoding="utf-8",
            )
            with self.assertRaises(probe.ProbeInputError):
                probe.load_allowlist(dup_p)
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                self.assertEqual(probe.main(["--allowlist", str(dup_p), "--no-registry-check"]), 1)

    def test_total_budget_enforced_inside_wave_and_report_metadata(self):
        gate = threading.Event()

        def slow_exchange(_ip, _p, _s, _h, path, *_rest):
            if "slow" in path:
                gate.wait(0.45)
            return 200, {}

        entries = [
            _entry("a_fast", base_path="/fast"),
            _entry("b_slow1", base_path="/slow1"),
            _entry("c_slow2", base_path="/slow2"),
            _entry("d_queued", base_path="/queued"),
        ]
        t0 = time.monotonic()
        rep = probe.run_probe(
            entries,
            registry_urls={},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=slow_exchange,
            concurrency=2,
            total_budget=0.08,
        )
        elapsed = time.monotonic() - t0
        gate.set()

        self.assertLess(elapsed, 0.35)
        self.assertTrue(rep["repo_commit"])
        self.assertRegex(rep["collected_at"], r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
        self.assertIsInstance(rep["duration_ms"], int)
        by_id = {r["id"]: r["outcome"] for r in rep["results"]}
        self.assertEqual(by_id["a_fast"], "reachable")
        self.assertEqual(by_id["b_slow1"], "not-run")
        self.assertEqual(by_id["c_slow2"], "not-run")
        self.assertEqual(by_id["d_queued"], "not-run")

        # Hung resolver respects per-target timeout
        dns_t_rep = probe.run_probe(
            [_entry("dns_slow")],
            registry_urls={},
            resolver=lambda h, p: (time.sleep(0.35), ["93.184.216.34"])[1],
            http_exchange=lambda *_a: (200, {}),
            timeout=0.05,
            total_budget=1.0,
        )
        self.assertEqual(dns_t_rep["results"][0]["outcome"], "timeout")

        # Full Python subprocess exits promptly on total_budget even when a resolver hangs for 5s
        import subprocess

        with tempfile.TemporaryDirectory() as tmp:
            allow_p = Path(tmp) / "allow.json"
            allow_p.write_text(
                json.dumps({"schema_version": 1, "entries": [_entry("hung_dns")]}),
                encoding="utf-8",
            )
            script = (
                "import socket, sys, time; "
                f"sys.path.insert(0, {str(ROOT / 'scripts')!r}); "
                "import probe_registry as p; "
                "socket.getaddrinfo = lambda *_a, **_kw: (time.sleep(5.0), [])[1]; "
                f"sys.exit(p.main(['--allowlist', {str(allow_p)!r}, '--no-registry-check', '--total-budget', '0.08']))"
            )
            t_sub = time.monotonic()
            proc = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True, timeout=2.0)
            sub_elapsed = time.monotonic() - t_sub
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertLess(sub_elapsed, 0.8)
            sub_rep = json.loads(proc.stdout)
            self.assertEqual(sub_rep["results"][0]["outcome"], "not-run")

    def test_redirect_traversal_encoding_and_exception_isolation(self):
        # Encoded dot segments on same origin must be blocked
        for loc in ("/v1/models/%2e%2e/%2e%2e/admin", "/v1/models/..%2f..%2fadmin"):
            rep = probe.run_probe(
                [_entry("trav")],
                registry_urls={},
                resolver=lambda h, p: ["93.184.216.34"],
                http_exchange=lambda *_a, _l=loc: (302, {"location": _l}),
            )
            self.assertEqual(rep["results"][0]["outcome"], "blocked-cross-origin-redirect")

        # Malformed Location / non-ASCII path / latin-1 digit status never abort sibling targets
        def multi_http(_ip, _p, _s, _h, path, *_rest):
            if path.startswith("/badloc"):
                return 302, {"location": "https://[bad"}
            if path.startswith("/nonascii"):
                if "caf" in path:
                    raise probe.ProbeConnectError("non-ASCII HTTP request target")
                return 302, {"location": "/nonascii/models/café"}
            return 200, {}

        rep = probe.run_probe(
            [
                _entry("a_ok", base_path="/ok"),
                _entry("b_badloc", base_path="/badloc"),
                _entry("c_nonascii", base_path="/nonascii"),
            ],
            registry_urls={},
            resolver=lambda h, p: ["93.184.216.34"],
            http_exchange=multi_http,
        )
        by_id = {r["id"]: r for r in rep["results"]}
        self.assertEqual(by_id["a_ok"]["outcome"], "reachable")
        self.assertEqual(by_id["b_badloc"]["outcome"], "blocked-invalid-url")
        self.assertEqual(by_id["b_badloc"]["final_target"], rs.REDACTED_URL)
        self.assertEqual(rep["input_hash_kind"], "sanitized")
        self.assertEqual(by_id["c_nonascii"]["outcome"], "connect-error")

        # Non-ASCII latin-1 digit in HTTP status line raises ProbeConnectError (not ValueError)
        with self.assertRaises(probe.ProbeConnectError):
            probe._read_status_and_headers(io.BufferedReader(io.BytesIO(b"HTTP/1.1 \xb200 OK\r\n\r\n")), 1024)

        # _default_http_exchange rejects non-ASCII path with ProbeConnectError (not UnicodeEncodeError)
        fake_sock = mock.MagicMock()
        with mock.patch("socket.create_connection", return_value=fake_sock):
            with self.assertRaises(probe.ProbeConnectError):
                probe._default_http_exchange(
                    "93.184.216.34", 80, "http", "api.example.com", "/v1/models/café", {}, 1.0, 1024, 1024
                )


if __name__ == "__main__":
    unittest.main(verbosity=2)
