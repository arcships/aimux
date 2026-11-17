#!/usr/bin/env python3
"""Compare RFC-0043 P-track benchmark runs against statistical & guardrail budgets.

Usage:
    python3 scripts/quality/compare-perf.py --validate-scenarios bench/scenarios.json [--environment bench/environment.json]
    python3 scripts/quality/compare-perf.py --base base.json --head head.json [--anchor anchor.json] [--scenarios bench/scenarios.json] [--out report.json]
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import random
import statistics
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
DEFAULT_SCENARIOS = ROOT / "bench" / "scenarios.json"
DEFAULT_ENVIRONMENT = ROOT / "bench" / "environment.json"

ALLOWED_PHASES = frozenset({"0.6", "0.8"})
ALLOWED_SCENARIO_STATUSES = frozenset({"active_contract", "future_stdio"})
ALLOWED_BUDGET_METRICS = frozenset(
    {
        "p50_ns",
        "p95_ns",
        "ttft_p95_ns",
        "throughput_events_per_s",
        "throughput_bytes_per_s",
        "retained_rss_bytes",
        "peak_rss_bytes",
        "rss_slope_bytes_per_1k",
    }
)
ALLOWED_TOP_LEVEL_RUN_KEYS = frozenset(
    {
        "schema_version",
        "scenario",
        "backend",
        "operation_timeout_ms",
        "run_timeout_ms",
        "recipe_hash",
        "concurrency",
        "source_sha",
        "runs",
    }
)
ALLOWED_RUN_ENTRY_KEYS = frozenset(
    {
        "pair_index",
        "samples",
        "timed_out_operations",
        "run_timed_out",
        "correctness_failures",
        "leaked_handles",
        "metrics",
        "rss_windows_bytes",
        "b0_drift_pct",
        "cpu_steal_pct",
        "background_cpu_pct",
    }
)


class ConfigError(ValueError):
    """Raised when benchmark scenario, environment, or run JSON violates schema contracts."""


def _is_strict_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _is_real_number(value: Any) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(float(value))
    )


def _no_duplicate_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    obj: dict[str, Any] = {}
    for key, val in pairs:
        if key in obj:
            raise ConfigError(f"duplicate JSON key: {key!r}")
        obj[key] = val
    return obj


def _reject_constant(constant: str) -> Any:
    raise ConfigError(f"invalid non-standard JSON constant: {constant!r}")


def load_json_strict(path: pathlib.Path) -> Any:
    """Parse JSON rejecting duplicate object keys, NaN, and Infinity."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ConfigError(f"cannot read {path.name}: {exc}") from exc
    try:
        return json.loads(
            text,
            object_pairs_hook=_no_duplicate_pairs,
            parse_constant=_reject_constant,
        )
    except ConfigError:
        raise
    except json.JSONDecodeError as exc:
        raise ConfigError(f"invalid JSON in {path.name}: {exc}") from exc


def validate_environment_config(data: Any) -> None:
    """Validate `bench/environment.json` per RFC-0043 §7.1."""
    if not isinstance(data, dict):
        raise ConfigError("environment config must be a JSON object")
    if data.get("schema_version") != 1:
        raise ConfigError("environment.schema_version must equal 1")
    pool_id = data.get("pool_id")
    if not isinstance(pool_id, str) or not pool_id.strip():
        raise ConfigError("environment.pool_id must be a non-empty string")
    status = data.get("status")
    if status not in ("pending_dedicated_runner", "active"):
        raise ConfigError(f"invalid environment.status: {status!r}")
    iso = data.get("isolation_requirements")
    if not isinstance(iso, dict):
        raise ConfigError("environment.isolation_requirements must be an object")
    if not _is_strict_int(iso.get("min_exclusive_physical_cores")) or iso["min_exclusive_physical_cores"] < 4:
        raise ConfigError("min_exclusive_physical_cores must be integer >= 4")
    if not _is_strict_int(iso.get("min_ram_gib")) or iso["min_ram_gib"] < 16:
        raise ConfigError("min_ram_gib must be integer >= 16")
    if not _is_strict_int(iso.get("tokio_worker_threads")) or iso["tokio_worker_threads"] != 2:
        raise ConfigError("tokio_worker_threads must equal integer 2")
    if iso.get("allow_target_cpu_native") is not False:
        raise ConfigError("allow_target_cpu_native must be false")
    if iso.get("network_egress") != "disabled":
        raise ConfigError("network_egress must be 'disabled'")
    toolchain = data.get("toolchain")
    if not isinstance(toolchain, dict) or not isinstance(toolchain.get("rust_toolchain"), str):
        raise ConfigError("environment.toolchain.rust_toolchain must be a version string")
    if status == "pending_dedicated_runner":
        if data.get("machine") is not None:
            raise ConfigError("pending_dedicated_runner must keep machine=null")
    else:
        if not isinstance(data.get("machine"), dict):
            raise ConfigError("active environment must provide machine attestation object")


