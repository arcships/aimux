#!/usr/bin/env python3
"""Unit and negative tests for RFC-0043 §7 performance comparator and bench fixtures."""

from __future__ import annotations

import copy
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent


def _load_module(name: str, path: pathlib.Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


compare_perf = _load_module("compare_perf", ROOT / "scripts" / "quality" / "compare-perf.py")
gen_fixtures = _load_module("generate_fixtures", ROOT / "bench" / "generate_fixtures.py")


def _make_bundle(
    *,
    scenario: str = "request-small",
    backend: str = "rust",
    operation_timeout_ms: int = 5000,
    run_timeout_ms: int = 600000,
    recipe_hash: str = "recipe-v1",
    concurrency: int = 1,
    runs: list[dict] | None = None,
) -> dict:
    if runs is None:
        runs = [
            {
                "pair_index": i,
                "samples": 10000,
                "timed_out_operations": 0,
                "run_timed_out": False,
                "correctness_failures": 0,
                "leaked_handles": 0,
                "b0_drift_pct": 0.4,
                "cpu_steal_pct": 0.1,
                "background_cpu_pct": 0.5,
                "metrics": {
                    "p50_ns": 200_000 + (i % 3) * 1000,
                    "p95_ns": 450_000 + (i % 3) * 2000,
                },
            }
            for i in range(10)
        ]
    return {
        "schema_version": 1,
        "scenario": scenario,
        "backend": backend,
        "operation_timeout_ms": operation_timeout_ms,
        "run_timeout_ms": run_timeout_ms,
        "recipe_hash": recipe_hash,
        "concurrency": concurrency,
        "runs": runs,
    }


class ComparePerfTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.scenarios = compare_perf.load_json_strict(ROOT / "bench" / "scenarios.json")
        cls.environment = compare_perf.load_json_strict(ROOT / "bench" / "environment.json")

    def test_real_configs_and_fixtures_validate(self) -> None:
        compare_perf.validate_environment_config(self.environment)
        by_id = compare_perf.validate_scenarios_config(self.scenarios)
        self.assertGreaterEqual(len(by_id), 18)
        errors = gen_fixtures.check_manifest(ROOT / "bench" / "fixtures" / "manifest.json")
        self.assertEqual(errors, [])

    def test_fixture_generator_exact_byte_sizes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            code = gen_fixtures.main(["--out-dir", str(out_dir)])
            self.assertEqual(code, 0)
            for spec in gen_fixtures.FIXTURE_SPECS:
                raw = (out_dir / f"{spec['id']}.json").read_bytes()
                self.assertEqual(len(raw), spec["exact_bytes"], spec["id"])
                json.loads(raw.decode("utf-8"))

    def test_passes_clean_pairs_and_sub_floor_latency_delta(self) -> None:
        base = _make_bundle()
        head = copy.deepcopy(base)
        # 100 us -> 103.5 us is within 5% AND small absolute deltas <= 5 us pass.
        for r in head["runs"]:
            r["metrics"]["p50_ns"] = r["metrics"]["p50_ns"] + 3_500
        anchor = copy.deepcopy(base)
        report = compare_perf.evaluate_comparison(
            base, head, self.scenarios, anchor_raw=anchor, seed=42
        )
        self.assertEqual(report["status"], "PASS", report)
        self.assertEqual(report["failure_codes"], [])

        # Even if relative ratio > 5% on a very fast op (50 us -> 54 us = +8%),
        # absolute delta (+4 us < 5 us floor) stays PASS per §7.4.
        fast_base = _make_bundle(
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "metrics": {"p50_ns": 50_000, "p95_ns": 90_000},
                }
                for i in range(10)
            ]
        )
        fast_head = copy.deepcopy(fast_base)
        for r in fast_head["runs"]:
            r["metrics"]["p50_ns"] = 54_000
        fast_report = compare_perf.evaluate_comparison(fast_base, fast_head, self.scenarios, seed=42)
        self.assertEqual(fast_report["status"], "PASS", fast_report)

    def test_detects_latency_and_throughput_regression(self) -> None:
        base = _make_bundle()
        head = copy.deepcopy(base)
        for r in head["runs"]:
            r["metrics"]["p50_ns"] = int(r["metrics"]["p50_ns"] * 1.15)
        report = compare_perf.evaluate_comparison(base, head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("PERF_REGRESSION", report["failure_codes"])

        # Throughput decrease > 5% with no floor exemption triggers PERF_REGRESSION.
        tp_base = _make_bundle(
            scenario="stream-small-chunks",
            operation_timeout_ms=15000,
            runs=[
                {
                    "pair_index": i,
                    "samples": 100,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "metrics": {
                        "throughput_events_per_s": 50_000.0 + (i % 3) * 100.0,
                        "throughput_bytes_per_s": 1_600_000.0,
                        "ttft_p95_ns": 1_000_000,
                    },
                }
                for i in range(10)
            ],
        )
        tp_head = copy.deepcopy(tp_base)
        for r in tp_head["runs"]:
            r["metrics"]["throughput_events_per_s"] *= 0.90
        tp_report = compare_perf.evaluate_comparison(tp_base, tp_head, self.scenarios, seed=42)
        self.assertEqual(tp_report["status"], "FAIL")
        self.assertIn("PERF_REGRESSION", tp_report["failure_codes"])

    def test_inconclusive_ci_never_passes(self) -> None:
        # 5 runs +8%, 5 runs +1%: 99% bootstrap CI straddles the 5% budget boundary.
        base = _make_bundle()
        head = copy.deepcopy(base)
        for i, r in enumerate(head["runs"]):
            factor = 1.09 if i % 2 == 0 else 1.005
            r["metrics"]["p50_ns"] = int(r["metrics"]["p50_ns"] * factor)
        report = compare_perf.evaluate_comparison(base, head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("PERF_INCONCLUSIVE", report["failure_codes"])

    def test_env_instability_and_cpu_steal_fail(self) -> None:
        base = _make_bundle()
        # Inject >3% robust dispersion into base P50 series.
        for i, r in enumerate(base["runs"]):
            r["metrics"]["p50_ns"] = 200_000 if i % 2 == 0 else 225_000
        head = copy.deepcopy(base)
        report = compare_perf.evaluate_comparison(base, head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("BENCH_ENV_UNSTABLE", report["failure_codes"])

        clean = _make_bundle()
        steal_head = copy.deepcopy(clean)
        steal_head["runs"][0]["cpu_steal_pct"] = 1.8
        steal_rep = compare_perf.evaluate_comparison(clean, steal_head, self.scenarios, seed=42)
        self.assertEqual(steal_rep["status"], "FAIL")
        self.assertIn("BENCH_ENV_UNSTABLE", steal_rep["failure_codes"])

    def test_absolute_guardrail_and_theil_sen_slope_fail_even_if_base_matches(self) -> None:
        # Both base and head at 1.5 ms (> 1.0 ms guardrail for request-small).
        slow_base = _make_bundle(
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "metrics": {"p50_ns": 1_500_000, "p95_ns": 1_800_000},
                }
                for i in range(10)
            ]
        )
        slow_head = copy.deepcopy(slow_base)
        report = compare_perf.evaluate_comparison(slow_base, slow_head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", report["failure_codes"])

        # Steady RSS leak over tail 15 windows: +700 KiB/window > 0.5 MiB/1k requests limit.
        windows = [50 * 1024 * 1024 + w * 700_000 for w in range(20)]
        rss_base = _make_bundle(
            scenario="sustained",
            runs=[
                {
                    "pair_index": i,
                    "samples": 20000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "rss_windows_bytes": [50 * 1024 * 1024] * 20,
                    "metrics": {
                        "retained_rss_bytes": 4 * 1024 * 1024,
                        "peak_rss_bytes": 80 * 1024 * 1024,
                    },
                }
                for i in range(10)
            ],
        )
        rss_head = copy.deepcopy(rss_base)
        rss_head["runs"][0]["rss_windows_bytes"] = windows
        slope_rep = compare_perf.evaluate_comparison(rss_base, rss_head, self.scenarios, seed=42)
        self.assertEqual(slope_rep["status"], "FAIL")
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", slope_rep["failure_codes"])

    def test_timeout_correctness_and_config_invalid_codes(self) -> None:
        base = _make_bundle()

        to_head = copy.deepcopy(base)
        to_head["runs"][2]["timed_out_operations"] = 1
        rep_to = compare_perf.evaluate_comparison(base, to_head, self.scenarios, seed=42)
        self.assertIn("BENCH_TIMEOUT", rep_to["failure_codes"])

        leak_head = copy.deepcopy(base)
        leak_head["runs"][0]["leaked_handles"] = 1
        rep_leak = compare_perf.evaluate_comparison(base, leak_head, self.scenarios, seed=42)
        self.assertIn("BENCH_CORRECTNESS_FAILURE", rep_leak["failure_codes"])

        mismatch_head = copy.deepcopy(base)
        mismatch_head["operation_timeout_ms"] = 4999
        rep_cfg = compare_perf.evaluate_comparison(base, mismatch_head, self.scenarios, seed=42)
        self.assertEqual(rep_cfg["failure_codes"], ["BENCH_CONFIG_INVALID"])

        with tempfile.TemporaryDirectory() as tmp:
            p = pathlib.Path(tmp) / "dup.json"
            p.write_text('{"a": 1, "a": 2}', encoding="utf-8")
            with self.assertRaises(compare_perf.ConfigError):
                compare_perf.load_json_strict(p)

    def test_cli_validate_and_compare(self) -> None:
        out = io.StringIO()
        err = io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = compare_perf.main(
                [
                    "--validate-scenarios",
                    str(ROOT / "bench" / "scenarios.json"),
                    "--environment",
                    str(ROOT / "bench" / "environment.json"),
                ]
            )
        self.assertEqual(rc, 0, err.getvalue())
        self.assertIn("perf scenarios ok", out.getvalue())

        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            base_p = d / "base.json"
            head_p = d / "head.json"
            out_p = d / "report.json"
            base_p.write_text(json.dumps(_make_bundle()), encoding="utf-8")
            head_p.write_text(json.dumps(_make_bundle()), encoding="utf-8")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc2 = compare_perf.main(
                    ["--base", str(base_p), "--head", str(head_p), "--out", str(out_p)]
                )
            self.assertEqual(rc2, 0)
            self.assertEqual(json.loads(out_p.read_text(encoding="utf-8"))["status"], "PASS")


if __name__ == "__main__":
    unittest.main()
