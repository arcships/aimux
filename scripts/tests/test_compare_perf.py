#!/usr/bin/env python3
"""Unit and negative tests for RFC-0043 §7 & §8 performance comparator and bench fixtures."""

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
    case: str | None = None,
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
    bundle = {
        "schema_version": 1,
        "scenario": scenario,
        "backend": backend,
        "operation_timeout_ms": operation_timeout_ms,
        "run_timeout_ms": run_timeout_ms,
        "recipe_hash": recipe_hash,
        "concurrency": concurrency,
        "runs": runs,
    }
    if case is not None:
        bundle["case"] = case
    return bundle


def _make_active_environment() -> dict:
    sha = "a" * 64
    return {
        "schema_version": 1,
        "pool_id": "perf-linux-x64-01",
        "status": "active",
        "isolation_requirements": {
            "min_exclusive_physical_cores": 4,
            "core_allocation": {
                "client_and_runtime": 2,
                "mock": 1,
                "sampler": 1,
                "smt_siblings_isolated": True,
            },
            "min_ram_gib": 16,
            "tokio_worker_threads": 2,
            "max_cpu_steal_pct": 1.0,
            "max_background_cpu_pct": 5.0,
            "allow_target_cpu_native": False,
            "network_egress": "disabled",
            "loopback_only": True,
        },
        "toolchain": {
            "rust_toolchain": "1.100.0",
            "llvm_version": "19.1.7",
            "node_version": "22.11.0",
            "python_version": "3.12.7",
            "c_compiler": "gcc-13.2.0",
            "linker": "ld-2.42",
            "ndk_version": "not_applicable",
            "xcode_version": "not_applicable",
            "require_pinned_versions": True,
            "cargo_lock_sha256": sha,
            "dependency_lock_digests": {
                "Cargo.lock": sha,
                "package-lock.json": "b" * 64,
            },
        },
        "machine": {
            "machine_id": "perf-linux-x64-01-node-a",
            "cpu_model": "AMD EPYC 7763",
            "cpu_stepping": "1",
            "physical_cores": 8,
            "smt_enabled": True,
            "smt_topology": {
                "threads_per_core": 2,
                "logical_cpus": 16,
                "core_affinity": {
                    "client_and_runtime_physical_cores": [0, 1],
                    "mock_physical_cores": [2],
                    "sampler_physical_cores": [3],
                    "smt_siblings_isolated": True,
                },
            },
            "microcode": "0xa0011d1",
            "ram_bytes": 32 * 1024 * 1024 * 1024,
            "kernel_version": "6.8.0-45-generic",
            "os_image_digest": f"sha256:{sha}",
            "cpu_governor": "performance",
            "turbo_policy": "disabled",
            "swap_active_bytes": 0,
            "build_target": "x86_64-unknown-linux-gnu",
            "rustflags": "",
        },
    }