def validate_scenarios_config(data: Any) -> dict[str, dict[str, Any]]:
    """Validate `bench/scenarios.json` and return a map keyed by scenario id."""
    if not isinstance(data, dict):
        raise ConfigError("scenarios config must be a JSON object")
    if data.get("schema_version") != 1:
        raise ConfigError("scenarios.schema_version must equal 1")
    protocol = data.get("protocol")
    if not isinstance(protocol, dict):
        raise ConfigError("scenarios.protocol must be an object")
    for int_key in (
        "independent_pairs",
        "max_rerun_pairs",
        "bootstrap_iterations",
        "default_seed",
        "rss_slope_tail_windows",
        "max_rss_slope_bytes_per_1k_requests",
    ):
        val = protocol.get(int_key)
        if not _is_strict_int(val) or val <= 0:
            raise ConfigError(f"protocol.{int_key} must be positive integer")
    if protocol["max_rerun_pairs"] < protocol["independent_pairs"]:
        raise ConfigError("protocol.max_rerun_pairs must be >= independent_pairs")
    for pct_key in ("bootstrap_confidence", "max_base_p50_mad_cv_pct", "max_b0_drift_pct"):
        if not _is_real_number(protocol.get(pct_key)) or protocol[pct_key] <= 0:
            raise ConfigError(f"protocol.{pct_key} must be positive number")

    scenarios_raw = data.get("scenarios")
    if not isinstance(scenarios_raw, list) or not scenarios_raw:
        raise ConfigError("scenarios.scenarios must be a non-empty list")

    by_id: dict[str, dict[str, Any]] = {}
    for entry in scenarios_raw:
        if not isinstance(entry, dict):
            raise ConfigError("each scenario entry must be an object")
        sid = entry.get("id")
        if not isinstance(sid, str) or not sid.strip():
            raise ConfigError("scenario.id must be a non-empty string")
        if sid in by_id:
            raise ConfigError(f"duplicate scenario.id: {sid!r}")
        if entry.get("phase") not in ALLOWED_PHASES:
            raise ConfigError(f"scenario {sid}: invalid phase {entry.get('phase')!r}")
        if entry.get("status") not in ALLOWED_SCENARIO_STATUSES:
            raise ConfigError(f"scenario {sid}: invalid status {entry.get('status')!r}")
        for timeout_key in ("operation_timeout_ms", "run_timeout_ms"):
            tval = entry.get(timeout_key)
            if not _is_strict_int(tval) or tval <= 0:
                raise ConfigError(
                    f"scenario {sid}: {timeout_key} must be positive integer, got {tval!r}"
                )
        backends = entry.get("backends")
        if not isinstance(backends, list) or not backends or not all(isinstance(b, str) and b for b in backends):
            raise ConfigError(f"scenario {sid}: backends must be non-empty string list")
        min_samples = entry.get("min_samples")
        if not _is_strict_int(min_samples) or min_samples <= 0:
            raise ConfigError(f"scenario {sid}: min_samples must be positive integer")
        budgets = entry.get("budgets")
        if not isinstance(budgets, dict):
            raise ConfigError(f"scenario {sid}: budgets must be an object")
        unknown_metrics = set(budgets) - ALLOWED_BUDGET_METRICS
        if unknown_metrics:
            raise ConfigError(f"scenario {sid}: unknown budget metrics {sorted(unknown_metrics)}")
        by_id[sid] = entry
    return by_id


