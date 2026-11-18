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
    concurrency: int | None = 1,
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
    bundle: dict = {
        "schema_version": 1,
        "scenario": scenario,
        "backend": backend,
        "operation_timeout_ms": operation_timeout_ms,
        "run_timeout_ms": run_timeout_ms,
        "recipe_hash": recipe_hash,
        "runs": runs,
    }
    if concurrency is not None:
        bundle["concurrency"] = concurrency
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


def _make_clean_bundle_for_scenario(
    sc: dict,
    *,
    case: str | None = None,
    concurrency: int | None = None,
) -> dict:
    if concurrency is None:
        concurrency = sc["concurrency_levels"][0] if "concurrency_levels" in sc else 1
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
                metrics["paired_delta_p50_ns"] = 200_000 + (40_000 if i % 2 == 0 else 0)
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
                metrics[m_name] = 0 if i % 2 == 0 else 1_048_576
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
        if "min_measure_ms" in sc:
            entry["measure_ms"] = sc["min_measure_ms"]
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
        cls.fixtures_manifest = compare_perf.load_json_strict(
            ROOT / "bench" / "fixtures" / "manifest.json"
        )

    def test_real_configs_and_fixtures_validate(self) -> None:
        compare_perf.validate_environment_config(self.environment)
        by_id = compare_perf.validate_scenarios_config(self.scenarios)
        self.assertEqual(len(by_id), 18)
        compare_perf.validate_scenarios_against_fixtures(by_id, self.fixtures_manifest)
        stdio_stream_cases = {c["id"]: c for c in by_id["stdio-stream"]["cases"]}
        self.assertEqual(set(stdio_stream_cases), {"10000x32", "1024x1024"})
        self.assertEqual(stdio_stream_cases["1024x1024"]["stream_events"], 1024)
        self.assertEqual(stdio_stream_cases["1024x1024"]["payload_bytes_per_event"], 1024)
        self.assertIn("paired_delta_p50_ns", by_id["stdio-incremental-cost"]["budgets"])
        self.assertEqual(by_id["stdio-backpressure-cancel"]["concurrency_levels"], [100])
        errors = gen_fixtures.check_manifest(ROOT / "bench" / "fixtures" / "manifest.json")
        self.assertEqual(errors, [])

    def test_fixture_generator_exact_byte_sizes_and_manifest_strictness(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp) / "out"
            code = gen_fixtures.main(["--out-dir", str(out_dir)])
            self.assertEqual(code, 0)
            for spec in gen_fixtures.FIXTURE_SPECS:
                raw = (out_dir / f"{spec['id']}.json").read_bytes()
                self.assertEqual(len(raw), spec["exact_bytes"], spec["id"])
                json.loads(raw.decode("utf-8"))

            # Reject bool schema_version, float exact_bytes, duplicate keys, non-dict root, and bad --manifest with --out-dir.
            valid_manifest = gen_fixtures.compute_manifest()
            bad_bool_m = copy.deepcopy(valid_manifest)
            bad_bool_m["schema_version"] = True
            m_path = pathlib.Path(tmp) / "bad_manifest.json"
            m_path.write_text(json.dumps(bad_bool_m), encoding="utf-8")
            self.assertNotEqual(gen_fixtures.check_manifest(m_path), [])

            bad_float_m = copy.deepcopy(valid_manifest)
            bad_float_m["fixtures"][0]["exact_bytes"] = 1024.0
            m_path.write_text(json.dumps(bad_float_m), encoding="utf-8")
            self.assertNotEqual(gen_fixtures.check_manifest(m_path), [])

            m_path.write_text(
                '{"schema_version": 2, "schema_version": 1, "fixtures": []}', encoding="utf-8"
            )
            self.assertNotEqual(gen_fixtures.check_manifest(m_path), [])

            for malformed in ("[]", '{"schema_version": 1, "fixtures": null}', "{}"):
                m_path.write_text(malformed, encoding="utf-8")
                self.assertNotEqual(gen_fixtures.check_manifest(m_path), [])
                err = io.StringIO()
                with redirect_stderr(err):
                    rc = gen_fixtures.main(["--out-dir", str(out_dir), "--manifest", str(m_path)])
                self.assertEqual(rc, 1)

        # Drift between scenarios.json request_bytes and fixtures manifest must fail.
        bad_sc = copy.deepcopy(self.scenarios)
        bad_sc["scenarios"][0]["request_bytes"] = 1025
        by_id = compare_perf.validate_scenarios_config(bad_sc)
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_against_fixtures(by_id, self.fixtures_manifest)

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
                    json.dumps(report, allow_nan=False)
                    self.assertNotIn("BENCH_ENV_UNSTABLE", report["failure_codes"])
                    if "p50_ns" not in sc["budgets"]:
                        self.assertIsNone(report["base_dispersion_pct"])

    def test_closed_schema_rejects_unknown_missing_float_bool_negative_and_overflow(self) -> None:
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

        # 3. Unknown key on scenario entry & missing protocol field must fail.
        bad_sc_key = copy.deepcopy(self.scenarios)
        bad_sc_key["scenarios"][0]["unexpected_field"] = 1
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_sc_key)

        bad_proto = copy.deepcopy(self.scenarios)
        del bad_proto["protocol"]["schedule"]
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_proto)

        # 4. Floats, booleans, 0, negative, and >int64 overflow must fail.
        for bad_val in (1000.5, 1000000.0, True, False, 0, -1, 1 << 64):
            bad_int = copy.deepcopy(self.scenarios)
            bad_int["scenarios"][0]["budgets"]["p50_ns"]["max_abs_ns"] = bad_val
            with self.assertRaises(compare_perf.ConfigError, msg=f"failed to reject {bad_val!r}"):
                compare_perf.validate_scenarios_config(bad_int)

        # 5. Unhashable phase, Unicode superscript digit concurrency key, slope_tail_windows=1.
        bad_phase = copy.deepcopy(self.scenarios)
        bad_phase["scenarios"][0]["phase"] = []
        rep_phase = compare_perf.evaluate_comparison(_make_bundle(), _make_bundle(), bad_phase)
        self.assertEqual(rep_phase["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_conc_key = copy.deepcopy(self.scenarios)
        sustained_idx = next(
            i for i, s in enumerate(bad_conc_key["scenarios"]) if s["id"] == "sustained"
        )
        bad_conc_key["scenarios"][sustained_idx]["budgets"]["peak_rss_bytes"][
            "max_abs_bytes_by_concurrency"
        ] = {"²": 268435456}
        rep_conc_key = compare_perf.evaluate_comparison(_make_bundle(), _make_bundle(), bad_conc_key)
        self.assertEqual(rep_conc_key["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_tail1 = copy.deepcopy(self.scenarios)
        bad_tail1["scenarios"][sustained_idx]["slope_tail_windows"] = 1
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_tail1)

        # 6. Unknown metric, float latency, bool sample count, negative cpu_steal, inverted quantiles, watchdog violation.
        base = _make_bundle()
        for mutator in (
            lambda b: b["runs"][0]["metrics"].update({"p50_ns_typo": 123}),
            lambda b: b["runs"][0]["metrics"].update({"p50_ns": 200_000.0}),
            lambda b: b["runs"][0].update({"samples": True}),
            lambda b: b["runs"][0].update({"cpu_steal_pct": -0.1}),
            lambda b: b["runs"][0]["metrics"].update({"p50_ns": 500_000, "p95_ns": 400_000}),
            lambda b: b["runs"][0]["metrics"].update({"p99_ns": 6_000_000_000}),
        ):
            bad_b = copy.deepcopy(base)
            mutator(bad_b)
            rep = compare_perf.evaluate_comparison(base, bad_b, self.scenarios, seed=42)
            self.assertEqual(rep["failure_codes"], ["BENCH_CONFIG_INVALID"], rep)

        # 7. Missing cpu_steal_pct, background_cpu_pct, or b0_drift_pct must fail.
        for missing_env_field in ("cpu_steal_pct", "background_cpu_pct", "b0_drift_pct"):
            missing_env_head = copy.deepcopy(base)
            del missing_env_head["runs"][0][missing_env_field]
            rep_missing_env = compare_perf.evaluate_comparison(
                base, missing_env_head, self.scenarios, seed=42
            )
            self.assertEqual(rep_missing_env["failure_codes"], ["BENCH_CONFIG_INVALID"])

        # 8. Extreme float overflow/underflow in throughput must fail cleanly with valid JSON.
        bin_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-binary")
        bin_base = _make_clean_bundle_for_scenario(bin_sc)
        for extreme_tp in (10**400, 1e308, 1e-320):
            bad_tp = copy.deepcopy(bin_base)
            bad_tp["runs"][0]["metrics"]["throughput_bytes_per_s"] = extreme_tp
            rep_ext = compare_perf.evaluate_comparison(bin_base, bad_tp, self.scenarios, seed=42)
            self.assertEqual(rep_ext["failure_codes"], ["BENCH_CONFIG_INVALID"])
            json.dumps(rep_ext, allow_nan=False)

        # 9. Explicit null anchor or null environment or anchor recipe_hash mismatch must fail.
        rep_null_anchor = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, anchor_raw=None, seed=42
        )
        self.assertEqual(rep_null_anchor["failure_codes"], ["BENCH_CONFIG_INVALID"])

        rep_null_env = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, environment_data=None, seed=42
        )
        self.assertEqual(rep_null_env["failure_codes"], ["BENCH_CONFIG_INVALID"])

        bad_anchor_recipe = copy.deepcopy(base)
        bad_anchor_recipe["recipe_hash"] = "other-recipe"
        rep_anchor_recipe = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, anchor_raw=bad_anchor_recipe, seed=42
        )
        self.assertEqual(rep_anchor_recipe["failure_codes"], ["BENCH_CONFIG_INVALID"])

    def test_zero_retained_rss_growth_passes_and_leak_from_zero_fails_valid_json(self) -> None:
        cancel_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "cancel-and-drop")
        zero_base = _make_clean_bundle_for_scenario(cancel_sc)
        for r in zero_base["runs"]:
            r["metrics"]["retained_rss_bytes"] = 0
        zero_head = copy.deepcopy(zero_base)
        rep_pass = compare_perf.evaluate_comparison(zero_base, zero_head, self.scenarios, seed=42)
        self.assertEqual(rep_pass["status"], "PASS", rep_pass)
        json.dumps(rep_pass, allow_nan=False)

        # +1 MiB from 0 is within the 2 MiB floor -> PASS.
        sub_floor_head = copy.deepcopy(zero_base)
        for r in sub_floor_head["runs"]:
            r["metrics"]["retained_rss_bytes"] = 1_048_576
        rep_sub = compare_perf.evaluate_comparison(
            zero_base, sub_floor_head, self.scenarios, seed=42
        )
        self.assertEqual(rep_sub["status"], "PASS", rep_sub)
        json.dumps(rep_sub, allow_nan=False)

        # +3 MiB from 0 exceeds max(10%, 2 MiB) -> PERF_REGRESSION with valid JSON (no Infinity/NaN).
        leak_head = copy.deepcopy(zero_base)
        for r in leak_head["runs"]:
            r["metrics"]["retained_rss_bytes"] = 3 * 1024 * 1024
        rep_leak = compare_perf.evaluate_comparison(zero_base, leak_head, self.scenarios, seed=42)
        self.assertEqual(rep_leak["status"], "FAIL")
        self.assertIn("PERF_REGRESSION", rep_leak["failure_codes"])
        json.dumps(rep_leak, allow_nan=False)

    def test_stdio_incremental_paired_delta_and_stdio_stream_and_backpressure(self) -> None:
        # Bimodal op distribution where marginal P50 delta is 100us, but true per-op paired_delta_p50_ns is 600us (> 500us cap) -> FAIL!
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
                    "metrics": {
                        "paired_delta_p50_ns": 300_000,
                        "stdio_p50_ns": 900_000,
                        "in_process_p50_ns": 800_000,
                    },
                }
                for i in range(10)
            ],
        )
        rep_1k_pass = compare_perf.evaluate_comparison(
            inc_1k_base, copy.deepcopy(inc_1k_base), self.scenarios, seed=42
        )
        self.assertEqual(rep_1k_pass["status"], "PASS", rep_1k_pass)

        inc_1k_bimodal_fail = copy.deepcopy(inc_1k_base)
        inc_1k_bimodal_fail["runs"][0]["metrics"]["paired_delta_p50_ns"] = 600_000
        rep_1k_fail = compare_perf.evaluate_comparison(
            inc_1k_base, inc_1k_bimodal_fail, self.scenarios, seed=42
        )
        self.assertEqual(rep_1k_fail["status"], "FAIL")
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_1k_fail["failure_codes"])

        # Self-referential reference_backend == 'stdio' must be rejected.
        bad_ref = copy.deepcopy(self.scenarios)
        inc_idx = next(
            i for i, s in enumerate(bad_ref["scenarios"]) if s["id"] == "stdio-incremental-cost"
        )
        bad_ref["scenarios"][inc_idx]["reference_backend"] = "stdio"
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(bad_ref)

        # stdio-backpressure-cancel requires concurrency=100; concurrency=1 or omitted must fail.
        bp_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-backpressure-cancel")
        bp_100 = _make_clean_bundle_for_scenario(bp_sc, concurrency=100)
        self.assertEqual(
            compare_perf.evaluate_comparison(bp_100, copy.deepcopy(bp_100), self.scenarios, seed=42)[
                "status"
            ],
            "PASS",
        )
        bp_c1 = copy.deepcopy(bp_100)
        bp_c1["concurrency"] = 1
        self.assertEqual(
            compare_perf.evaluate_comparison(bp_c1, bp_c1, self.scenarios, seed=42)["failure_codes"],
            ["BENCH_CONFIG_INVALID"],
        )
        bp_no_conc = copy.deepcopy(bp_100)
        del bp_no_conc["concurrency"]
        self.assertEqual(
            compare_perf.evaluate_comparison(bp_no_conc, bp_no_conc, self.scenarios, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )

        # §8 scenario-level caps (idle_rss_max_bytes, shutdown_max_ms, max_queued_payload_bytes) enforced.
        mem_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-memory")
        mem_base = _make_clean_bundle_for_scenario(mem_sc, concurrency=1)
        mem_fail = copy.deepcopy(mem_base)
        mem_fail["runs"][0]["metrics"]["idle_rss_bytes"] = 67_108_865
        self.assertIn(
            "PERF_GUARDRAIL_EXCEEDED",
            compare_perf.evaluate_comparison(mem_base, mem_fail, self.scenarios, seed=42)[
                "failure_codes"
            ],
        )

        sc_life_cap = copy.deepcopy(self.scenarios)
        life_idx = next(i for i, s in enumerate(sc_life_cap["scenarios"]) if s["id"] == "stdio-lifecycle")
        sc_life_cap["scenarios"][life_idx]["operation_timeout_ms"] = 3000
        life_sc = sc_life_cap["scenarios"][life_idx]
        life_base = _make_clean_bundle_for_scenario(life_sc)
        life_fail = copy.deepcopy(life_base)
        life_fail["runs"][0]["metrics"]["shutdown_ms"] = 2001
        self.assertIn(
            "PERF_GUARDRAIL_EXCEEDED",
            compare_perf.evaluate_comparison(life_base, life_fail, sc_life_cap, seed=42)[
                "failure_codes"
            ],
        )

        bp_fail = copy.deepcopy(bp_100)
        bp_fail["runs"][0]["metrics"]["max_queued_payload_bytes"] = 8_388_609
        self.assertIn(
            "PERF_GUARDRAIL_EXCEEDED",
            compare_perf.evaluate_comparison(bp_100, bp_fail, self.scenarios, seed=42)[
                "failure_codes"
            ],
        )

    def test_active_environment_attestation_and_custom_isolation_caps(self) -> None:
        active = _make_active_environment()
        compare_perf.validate_environment_config(active)

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

        for bad_flags in ("-C target-cpu=native", "-C target-cpu = native", "-march=native"):
            native_flags = copy.deepcopy(active)
            native_flags["machine"]["rustflags"] = bad_flags
            with self.assertRaises(compare_perf.ConfigError, msg=bad_flags):
                compare_perf.validate_environment_config(native_flags)

        # Tighter environment max_cpu_steal_pct=0.1 must fail a run with cpu_steal_pct=0.5.
        strict_env = copy.deepcopy(active)
        strict_env["isolation_requirements"]["max_cpu_steal_pct"] = 0.1
        base = _make_bundle()
        base["runs"][0]["cpu_steal_pct"] = 0.5
        rep_steal = compare_perf.evaluate_comparison(
            base, copy.deepcopy(base), self.scenarios, environment_data=strict_env, seed=42
        )
        self.assertEqual(rep_steal["status"], "FAIL")
        self.assertIn("BENCH_ENV_UNSTABLE", rep_steal["failure_codes"])

    def test_bootstrap_boundary_precision_and_joint_floor_and_registered_tp_floor(self) -> None:
        # Exact +5.0% across alternating 200000 / 200020 ns must PASS (no IEEE-754 false inconclusive).
        exact_base = _make_bundle(
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "b0_drift_pct": 0.2,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.4,
                    "metrics": {
                        "p50_ns": 200_000 if i % 2 == 0 else 200_020,
                        "p95_ns": 400_000,
                    },
                }
                for i in range(10)
            ]
        )
        exact_head = copy.deepcopy(exact_base)
        for i, r in enumerate(exact_head["runs"]):
            r["metrics"]["p50_ns"] = 210_000 if i % 2 == 0 else 210_021
        rep_exact = compare_perf.evaluate_comparison(exact_base, exact_head, self.scenarios, seed=42)
        self.assertEqual(rep_exact["status"], "PASS", rep_exact)

        # 7 pairs within 5us floor (50k -> 54.9k) + 3 pairs within 5% (150k -> 155.1k) must PASS.
        # Use a custom scenario without p50_ns dispersion failure (or test via request-small with p95_ns).
        mixed_base = _make_bundle(
            runs=[
                {
                    "pair_index": i,
                    "samples": 10000,
                    "timed_out_operations": 0,
                    "run_timed_out": False,
                    "correctness_failures": 0,
                    "leaked_handles": 0,
                    "b0_drift_pct": 0.2,
                    "cpu_steal_pct": 0.1,
                    "background_cpu_pct": 0.4,
                    "metrics": {
                        "p50_ns": 50_000,
                        "p95_ns": 80_000 if i < 7 else 300_000,
                    },
                }
                for i in range(10)
            ]
        )
        mixed_head = copy.deepcopy(mixed_base)
        for i, r in enumerate(mixed_head["runs"]):
            # p95_ns budget is 10% OR 10,000 ns floor: +9,800 ns on 80k (+12.25%) and +10,200 ns on 300k (+3.4%).
            r["metrics"]["p95_ns"] = 89_800 if i < 7 else 310_200
        rep_mixed = compare_perf.evaluate_comparison(mixed_base, mixed_head, self.scenarios, seed=42)
        self.assertEqual(rep_mixed["status"], "PASS", rep_mixed)

        # Registered integer throughput floor min_abs_per_s catches slow batch even if anchor dropped.
        sc_with_floor = copy.deepcopy(self.scenarios)
        bin_idx = next(i for i, s in enumerate(sc_with_floor["scenarios"]) if s["id"] == "stdio-binary")
        sc_with_floor["scenarios"][bin_idx]["budgets"]["throughput_bytes_per_s"]["min_abs_per_s"] = 90_000
        bin_sc = sc_with_floor["scenarios"][bin_idx]
        low_tp = _make_clean_bundle_for_scenario(bin_sc)
        for r in low_tp["runs"]:
            r["metrics"]["throughput_bytes_per_s"] = 89_000.0
        anchor_tp = copy.deepcopy(low_tp)
        for r in anchor_tp["runs"]:
            r["metrics"]["throughput_bytes_per_s"] = 90_000.0
        rep_floor = compare_perf.evaluate_comparison(
            low_tp, copy.deepcopy(low_tp), sc_with_floor, anchor_raw=anchor_tp, seed=42
        )
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_floor["failure_codes"])

        # min_abs_per_s: null must be rejected, and 10**18 - 1 < 10**18 must fail exact int comparison.
        sc_null_floor = copy.deepcopy(self.scenarios)
        sc_null_floor["scenarios"][bin_idx]["budgets"]["throughput_bytes_per_s"]["min_abs_per_s"] = None
        with self.assertRaises(compare_perf.ConfigError):
            compare_perf.validate_scenarios_config(sc_null_floor)

        sc_huge_floor = copy.deepcopy(self.scenarios)
        sc_huge_floor["scenarios"][bin_idx]["budgets"]["throughput_bytes_per_s"]["min_abs_per_s"] = 10**18
        huge_tp = _make_clean_bundle_for_scenario(sc_huge_floor["scenarios"][bin_idx])
        for r in huge_tp["runs"]:
            r["metrics"]["throughput_bytes_per_s"] = 10**18 - 1
        rep_huge = compare_perf.evaluate_comparison(
            huge_tp, copy.deepcopy(huge_tp), sc_huge_floor, seed=42
        )
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_huge["failure_codes"])

        # Small positive throughput regression (1.1e-9 -> 1.0445e-9 = -5.045%) must NOT be hidden by tol.
        tiny_base = _make_clean_bundle_for_scenario(bin_sc)
        for r in tiny_base["runs"]:
            r["metrics"]["throughput_bytes_per_s"] = 1.1e-9
        tiny_head = copy.deepcopy(tiny_base)
        for r in tiny_head["runs"]:
            r["metrics"]["throughput_bytes_per_s"] = 1.0445e-9
        rep_tiny = compare_perf.evaluate_comparison(tiny_base, tiny_head, self.scenarios, seed=42)
        self.assertIn("PERF_REGRESSION", rep_tiny["failure_codes"])

        # 9 pairs at 1.1e-9 -> 1.0445e-9 + 1 unchanged pair at 1000 must NOT PASS (scale-invariant per-resample tol).
        outlier_base = _make_clean_bundle_for_scenario(bin_sc)
        for i, r in enumerate(outlier_base["runs"]):
            r["metrics"]["throughput_bytes_per_s"] = 1000.0 if i == 0 else 1.1e-9
        outlier_head = copy.deepcopy(outlier_base)
        for i, r in enumerate(outlier_head["runs"]):
            r["metrics"]["throughput_bytes_per_s"] = 1000.0 if i == 0 else 1.0445e-9
        rep_outlier = compare_perf.evaluate_comparison(
            outlier_base, outlier_head, self.scenarios, seed=42
        )
        self.assertEqual(rep_outlier["status"], "FAIL", rep_outlier)
        self.assertIn("PERF_INCONCLUSIVE", rep_outlier["failure_codes"])

    def test_watchdog_paired_delta_required_exact_slope_and_parser_limits(self) -> None:
        # 1. measure_ms / warmup_ms > run_timeout_ms and stdio_p50_ns > operation_timeout_ms must fail.
        base = _make_bundle()
        for mutator in (
            lambda b: b["runs"][0].update({"measure_ms": 600_001}),
            lambda b: b["runs"][0].update({"warmup_ms": 600_001}),
            lambda b: b["runs"][0].update({"warmup_ms": 400_000, "measure_ms": 200_001}),
        ):
            bad_w = copy.deepcopy(base)
            mutator(bad_w)
            rep_w = compare_perf.evaluate_comparison(base, bad_w, self.scenarios, seed=42)
            self.assertEqual(rep_w["failure_codes"], ["BENCH_CONFIG_INVALID"])

        inc_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stdio-incremental-cost")
        inc_base = _make_clean_bundle_for_scenario(inc_sc, case="1kb")
        # stdio_p50_ns or abs(paired_delta_p50_ns) exceeding 5s operation_timeout_ms without timeout flag must fail.
        bad_stdio_lat = copy.deepcopy(inc_base)
        bad_stdio_lat["runs"][0]["metrics"]["stdio_p50_ns"] = 5_000_000_001
        self.assertEqual(
            compare_perf.evaluate_comparison(inc_base, bad_stdio_lat, self.scenarios, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )
        bad_neg_delta = copy.deepcopy(inc_base)
        bad_neg_delta["runs"][0]["metrics"]["paired_delta_p50_ns"] = -5_000_000_001
        self.assertEqual(
            compare_perf.evaluate_comparison(inc_base, bad_neg_delta, self.scenarios, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )

        # shutdown_ms exceeding operation_timeout_ms without timeout flag must fail.
        sc_short_life = copy.deepcopy(self.scenarios)
        life_idx = next(i for i, s in enumerate(sc_short_life["scenarios"]) if s["id"] == "stdio-lifecycle")
        sc_short_life["scenarios"][life_idx]["operation_timeout_ms"] = 1000
        life_b = _make_clean_bundle_for_scenario(sc_short_life["scenarios"][life_idx])
        life_over = copy.deepcopy(life_b)
        life_over["runs"][0]["metrics"]["shutdown_ms"] = 1001
        self.assertEqual(
            compare_perf.evaluate_comparison(life_b, life_over, sc_short_life, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )

        # Omitting measure_ms on streaming scenarios with min_measure_ms must fail BENCH_CONFIG_INVALID.
        stream_sc = next(s for s in self.scenarios["scenarios"] if s["id"] == "stream-small-chunks")
        stream_b = _make_clean_bundle_for_scenario(stream_sc)
        stream_no_ms = copy.deepcopy(stream_b)
        del stream_no_ms["runs"][0]["measure_ms"]
        self.assertEqual(
            compare_perf.evaluate_comparison(stream_b, stream_no_ms, self.scenarios, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )

        # 4301-digit concurrency key must return BENCH_CONFIG_INVALID without ValueError traceback.
        sc_huge_key = copy.deepcopy(self.scenarios)
        sus_k_idx = next(i for i, s in enumerate(sc_huge_key["scenarios"]) if s["id"] == "sustained")
        sc_huge_key["scenarios"][sus_k_idx]["budgets"]["peak_rss_bytes"][
            "max_abs_bytes_by_concurrency"
        ] = {"9" * 4301: 268435456}
        rep_huge_key = compare_perf.evaluate_comparison(base, copy.deepcopy(base), sc_huge_key, seed=42)
        self.assertEqual(rep_huge_key["failure_codes"], ["BENCH_CONFIG_INVALID"])

        # 2. Omitting paired_delta_p50_ns and supplying only marginal medians must be rejected.
        missing_paired = copy.deepcopy(inc_base)
        for r in missing_paired["runs"]:
            del r["metrics"]["paired_delta_p50_ns"]
        self.assertEqual(
            compare_perf.evaluate_comparison(inc_base, missing_paired, self.scenarios, seed=42)[
                "failure_codes"
            ],
            ["BENCH_CONFIG_INVALID"],
        )

        # 3. Exact rational Theil-Sen slope: 960 req/window with +480,000 B/window = exact 500,000 B/1k -> PASS cap 500,000.
        sc_exact_slope = copy.deepcopy(self.scenarios)
        sus_idx = next(i for i, s in enumerate(sc_exact_slope["scenarios"]) if s["id"] == "sustained")
        sc_exact_slope["scenarios"][sus_idx]["window_requests"] = 960
        sc_exact_slope["scenarios"][sus_idx]["budgets"]["rss_slope_bytes_per_1k"][
            "max_abs_bytes_per_1k"
        ] = 500_000
        ex_base = _make_clean_bundle_for_scenario(sc_exact_slope["scenarios"][sus_idx], concurrency=1)
        ex_head = copy.deepcopy(ex_base)
        ex_head["runs"][0]["rss_windows_bytes"] = [50 * 1024 * 1024 + w * 480_000 for w in range(20)]
        rep_ex = compare_perf.evaluate_comparison(ex_base, ex_head, sc_exact_slope, seed=42)
        self.assertEqual(rep_ex["status"], "PASS", rep_ex)

        # 4. Missing fixtures manifest, deeply nested JSON, and >4300-digit int must fail cleanly (no traceback).
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            err = io.StringIO()
            with redirect_stderr(err):
                rc_nom = compare_perf.main(
                    [
                        "--validate-scenarios",
                        str(ROOT / "bench" / "scenarios.json"),
                        "--environment",
                        str(ROOT / "bench" / "environment.json"),
                        "--fixtures-manifest",
                        str(d / "missing_manifest.json"),
                    ]
                )
            self.assertEqual(rc_nom, 1)

            deep_p = d / "deep.json"
            deep_p.write_text("[" * 2000 + "]" * 2000, encoding="utf-8")
            self.assertNotEqual(gen_fixtures.check_manifest(deep_p), [])

            big_int_p = d / "big_int.json"
            big_int_p.write_text('{"schema_version": ' + ("9" * 4500) + "}", encoding="utf-8")
            base_p = d / "base.json"
            out_p = d / "out.json"
            base_p.write_text(json.dumps(base), encoding="utf-8")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc_big = compare_perf.main(
                    ["--base", str(base_p), "--head", str(big_int_p), "--out", str(out_p)]
                )
            self.assertEqual(rc_big, 1)
            self.assertEqual(
                json.loads(out_p.read_text(encoding="utf-8"))["failure_codes"],
                ["BENCH_CONFIG_INVALID"],
            )

    def test_theil_sen_window_scaling_and_scenario_tail_slice(self) -> None:
        # 40 windows of 500 requests (+400kB/window = +800kB/1k requests > 524,288B/1k) must FAIL.
        sc_500 = copy.deepcopy(self.scenarios)
        sus_idx = next(i for i, s in enumerate(sc_500["scenarios"]) if s["id"] == "sustained")
        sc_500["scenarios"][sus_idx]["memory_windows"] = 40
        sc_500["scenarios"][sus_idx]["window_requests"] = 500
        sus_base = _make_clean_bundle_for_scenario(sc_500["scenarios"][sus_idx], concurrency=1)
        sus_head = copy.deepcopy(sus_base)
        sus_head["runs"][0]["rss_windows_bytes"] = [50 * 1024 * 1024 + w * 400_000 for w in range(40)]
        rep_500 = compare_perf.evaluate_comparison(sus_base, sus_head, sc_500, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_500["failure_codes"])
        self.assertAlmostEqual(
            rep_500["metrics"]["rss_slope_bytes_per_1k"]["worst_slope_bytes_per_1k"], 800_000.0
        )

        # Scenario slope_tail_windows=2 must slice last 2 windows rather than protocol's 15.
        sc_tail2 = copy.deepcopy(self.scenarios)
        sc_tail2["scenarios"][sus_idx]["slope_tail_windows"] = 2
        t2_base = _make_clean_bundle_for_scenario(sc_tail2["scenarios"][sus_idx], concurrency=1)
        t2_head = copy.deepcopy(t2_base)
        series = [50 * 1024 * 1024] * 19 + [50 * 1024 * 1024 + 1_000_000]
        t2_head["runs"][0]["rss_windows_bytes"] = series
        rep_t2 = compare_perf.evaluate_comparison(t2_base, t2_head, sc_tail2, seed=42)
        self.assertIn("PERF_GUARDRAIL_EXCEEDED", rep_t2["failure_codes"])

    def test_timeout_partial_run_and_correctness_failure_codes(self) -> None:
        base = _make_bundle()

        # Partial/empty metrics on a timed-out run must report BENCH_TIMEOUT, not BENCH_CONFIG_INVALID.
        to_head = copy.deepcopy(base)
        to_head["runs"][0] = {
            "pair_index": 0,
            "samples": 0,
            "timed_out_operations": 0,
            "run_timed_out": True,
            "correctness_failures": 0,
            "leaked_handles": 0,
            "cpu_steal_pct": 0.1,
            "background_cpu_pct": 0.5,
            "metrics": {},
        }
        rep_to = compare_perf.evaluate_comparison(base, to_head, self.scenarios, seed=42)
        self.assertEqual(rep_to["failure_codes"], ["BENCH_TIMEOUT"], rep_to)

        # Leaked handles and insufficient warmup/measure_ms must report RFC-0043 §9.4 CORRECTNESS_FAILURE.
        leak_head = copy.deepcopy(base)
        leak_head["runs"][0]["leaked_handles"] = 1
        rep_leak = compare_perf.evaluate_comparison(base, leak_head, self.scenarios, seed=42)
        self.assertIn("CORRECTNESS_FAILURE", rep_leak["failure_codes"])

        short_warmup = copy.deepcopy(base)
        short_warmup["runs"][0]["warmup_ms"] = 100
        rep_warm = compare_perf.evaluate_comparison(base, short_warmup, self.scenarios, seed=42)
        self.assertIn("CORRECTNESS_FAILURE", rep_warm["failure_codes"])

    def test_detects_latency_throughput_and_inconclusive_regressions(self) -> None:
        base = _make_bundle()
        head = copy.deepcopy(base)
        for r in head["runs"]:
            r["metrics"]["p50_ns"] = int(r["metrics"]["p50_ns"] * 1.15)
        report = compare_perf.evaluate_comparison(base, head, self.scenarios, seed=42)
        self.assertEqual(report["status"], "FAIL")
        self.assertIn("PERF_REGRESSION", report["failure_codes"])

        # Inconclusive straddling CI never passes.
        inc_head = copy.deepcopy(base)
        for i, r in enumerate(inc_head["runs"]):
            r["metrics"]["p50_ns"] = int(r["metrics"]["p50_ns"] * (1.09 if i % 2 == 0 else 1.005))
        inc_rep = compare_perf.evaluate_comparison(base, inc_head, self.scenarios, seed=42)
        self.assertEqual(inc_rep["status"], "FAIL")
        self.assertIn("PERF_INCONCLUSIVE", inc_rep["failure_codes"])

    def test_cli_validate_missing_env_and_invalid_utf8(self) -> None:
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

        # Explicit nonexistent --environment must fail non-zero.
        err2 = io.StringIO()
        with redirect_stderr(err2):
            rc_missing = compare_perf.main(
                [
                    "--validate-scenarios",
                    str(ROOT / "bench" / "scenarios.json"),
                    "--environment",
                    "/nonexistent/environment.json",
                ]
            )
        self.assertEqual(rc_missing, 1)

        # Invalid UTF-8 run bundle writes structured BENCH_CONFIG_INVALID JSON report.
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            base_p = d / "base.json"
            bad_p = d / "head.json"
            out_p = d / "report.json"
            base_p.write_text(json.dumps(_make_bundle()), encoding="utf-8")
            bad_p.write_bytes(b"\xff\xfeinvalid-utf8")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc_bad = compare_perf.main(
                    ["--base", str(base_p), "--head", str(bad_p), "--out", str(out_p)]
                )
            self.assertEqual(rc_bad, 1)
            self.assertEqual(
                json.loads(out_p.read_text(encoding="utf-8"))["failure_codes"],
                ["BENCH_CONFIG_INVALID"],
            )


if __name__ == "__main__":
    unittest.main()