def _make_clean_bundle_for_scenario(sc: dict, *, case: str | None = None, concurrency: int = 1) -> dict:
    budgets = sc["budgets"]
    runs = []
    for i in range(10):
        # Intentionally inject >5% variation across runs on non-P50 metrics to prove
        # the 3% MAD/median stability check does NOT falsely trigger on non-P50 scenarios.
        spread = 1.12 if i % 2 == 0 else 0.90
        metrics: dict[str, int | float] = {}
        for m_name in budgets:
            if m_name == "rss_slope_bytes_per_1k":
                continue
            elif m_name == "paired_delta_p50_ns":
                metrics["in_process_p50_ns"] = 800_000
                metrics["stdio_p50_ns"] = 1_000_000 + (40_000 if i % 2 == 0 else 0)
            elif m_name == "p50_ns":
                metrics[m_name] = 200_000 + (i % 2) * 500  # < 3% MAD CV
            elif m_name == "p95_ns":
                metrics[m_name] = int(600_000 * spread)
            elif m_name == "ttft_p95_ns":
                metrics[m_name] = int(1_000_000 * spread)
            elif m_name == "throughput_events_per_s":
                metrics[m_name] = 40_000.0 * spread
            elif m_name == "throughput_bytes_per_s":
                metrics[m_name] = 1_500_000.0 * spread
            elif m_name == "retained_rss_bytes":
                metrics[m_name] = 1_048_576 if i % 2 == 0 else 2_097_152
            elif m_name == "peak_rss_bytes":
                metrics[m_name] = int(80_000_000 * spread)
        if "idle_rss_max_bytes" in sc:
            metrics["idle_rss_bytes"] = 32 * 1024 * 1024
        if "shutdown_max_ms" in sc:
            metrics["shutdown_ms"] = 250
        if "max_queued_payload_bytes" in sc:
            metrics["max_queued_payload_bytes"] = 1_048_576
        entry: dict = {
            "pair_index": i,
            "samples": sc["min_samples"],
            "timed_out_operations": 0,
            "run_timed_out": False,
            "correctness_failures": 0,
            "leaked_handles": 0,
            "cpu_steal_pct": 0.1,
            "background_cpu_pct": 0.5,
            "metrics": metrics,
        }
        if sc.get("requires_b0_calibration"):
            entry["b0_drift_pct"] = 0.4
        if "rss_slope_bytes_per_1k" in budgets:
            entry["rss_windows_bytes"] = [50 * 1024 * 1024] * sc.get("memory_windows", 20)
        runs.append(entry)
    return _make_bundle(
        scenario=sc["id"],
        backend=sc["backends"][0],
        operation_timeout_ms=sc["operation_timeout_ms"],
        run_timeout_ms=sc["run_timeout_ms"],
        concurrency=concurrency,
        case=case,
        runs=runs,
    )


class ComparePerfTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.scenarios = compare_perf.load_json_strict(ROOT / "bench" / "scenarios.json")
        cls.environment = compare_perf.load_json_strict(ROOT / "bench" / "environment.json")

    def test_real_configs_and_fixtures_validate(self) -> None:
        compare_perf.validate_environment_config(self.environment)
        by_id = compare_perf.validate_scenarios_config(self.scenarios)
        self.assertEqual(len(by_id), 18)
        stdio_stream_cases = {c["id"]: c for c in by_id["stdio-stream"]["cases"]}
        self.assertEqual(set(stdio_stream_cases), {"10000x32", "1024x1024"})
        self.assertEqual(stdio_stream_cases["1024x1024"]["stream_events"], 1024)
        self.assertEqual(stdio_stream_cases["1024x1024"]["payload_bytes_per_event"], 1024)
        self.assertIn("paired_delta_p50_ns", by_id["stdio-incremental-cost"]["budgets"])
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

    def test_all_18_scenarios_pass_without_false_dispersion_failure(self) -> None:
        for sc in self.scenarios["scenarios"]:
            cases = [c["id"] for c in sc["cases"]] if "cases" in sc else [None]
            concurrencies = sc.get("concurrency_levels", [1])
            for case_id in cases:
                for conc in concurrencies:
                    base = _make_clean_bundle_for_scenario(sc, case=case_id, concurrency=conc)
                    head = copy.deepcopy(base)
                    report = compare_perf.evaluate_comparison(base, head, self.scenarios, seed=42)
                    self.assertEqual(
                        report["status"],
                        "PASS",
                        f"{sc['id']} case={case_id} conc={conc} failed: {report}",
                    )
                    self.assertNotIn("BENCH_ENV_UNSTABLE", report["failure_codes"])
                    if "p50_ns" not in sc["budgets"]:
                        self.assertIsNone(report["base_dispersion_pct"])

    def test_closed_schema_rejects_unknown_missing_float_bool_and_negative(self) -> None:
        # 1. Unknown key in budget spec (e.g., max_abs_ns_typo) must fail.
        bad_typo = copy.deepcopy(self.scenarios)
        bad_typo["scenarios"][0]["budgets"]["p50_ns"]["max_abs_ns_typo"] = 999999
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_typo)

        # 2. Missing required key in budget spec must fail.
        bad_missing_budget = copy.deepcopy(self.scenarios)
        del bad_missing_budget["scenarios"][0]["budgets"]["p50_ns"]["max_abs_ns"]
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_missing_budget)

        # 3. Unknown key on scenario entry & top-level & protocol must fail.
        bad_sc_key = copy.deepcopy(self.scenarios)
        bad_sc_key["scenarios"][0]["unexpected_field"] = 1
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_sc_key)

        bad_proto = copy.deepcopy(self.scenarios)
        del bad_proto["protocol"]["schedule"]
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_proto)

        # 4. Floats and booleans where strict integers are required must fail.
        for bad_val in (1000.5, 1000000.0, True, False, 0, -1):
            bad_int = copy.deepcopy(self.scenarios)
            bad_int["scenarios"][0]["budgets"]["p50_ns"]["max_abs_ns"] = bad_val
            with self.assertRaises(compare_perf.ConfigError, msg=f"failed to reject {bad_val!r}"):
                compare_perf.validate_scenarios_config(bad_int)

        for bad_schema_ver in (True, 1.0, 2, -1):
            bad_sv = copy.deepcopy(self.scenarios)
            bad_sv["schema_version"] = bad_schema_ver
            with self.assertRaises(compare_perf.ConfigError, msg=f"failed to reject schema_version={bad_schema_ver!r}"):
                compare_perf.validate_scenarios_config(bad_sv)

        # 5. Negative / bool min_abs_delta_ns and negative optional scenario counts must fail.
        for bad_delta in (-1, 5.5, True):
            bad_d = copy.deepcopy(self.scenarios)
            bad_d["scenarios"][0]["budgets"]["p50_ns"]["min_abs_delta_ns"] = bad_delta
            with self.assertRaises(compare_perf.ConfigError):
                compare_perf.validate_scenarios_config(bad_d)

        bad_req_bytes = copy.deepcopy(self.scenarios)
        bad_req_bytes["scenarios"][0]["request_bytes"] = -1024
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_req_bytes)

        # 6. Unknown metric key, float latency, bool sample count, negative cpu_steal in run bundle.
        base = _make_bundle()
        bad_run_typo = copy.deepcopy(base)
        bad_run_typo["runs"][0]["metrics"]["p50_ns_typo"] = 123
        rep_typo = compare_perf.evaluate_comparison(base, bad_run_typo, self.scenarios, seed=42)
        self.assertEqual(rep_typo["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_run_float = copy.deepcopy(base)
        bad_run_float["runs"][0]["metrics"]["p50_ns"] = 200_000.0
        rep_float = compare_perf.evaluate_comparison(base, bad_run_float, self.scenarios, seed=42)
        self.assertEqual(rep_float["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_run_bool = copy.deepcopy(base)
        bad_run_bool["runs"][0]["samples"] = True
        rep_bool = compare_perf.evaluate_comparison(base, bad_run_bool, self.scenarios, seed=42)
        self.assertEqual(rep_bool["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_run_steal = copy.deepcopy(base)
        bad_run_steal["runs"][0]["cpu_steal_pct"] = -0.1
        rep_steal = compare_perf.evaluate_comparison(base, bad_run_steal, self.scenarios, seed=42)
        self.assertEqual(rep_steal["failure_codes"], ["BENCH_CONFIG_INVALID"])

        # 7. retained_rss_bytes == 0 must fail BENCH_CONFIG_INVALID (never pass or ZeroDivisionError).
        cancel_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "cancel-and-drop")
        rss_zero_base = _make_clean_bundle_for_scenario(cancel_sc)
        rss_zero_head = copy.deepcopy(rss_zero_base)
        rss_zero_head["runs"][0]["metrics"]["retained_rss_bytes"] = 0
        rep_zero_head = compare_perf.evaluate_comparison(
            rss_zero_base, rss_zero_head, self.scenarios, seed=42
        )
        self.assertEqual(rep_zero_head["failure_codes"], ["BENCH_CONFIG_INVALID"])

        rss_zero_all_base = copy.deepcopy(rss_zero_base)
        for r in rss_zero_all_base["runs"]:
            r["metrics"]["retained_rss_bytes"] = 0
        rep_zero_base = compare_perf.evaluate_comparison(
            rss_zero_all_base, copy.deepcopy(rss_zero_base), self.scenarios, seed=42
        )
        self.assertEqual(rep_zero_base["failure_codes"], ["BENCH_CONFIG_INVALID"])

        # 8. Missing cpu_steal_pct, background_cpu_pct, or b0_drift_pct (on B0-calibrated scenario) must fail.
        for missing_env_field in ("cpu_steal_pct", "background_cpu_pct", "b0_drift_pct"):
            missing_env_head = copy.deepcopy(base)
            del missing_env_head["runs"][0][missing_env_field]
            rep_missing_env = compare_perf.evaluate_comparison(
                base, missing_env_head, self.scenarios, seed=42
            )
            self.assertEqual(
                rep_missing_env["failure_codes"],
                ["BENCH_CONFIG_INVALID"],
                f"missing {missing_env_field} was not rejected",
            )

        # 9. Short rss_windows_bytes (< memory_windows=20) must fail BENCH_CONFIG_INVALID.
        sustained_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "sustained")
        short_rss_base = _make_clean_bundle_for_scenario(sustained_sc, concurrency=1)
        short_rss_head = copy.deepcopy(short_rss_base)
        short_rss_head["runs"][0]["rss_windows_bytes"] = [50 * 1024 * 1024] * 15
        rep_short_rss = compare_perf.evaluate_comparison(
            short_rss_base, short_rss_head, self.scenarios, seed=42
        )
        self.assertEqual(rep_short_rss["failure_codes"], ["BENCH_CONFIG_INVALID"])

    def test_stdio_incremental_paired_delta_and_stdio_stream_cases(self) -> None:
        # 1kb case: total stdio_p50_ns=1.2ms > 0.5ms, but paired delta = 1.2ms - 0.9ms = 0.3ms <= 0.5ms -> PASS
        inc_1k_base = _make_bundle(
            scenario="stdio-incremental-cost",
            backend="stdio",
            case="1kb",
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.5,
                    "metrics": {"stdio_p50_ns": 1_200_000, "in_process_p50_ns": 900_000},
                }
                for i in range(10)
            ],
        )
        inc_1k_pass = copy.deepcopy(inc_1k_base)
        rep_1k_pass = compare_perf.evaluate_comparison(inc_1k_base, inc_1k_pass, self.scenarios, seed=42)
        self.assertEqual(rep_1k_pass["status"], "PASS", rep_1k_pass)
        self.assertEqual(rep_1k_pass["metrics"]["paired_delta_p50_ns"]["worst_delta_ns"], 300_000.0)

        # 1kb case exceeding 0.5ms paired delta -> PERF_GUARDRAIL_EXCEEDED
        inc_1k_fail = copy.deepcopy(inc_1k_base)
        inc_1k_fail["runs"][0]["metrics"]["stdio_p50_ns"] = 1_500_000  # delta = 600_000 > 500_000
        rep_1k_fail = compare_perf.evaluate_comparison(inc_1k_base, inc_1k_fail, self.scenarios, seed=42)
        self.assertEqual(rep_1k_fail["status"], "FAIL")
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_1k_fail["failure_codes"])

        # 200kb case: delta = 1.6ms <= 2ms PASS; 2.2ms > 2ms FAIL
        inc_200k_base = _make_bundle(
            scenario="stdio-incremental-cost",
            backend="stdio",
            case="200kb",
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.5,
                    "metrics": {"stdio_p50_ns": 4_600_000, "in_process_p50_ns": 3_000_000},
                }
                for i in range(10)
            ],
        )
        rep_200k_pass = compare_perf.evaluate_comparison(
            inc_200k_base, copy.deepcopy(inc_200k_base), self.scenarios, seed=42
        )
        self.assertEqual(rep_200k_pass["status"], "PASS", rep_200k_pass)

        inc_200k_fail = copy.deepcopy(inc_200k_base)
        inc_200k_fail["runs"][0]["metrics"]["stdio_p50_ns"] = 5_200_000  # delta = 2.2ms > 2ms
        rep_200k_fail = compare_perf.evaluate_comparison(inc_200k_base, inc_200k_fail, self.scenarios, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_200k_fail["failure_codes"])

        # stdio-stream requires valid case ('10000x32' or '1024x1024')
        stream_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-stream")
        stream_1024 = _make_clean_bundle_for_scenario(stream_sc, case="1024x1024")
        rep_stream = compare_perf.evaluate_comparison(
            stream_1024, copy.deepcopy(stream_1024), self.scenarios, seed=42
        )
        self.assertEqual(rep_stream["status"], "PASS", rep_stream)
        self.assertEqual(rep_stream["case"], "1024x1024")

        # §8 scenario-level caps (idle_rss_max_bytes, shutdown_max_ms, max_queued_payload_bytes) must be enforced.
        mem_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-memory")
        mem_base = _make_clean_bundle_for_scenario(mem_sc, concurrency=1)
        mem_fail = copy.deepcopy(mem_base)
        mem_fail["runs"][0]["metrics"]["idle_rss_bytes"] = 67_108_865
        rep_idle_fail = compare_perf.evaluate_comparison(mem_base, mem_fail, self.scenarios, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_idle_fail["failure_codes"])

        life_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-lifecycle")
        life_base = _make_clean_bundle_for_scenario(life_sc)
        life_fail = copy.deepcopy(life_base)
        life_fail["runs"][0]["metrics"]["shutdown_ms"] = 2001
        rep_life_fail = compare_perf.evaluate_comparison(life_base, life_fail, self.scenarios, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_life_fail["failure_codes"])

        bp_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-backpressure-cancel")
        bp_base = _make_clean_bundle_for_scenario(bp_sc)
        bp_fail = copy.deepcopy(bp_base)
        bp_fail["runs"][0]["metrics"]["max_queued_payload_bytes"] = 8_388_609
        rep_bp_fail = compare_perf.evaluate_comparison(bp_base, bp_fail, self.scenarios, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_bp_fail["failure_codes"])

        bad_stream_missing_case = copy.deepcopy(stream_1024)
        del bad_stream_missing_case["case"]
        rep_no_case = compare_perf.evaluate_comparison(
            bad_stream_missing_case, bad_stream_missing_case, self.scenarios, seed=42
        )
        self.assertEqual(rep_no_case["failure_codes"], ["BENCH_CONFIG_INVALID"])

    def test_active_environment_requires_all_rfc_7_1_attestation_fields(self) -> None:
        active = _make_active_environment()
        compare_perf.validate_environment_config(active)

        # Empty machine object must be rejected.
        empty_machine = copy.deepcopy(active)
        empty_machine["machine"] = {}
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_environment_config(empty_machine)

        # Missing any §7.1 machine field must be rejected.
        for field in (
            "machine_id",
            "cpu_model",
            "cpu_stepping",
            "physical_cores",
            "smt_topology",
            "microcode",
            "ram_bytes",
            "kernel_version",
            "os_image_digest",
            "cpu_governor",
            "turbo_policy",
            "swap_active_bytes",
            "build_target",
        ):
            missing_m = copy.deepcopy(active)
            del missing_m["machine"][field]
            with self.assertRaises(compare_perf.ConfigError, msg=f"missing machine.{field}"):
                compare_perf.validate_environment_config(missing_m)

        # Missing any §7.1 active toolchain field must be rejected.
        for tc_field in (
            "llvm_version",
            "node_version",
            "python_version",
            "c_compiler",
            "linker",
            "ndk_version",
            "xcode_version",
            "cargo_lock_sha256",
            "dependency_lock_digests",
        ):
            missing_tc = copy.deepcopy(active)
            missing_tc["toolchain"][tc_field] = None
            with self.assertRaises(compare_perf.ConfigError, msg=f"missing toolchain.{tc_field}"):
                compare_perf.validate_environment_config(missing_tc)

        # Floating / range / placeholder toolchain versions must be rejected across all lock fields.
        for bad_ver in ("stable", "latest", "^1.100.0", "~1.100.0", "1.100.x", "18.1.7 || 19.1.7"):
            floating_tc = copy.deepcopy(active)
            floating_tc["toolchain"]["rust_toolchain"] = bad_ver
            with self.assertRaises(compare_perf.ConfigError):
                compare_perf.validate_environment_config(floating_tc)

        for tc_field, bad_val in (
            ("llvm_version", "18.1.7 || 19.1.7"),
            ("llvm_version", "19.1"),
            ("c_compiler", "gcc-13.2.0 | gcc-14.0.0"),
            ("linker", "ld-2.42, 2.43"),
            ("c_compiler", "todo"),
        ):
            bad_tc = copy.deepcopy(active)
            bad_tc["toolchain"][tc_field] = bad_val
            with self.assertRaises(compare_perf.ConfigError, msg=f"{tc_field}={bad_val!r}"):
                compare_perf.validate_environment_config(bad_tc)

        bad_digest = copy.deepcopy(active)
        bad_digest["machine"]["os_image_digest"] = "todo"
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_environment_config(bad_digest)

        # Overlapping core affinity, swap > 0, target-cpu=native must be rejected.
        overlap_cores = copy.deepcopy(active)
        overlap_cores["machine"]["smt_topology"]["core_affinity"]["mock_physical_cores"] = [1]
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_environment_config(overlap_cores)

        active_swap = copy.deepcopy(active)
        active_swap["machine"]["swap_active_bytes"] = 4096
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_environment_config(active_swap)

        for bad_flags in ("-C target-cpu=native", "-C target-cpu = native", "-march=native"):
            native_flags = copy.deepcopy(active)
            native_flags["machine"]["rustflags"] = bad_flags
            with self.assertRaises(compare_perf.ConfigError, msg=bad_flags):
                compare_perf.validate_environment_config(native_flags)

        # Formal comparison with pending_dedicated_runner environment must fail BENCH_CONFIG_INVALID.
        base = _make_bundle()
        rep_pending = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, environment_data=self.environment, seed=42
        )
        self.assertEqual(rep_pending["failure_codes"], ["BENCH_CONFIG_INVALID"])

        rep_active = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, environment_data=active, seed=42
        )
        self.assertEqual(rep_active["status"], "PASS")

    def test_passes_clean_pairs_and_sub_floor_latency_delta(self) -> None:
        base = _make_bundle()
        head = copy.deepcopy(base)
        for r in head["runs"]:
            r["metrics"]["p50_ns"] = r["metrics"]["p50_ns"] + 3_500
        anchor = copy.deepcopy(base)
        report = compare_perf.evaluate_comparison(
            base, head, self.scenarios, anchor_raw=anchor, seed=42
        )
        self.assertEqual(report["status"], "PASS", report)
        self.assertEqual(report["failure_codes"], [])

        fast_base = _make_bundle(
            runs=[
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
                    "b0_drift_pct": 0.4,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.5,
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
        slow_base = _make_bundle(
            runs=[
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
                    "metrics": {"p50_ns": 1_500_000, "p95_ns": 1_800_000},
                }
                for i in range(10)
            ]
        )
        slow_head = copy.deepcopy(slow_base)
        report = compare_perf.evaluate_comparison(slow_base, slow_head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", report["failure_codes"])

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
                    "b0_drift_pct": 0.4,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.5,
                    "rss_windows_bytes": [50 * 1024 * 1024] * 20,
                    "metrics": {
                        "p50_ns": 2_000_000,
                        "p95_ns": 4_000_000,
                        "throughput_bytes_per_s": 10_000_000.0,
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

        # sustained must also gate latency & throughput per RFC-0043 §7.3.
        sustained_lat_fail = copy.deepcopy(rss_base)
        for r in sustained_lat_fail["runs"]:
            r["metrics"]["p50_ns"] = 6_000_000
        lat_rep = compare_perf.evaluate_comparison(
            rss_base, sustained_lat_fail, self.scenarios, seed=42
        )
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", lat_rep["failure_codes"])
        self.assertIn("PERF_REGRESSION", lat_rep["failure_codes"])

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