def _validate_run_bundle(
    data: Any,
    label: str,
    scenarios_by_id: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any]]:
    if not isinstance(data, dict):
        raise ConfigError(f"{label}: run bundle must be a JSON object")
    unknown = set(data) - ALLOWED_TOP_LEVEL_RUN_KEYS
    if unknown:
        raise ConfigError(f"{label}: unknown keys {sorted(unknown)}")
    if data.get("schema_version") != 1:
        raise ConfigError(f"{label}: schema_version must equal 1")
    scenario_id = data.get("scenario")
    if not isinstance(scenario_id, str) or scenario_id not in scenarios_by_id:
        raise ConfigError(f"{label}: unknown scenario {scenario_id!r}")
    scenario = scenarios_by_id[scenario_id]

    backend = data.get("backend")
    if not isinstance(backend, str) or backend not in scenario["backends"]:
        raise ConfigError(
            f"{label}: backend {backend!r} not registered for scenario {scenario_id!r}"
        )

    for timeout_key in ("operation_timeout_ms", "run_timeout_ms"):
        tval = data.get(timeout_key)
        if not _is_strict_int(tval) or tval <= 0:
            raise ConfigError(f"{label}: {timeout_key} must be positive integer, got {tval!r}")
        if tval != scenario[timeout_key]:
            raise ConfigError(
                f"{label}: {timeout_key}={tval} diverges from registered scenario "
                f"value {scenario[timeout_key]}"
            )

    recipe_hash = data.get("recipe_hash")
    if not isinstance(recipe_hash, str) or not recipe_hash.strip():
        raise ConfigError(f"{label}: recipe_hash must be a non-empty string")

    concurrency = data.get("concurrency", 1)
    if not _is_strict_int(concurrency) or concurrency <= 0:
        raise ConfigError(f"{label}: concurrency must be positive integer, got {concurrency!r}")

    runs = data.get("runs")
    if not isinstance(runs, list) or not runs:
        raise ConfigError(f"{label}: runs must be a non-empty list")

    required_budgets = scenario["budgets"]
    slope_tail = scenario.get("slope_tail_windows")

    for idx, run in enumerate(runs):
        if not isinstance(run, dict):
            raise ConfigError(f"{label}: run[{idx}] must be an object")
        unknown_run = set(run) - ALLOWED_RUN_ENTRY_KEYS
        if unknown_run:
            raise ConfigError(f"{label}: run[{idx}] unknown keys {sorted(unknown_run)}")
        for int_field in (
            "samples",
            "timed_out_operations",
            "correctness_failures",
            "leaked_handles",
        ):
            val = run.get(int_field)
            if not _is_strict_int(val) or val < 0:
                raise ConfigError(
                    f"{label}: run[{idx}].{int_field} must be non-negative integer"
                )
        if not isinstance(run.get("run_timed_out"), bool):
            raise ConfigError(f"{label}: run[{idx}].run_timed_out must be boolean")

        metrics = run.get("metrics")
        if not isinstance(metrics, dict):
            raise ConfigError(f"{label}: run[{idx}].metrics must be an object")

        for metric_name in required_budgets:
            if metric_name == "rss_slope_bytes_per_1k":
                windows = run.get("rss_windows_bytes")
                if not isinstance(windows, list):
                    raise ConfigError(
                        f"{label}: run[{idx}] missing rss_windows_bytes for slope check"
                    )
                if not _is_strict_int(slope_tail) or len(windows) < slope_tail:
                    raise ConfigError(
                        f"{label}: run[{idx}].rss_windows_bytes length {len(windows)} "
                        f"< required tail {slope_tail}"
                    )
                for w in windows:
                    if not _is_strict_int(w) or w <= 0:
                        raise ConfigError(
                            f"{label}: run[{idx}].rss_windows_bytes must contain positive integers"
                        )
                continue

            if metric_name not in metrics:
                raise ConfigError(f"{label}: run[{idx}].metrics missing {metric_name!r}")
            mval = metrics[metric_name]
            if metric_name in ("throughput_events_per_s", "throughput_bytes_per_s"):
                if not _is_real_number(mval) or mval <= 0:
                    raise ConfigError(
                        f"{label}: run[{idx}].metrics.{metric_name} must be positive number"
                    )
            elif metric_name == "retained_rss_bytes":
                if not _is_strict_int(mval) or mval < 0:
                    raise ConfigError(
                        f"{label}: run[{idx}].metrics.{metric_name} must be non-negative integer"
                    )
            else:
                if not _is_strict_int(mval) or mval <= 0:
                    raise ConfigError(
                        f"{label}: run[{idx}].metrics.{metric_name} must be positive integer"
                    )

        for opt_float in ("b0_drift_pct", "cpu_steal_pct", "background_cpu_pct"):
            if opt_float in run and not _is_real_number(run[opt_float]):
                raise ConfigError(f"{label}: run[{idx}].{opt_float} must be finite number")

    normalized = dict(data)
    normalized["concurrency"] = concurrency
    return normalized, scenario


def compute_theil_sen_slope(values: list[int]) -> float:
    """Compute the Theil-Sen median pairwise slope over an equally spaced series."""
    n = len(values)
    if n < 2:
        raise ConfigError("Theil-Sen slope requires at least 2 points")
    slopes: list[float] = []
    for i in range(n):
        for j in range(i + 1, n):
            slopes.append((values[j] - values[i]) / float(j - i))
    return float(statistics.median(slopes))


def _quantile(sorted_vals: list[float], q: float) -> float:
    if not sorted_vals:
        raise ValueError("empty sample")
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    pos = q * (len(sorted_vals) - 1)
    lo = int(math.floor(pos))
    hi = int(math.ceil(pos))
    frac = pos - lo
    return sorted_vals[lo] * (1.0 - frac) + sorted_vals[hi] * frac


def _robust_mad_cv_pct(values: list[float]) -> float:
    med = float(statistics.median(values))
    if med <= 0.0:
        return float("inf")
    abs_devs = [abs(v - med) for v in values]
    mad = float(statistics.median(abs_devs))
    return (1.4826 * mad / med) * 100.0


def _paired_bootstrap(
    ref_vals: list[float],
    head_vals: list[float],
    *,
    higher_is_worse: bool,
    iterations: int,
    confidence: float,
    rng: random.Random,
) -> tuple[float, float, float, float, float, float]:
    """Return `(point_ratio, ci_low_ratio, ci_high_ratio, point_abs, ci_low_abs, ci_high_abs)`."""
    n = len(ref_vals)
    ref_mean = sum(ref_vals) / n
    head_mean = sum(head_vals) / n
    if higher_is_worse:
        point_abs = head_mean - ref_mean
        point_ratio = point_abs / ref_mean
    else:
        point_abs = ref_mean - head_mean
        point_ratio = point_abs / ref_mean

    ratios: list[float] = []
    abs_deltas: list[float] = []
    indices = range(n)
    for _ in range(iterations):
        sample_idx = rng.choices(indices, k=n)
        s_ref = sum(ref_vals[i] for i in sample_idx) / n
        s_head = sum(head_vals[i] for i in sample_idx) / n
        if higher_is_worse:
            d_abs = s_head - s_ref
            d_ratio = d_abs / s_ref if s_ref > 0 else 0.0
        else:
            d_abs = s_ref - s_head
            d_ratio = d_abs / s_ref if s_ref > 0 else 0.0
        ratios.append(d_ratio)
        abs_deltas.append(d_abs)

    ratios.sort()
    abs_deltas.sort()
    alpha = (1.0 - confidence) / 2.0
    return (
        point_ratio,
        _quantile(ratios, alpha),
        _quantile(ratios, 1.0 - alpha),
        point_abs,
        _quantile(abs_deltas, alpha),
        _quantile(abs_deltas, 1.0 - alpha),
    )


def _peak_rss_guardrail_bytes(spec: dict[str, Any], concurrency: int) -> int | None:
    by_conc = spec.get("max_abs_bytes_by_concurrency")
    if isinstance(by_conc, dict) and str(concurrency) in by_conc:
        return int(by_conc[str(concurrency)])
    val = spec.get("max_abs_bytes")
    return int(val) if val is not None else None


def evaluate_comparison(
    base_raw: Any,
    head_raw: Any,
    scenarios_data: Any,
    *,
    anchor_raw: Any | None = None,
    seed: int | None = None,
) -> dict[str, Any]:
    """Run all RFC-0043 §7 checks and return a structured result dictionary."""
    failure_codes: list[str] = []

    def add_failure(code: str) -> None:
        if code not in failure_codes:
            failure_codes.append(code)

    try:
        scenarios_by_id = validate_scenarios_config(scenarios_data)
        protocol = scenarios_data["protocol"]
        base, scenario = _validate_run_bundle(base_raw, "base", scenarios_by_id)
        head, _ = _validate_run_bundle(head_raw, "head", scenarios_by_id)
        anchor = None
        if anchor_raw is not None:
            anchor, _ = _validate_run_bundle(anchor_raw, "anchor", scenarios_by_id)
    except ConfigError as exc:
        return {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": ["BENCH_CONFIG_INVALID"],
            "errors": [str(exc)],
            "metrics": {},
        }

    errors: list[str] = []

    def check_matching_config(a: dict[str, Any], b: dict[str, Any], b_name: str) -> None:
        for field in ("scenario", "backend", "concurrency", "operation_timeout_ms", "run_timeout_ms"):
            if a[field] != b[field]:
                add_failure("BENCH_CONFIG_INVALID")
                errors.append(f"{b_name}.{field}={b[field]!r} does not match base.{field}={a[field]!r}")
        if b_name == "head" and a["recipe_hash"] != b["recipe_hash"]:
            add_failure("BENCH_CONFIG_INVALID")
            errors.append(f"head.recipe_hash={b['recipe_hash']!r} differs from base.recipe_hash={a['recipe_hash']!r}")

    check_matching_config(base, head, "head")
    if anchor is not None:
        check_matching_config(base, anchor, "anchor")

    allowed_counts = (protocol["independent_pairs"], protocol["max_rerun_pairs"])
    if len(base["runs"]) not in allowed_counts or len(base["runs"]) != len(head["runs"]):
        add_failure("BENCH_CONFIG_INVALID")
        errors.append(
            f"base ({len(base['runs'])}) and head ({len(head['runs'])}) must have equal paired "
            f"run counts in {allowed_counts}"
        )
    if anchor is not None and len(anchor["runs"]) != len(head["runs"]):
        add_failure("BENCH_CONFIG_INVALID")
        errors.append(
            f"anchor run count ({len(anchor['runs'])}) must match head ({len(head['runs'])}) "
            "per RFC-0043 §7.2"
        )

    if failure_codes:
        return {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": failure_codes,
            "errors": errors,
            "scenario": base["scenario"],
            "backend": base["backend"],
            "metrics": {},
        }

    all_bundles = [("base", base), ("head", head)]
    if anchor is not None:
        all_bundles.append(("anchor", anchor))

    # 1. Timeout and correctness gates across every run.
    min_samples = scenario["min_samples"]
    for b_name, bundle in all_bundles:
        for idx, run in enumerate(bundle["runs"]):
            if run["run_timed_out"] or run["timed_out_operations"] > 0:
                add_failure("BENCH_TIMEOUT")
                errors.append(f"{b_name}.runs[{idx}] timed out")
            if run["correctness_failures"] > 0 or run["leaked_handles"] > 0:
                add_failure("BENCH_CORRECTNESS_FAILURE")
                errors.append(
                    f"{b_name}.runs[{idx}] correctness={run['correctness_failures']} "
                    f"leaked_handles={run['leaked_handles']}"
                )
            if run["samples"] < min_samples:
                add_failure("BENCH_CORRECTNESS_FAILURE")
                errors.append(
                    f"{b_name}.runs[{idx}].samples={run['samples']} < required min_samples={min_samples}"
                )

    # 2. Environment stability: B0 drift, CPU steal / background, base robust MAD CV <= 3%.
    for b_name, bundle in all_bundles:
        for idx, run in enumerate(bundle["runs"]):
            if "b0_drift_pct" in run and abs(run["b0_drift_pct"]) > protocol["max_b0_drift_pct"]:
                add_failure("BENCH_ENV_UNSTABLE")
                errors.append(f"{b_name}.runs[{idx}] B0 drift {run['b0_drift_pct']}% exceeds limit")
            if "cpu_steal_pct" in run and run["cpu_steal_pct"] > 1.0:
                add_failure("BENCH_ENV_UNSTABLE")
                errors.append(f"{b_name}.runs[{idx}] cpu_steal_pct={run['cpu_steal_pct']}% > 1.0%")
            if "background_cpu_pct" in run and run["background_cpu_pct"] > 5.0:
                add_failure("BENCH_ENV_UNSTABLE")
                errors.append(
                    f"{b_name}.runs[{idx}] background_cpu_pct={run['background_cpu_pct']}% > 5.0%"
                )

    budgets = scenario["budgets"]
    primary_dispersion_metric = (
        "p50_ns"
        if "p50_ns" in budgets
        else next((m for m in budgets if m != "rss_slope_bytes_per_1k"), None)
    )
    base_mad_cv_pct: float | None = None
    if primary_dispersion_metric is not None:
        base_series = [float(r["metrics"][primary_dispersion_metric]) for r in base["runs"]]
        base_mad_cv_pct = _robust_mad_cv_pct(base_series)
        if base_mad_cv_pct > protocol["max_base_p50_mad_cv_pct"]:
            add_failure("BENCH_ENV_UNSTABLE")
            errors.append(
                f"base {primary_dispersion_metric} robust MAD/median dispersion "
                f"{base_mad_cv_pct:.2f}% > {protocol['max_base_p50_mad_cv_pct']}%"
            )

    if any(code in failure_codes for code in ("BENCH_TIMEOUT", "BENCH_CORRECTNESS_FAILURE")):
        return {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": failure_codes,
            "errors": errors,
            "scenario": head["scenario"],
            "backend": head["backend"],
            "base_dispersion_pct": base_mad_cv_pct,
            "metrics": {},
        }

    # 3. Statistical paired bootstrap and absolute guardrails on head runs.
    rng = random.Random(protocol["default_seed"] if seed is None else seed)
    iterations = protocol["bootstrap_iterations"]
    confidence = protocol["bootstrap_confidence"]
    metric_reports: dict[str, Any] = {}

    for metric_name, spec in budgets.items():
        if metric_name == "rss_slope_bytes_per_1k":
            tail_n = protocol["rss_slope_tail_windows"]
            slope_limit = float(spec["max_abs_bytes_per_1k"])
            head_slopes = [
                compute_theil_sen_slope(r["rss_windows_bytes"][-tail_n:]) for r in head["runs"]
            ]
            worst_slope = max(head_slopes)
            passed = worst_slope <= slope_limit
            if not passed:
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(
                    f"head rss_slope_bytes_per_1k worst={worst_slope:.1f} > limit={slope_limit:.1f}"
                )
            metric_reports[metric_name] = {
                "status": "PASS" if passed else "FAIL",
                "worst_slope_bytes_per_1k": worst_slope,
                "limit_bytes_per_1k": slope_limit,
            }
            continue

        head_vals = [float(r["metrics"][metric_name]) for r in head["runs"]]
        base_vals = [float(r["metrics"][metric_name]) for r in base["runs"]]
        higher_is_worse = metric_name not in ("throughput_events_per_s", "throughput_bytes_per_s")

        # Absolute product guardrails apply per run (RFC-0043 §7.4).
        guardrail_exceeded = False
        if metric_name in ("p50_ns", "p95_ns", "ttft_p95_ns"):
            cap = spec.get("max_abs_ns")
            if cap is not None and max(head_vals) > float(cap):
                guardrail_exceeded = True
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(f"head {metric_name} max={max(head_vals):.0f} > guardrail={cap}")
        elif metric_name == "retained_rss_bytes":
            cap = spec.get("max_abs_bytes")
            if cap is not None and max(head_vals) > float(cap):
                guardrail_exceeded = True
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(f"head {metric_name} max={max(head_vals):.0f} > guardrail={cap}")
        elif metric_name == "peak_rss_bytes":
            cap = _peak_rss_guardrail_bytes(spec, head["concurrency"])
            if cap is not None and max(head_vals) > float(cap):
                guardrail_exceeded = True
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(f"head {metric_name} max={max(head_vals):.0f} > guardrail={cap}")
        elif not higher_is_worse and anchor is not None:
            anchor_vals = [float(r["metrics"][metric_name]) for r in anchor["runs"]]
            floor_tp = (sum(anchor_vals) / len(anchor_vals)) * protocol["anchor_throughput_floor_ratio"]
            if min(head_vals) < floor_tp:
                guardrail_exceeded = True
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(
                    f"head {metric_name} min={min(head_vals):.1f} < 90% of anchor={floor_tp:.1f}"
                )

        if higher_is_worse:
            limit_ratio = float(spec["max_regression_pct"]) / 100.0
            floor_abs = float(
                spec.get("min_abs_delta_ns", spec.get("min_abs_delta_bytes", 0))
            )
        else:
            limit_ratio = float(spec["max_drop_pct"]) / 100.0
            floor_abs = 0.0

        refs: list[tuple[str, list[float]]] = [("base", base_vals)]
        if anchor is not None:
            refs.append(("anchor", [float(r["metrics"][metric_name]) for r in anchor["runs"]]))

        comp_details: dict[str, Any] = {}
        metric_verdicts: list[str] = []
        for ref_name, ref_vals in refs:
            pt_r, lo_r, hi_r, pt_a, lo_a, hi_a = _paired_bootstrap(
                ref_vals,
                head_vals,
                higher_is_worse=higher_is_worse,
                iterations=iterations,
                confidence=confidence,
                rng=rng,
            )
            if higher_is_worse:
                within_floor = floor_abs > 0.0 and hi_a <= floor_abs
                passes_rel = hi_r <= limit_ratio or within_floor
                regressed = (
                    pt_r > limit_ratio
                    and lo_r > limit_ratio
                    and (floor_abs == 0.0 or (pt_a > floor_abs and lo_a > floor_abs))
                )
            else:
                passes_rel = hi_r <= limit_ratio
                regressed = pt_r > limit_ratio and lo_r > limit_ratio

            if regressed:
                verdict = "PERF_REGRESSION"
                add_failure("PERF_REGRESSION")
                errors.append(
                    f"head vs {ref_name} {metric_name}: change={pt_r * 100:.2f}% "
                    f"CI99=[{lo_r * 100:.2f}%, {hi_r * 100:.2f}%] exceeds limit {limit_ratio * 100:.1f}%"
                )
            elif passes_rel:
                verdict = "PASS"
            else:
                verdict = "PERF_INCONCLUSIVE"
                add_failure("PERF_INCONCLUSIVE")
                errors.append(
                    f"head vs {ref_name} {metric_name}: CI99=[{lo_r * 100:.2f}%, {hi_r * 100:.2f}%] "
                    f"straddles budget {limit_ratio * 100:.1f}% (inconclusive does not pass)"
                )
            metric_verdicts.append(verdict)
            comp_details[f"vs_{ref_name}"] = {
                "verdict": verdict,
                "point_change_pct": pt_r * 100.0,
                "ci99_change_pct": [lo_r * 100.0, hi_r * 100.0],
                "point_abs_delta": pt_a,
                "ci99_abs_delta": [lo_a, hi_a],
            }

        metric_reports[metric_name] = {
            "guardrail_exceeded": guardrail_exceeded,
            "comparisons": comp_details,
            "verdict": (
                "FAIL"
                if guardrail_exceeded or any(v != "PASS" for v in metric_verdicts)
                else "PASS"
            ),
        }

    overall = "PASS" if not failure_codes else "FAIL"
    return {
        "schema_version": 1,
        "status": overall,
        "failure_codes": failure_codes,
        "errors": errors,
        "scenario": head["scenario"],
        "backend": head["backend"],
        "pair_count": len(head["runs"]),
        "base_dispersion_pct": base_mad_cv_pct,
        "metrics": metric_reports,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate and compare RFC-0043 §7 benchmarks.")
    parser.add_argument("--scenarios", type=pathlib.Path, default=DEFAULT_SCENARIOS)
    parser.add_argument("--environment", type=pathlib.Path)
    parser.add_argument("--validate-scenarios", type=pathlib.Path)
    parser.add_argument("--base", type=pathlib.Path)
    parser.add_argument("--head", type=pathlib.Path)
    parser.add_argument("--anchor", type=pathlib.Path)
    parser.add_argument("--seed", type=int)
    parser.add_argument("--out", type=pathlib.Path)
    args = parser.parse_args(argv)

    if args.validate_scenarios is not None:
        try:
            scenarios = load_json_strict(args.validate_scenarios)
            by_id = validate_scenarios_config(scenarios)
            env_path = args.environment or DEFAULT_ENVIRONMENT
            if env_path.is_file():
                validate_environment_config(load_json_strict(env_path))
        except ConfigError as exc:
            print(f"BENCH_CONFIG_INVALID: {exc}", file=sys.stderr)
            return 1
        print(f"perf scenarios ok ({len(by_id)} scenarios validated)")
        return 0

    if args.base is None or args.head is None:
        parser.error("--base and --head are required unless --validate-scenarios is given")

    try:
        scenarios_data = load_json_strict(args.scenarios)
        base_raw = load_json_strict(args.base)
        head_raw = load_json_strict(args.head)
        anchor_raw = load_json_strict(args.anchor) if args.anchor is not None else None
    except ConfigError as exc:
        report = {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": ["BENCH_CONFIG_INVALID"],
            "errors": [str(exc)],
            "metrics": {},
        }
        if args.out is not None:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
        return 1

    report = evaluate_comparison(
        base_raw,
        head_raw,
        scenarios_data,
        anchor_raw=anchor_raw,
        seed=args.seed,
    )
    rendered = json.dumps(report, indent=2) + "\n"
    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
