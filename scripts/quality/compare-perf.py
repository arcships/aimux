#!/usr/bin/env python3
"""Compare RFC-0043 P-track benchmark runs against statistical & guardrail budgets.

Usage:
    python3 scripts/quality/compare-perf.py --validate-scenarios bench/scenarios.json [--environment bench/environment.json]
    python3 scripts/quality/compare-perf.py --base base.json --head head.json [--anchor anchor.json] [--scenarios bench/scenarios.json] [--environment bench/environment.json] [--out report.json]
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import random
import re
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
        "paired_delta_p50_ns",
    }
)
ALLOWED_ENVIRONMENT_TOP_KEYS = frozenset(
    {
        "schema_version",
        "pool_id",
        "status",
        "isolation_requirements",
        "toolchain",
        "machine",
    }
)
ALLOWED_ISOLATION_KEYS = frozenset(
    {
        "min_exclusive_physical_cores",
        "core_allocation",
        "min_ram_gib",
        "tokio_worker_threads",
        "max_cpu_steal_pct",
        "max_background_cpu_pct",
        "allow_target_cpu_native",
        "network_egress",
        "loopback_only",
    }
)
ALLOWED_CORE_ALLOC_KEYS = frozenset(
    {
        "client_and_runtime",
        "mock",
        "sampler",
        "smt_siblings_isolated",
    }
)
ALLOWED_TOOLCHAIN_KEYS = frozenset(
    {
        "rust_toolchain",
        "llvm_version",
        "node_version",
        "python_version",
        "c_compiler",
        "linker",
        "ndk_version",
        "xcode_version",
        "require_pinned_versions",
        "cargo_lock_sha256",
        "dependency_lock_digests",
    }
)
ALLOWED_MACHINE_KEYS = frozenset(
    {
        "machine_id",
        "cpu_model",
        "cpu_stepping",
        "physical_cores",
        "smt_enabled",
        "smt_topology",
        "microcode",
        "ram_bytes",
        "kernel_version",
        "os_image_digest",
        "cpu_governor",
        "turbo_policy",
        "swap_active_bytes",
        "build_target",
        "rustflags",
    }
)
ALLOWED_SMT_TOPOLOGY_KEYS = frozenset(
    {
        "threads_per_core",
        "logical_cpus",
        "core_affinity",
    }
)
ALLOWED_CORE_AFFINITY_KEYS = frozenset(
    {
        "client_and_runtime_physical_cores",
        "mock_physical_cores",
        "sampler_physical_cores",
        "smt_siblings_isolated",
    }
)
ALLOWED_SCENARIOS_TOP_KEYS = frozenset({"schema_version", "protocol", "scenarios"})
ALLOWED_PROTOCOL_KEYS = frozenset(
    {
        "independent_pairs",
        "max_rerun_pairs",
        "schedule",
        "bootstrap_iterations",
        "bootstrap_confidence",
        "default_seed",
        "max_base_p50_mad_cv_pct",
        "max_b0_drift_pct",
        "rss_slope_tail_windows",
        "max_rss_slope_bytes_per_1k_requests",
        "anchor_throughput_floor_ratio",
    }
)
REQUIRED_SCENARIO_KEYS = frozenset(
    {
        "id",
        "phase",
        "status",
        "operation_timeout_ms",
        "run_timeout_ms",
        "backends",
        "min_samples",
        "budgets",
    }
)
POSITIVE_INT_SCENARIO_FIELDS = (
    "operation_timeout_ms",
    "run_timeout_ms",
    "min_samples",
    "min_warmup_requests",
    "min_warmup_ms",
    "min_warmup_streams",
    "min_measure_ms",
    "request_bytes",
    "response_bytes",
    "stream_events",
    "payload_bytes_per_event",
    "tool_schemas",
    "memory_windows",
    "window_requests",
    "slope_tail_windows",
    "frame_bytes",
    "frame_count",
    "max_queued_payload_bytes",
    "shutdown_max_ms",
    "idle_rss_max_bytes",
)
ALLOWED_SCENARIO_KEYS = (
    REQUIRED_SCENARIO_KEYS
    | frozenset(POSITIVE_INT_SCENARIO_FIELDS)
    | frozenset(
        {
            "concurrency_levels",
            "cases",
            "reference_backend",
            "comparison_mode",
            "requires_b0_calibration",
        }
    )
)
ALLOWED_CASE_KEYS = frozenset(
    {"id", "request_bytes", "response_bytes", "stream_events", "payload_bytes_per_event"}
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
        "case",
        "source_sha",
        "runs",
    }
)
REQUIRED_TOP_LEVEL_RUN_KEYS = frozenset(
    {
        "schema_version",
        "scenario",
        "backend",
        "operation_timeout_ms",
        "run_timeout_ms",
        "recipe_hash",
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
REQUIRED_RUN_ENTRY_KEYS = frozenset(
    {
        "pair_index",
        "samples",
        "timed_out_operations",
        "run_timed_out",
        "correctness_failures",
        "leaked_handles",
        "metrics",
        "cpu_steal_pct",
        "background_cpu_pct",
    }
)

_SHA256_HEX_RE = re.compile(r"^[0-9a-f]{64}$")
_DIGEST_RE = re.compile(r"^(?:sha256:)?[0-9a-f]{64}$")
_SEMVER_PINNED_RE = re.compile(r"^\d+\.\d+\.\d+(?:[+-][0-9A-Za-z.-]+)?$")
_FLOATING_VERSION_TOKENS = frozenset(
    {
        "stable",
        "latest",
        "nightly",
        "beta",
        "default",
        "system",
        "auto",
        "current",
        "lts",
        "or",
        "and",
        "any",
        "none",
        "tbd",
        "todo",
        "unknown",
    }
)
_GIB = 1024 * 1024 * 1024


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


def _require_pos_int(value: Any, ctx: str) -> int:
    if not _is_strict_int(value) or value <= 0:
        raise ConfigError(f"{ctx} must be positive integer, got {value!r}")
    return int(value)


def _require_non_neg_int(value: Any, ctx: str) -> int:
    if not _is_strict_int(value) or value < 0:
        raise ConfigError(f"{ctx} must be non-negative integer, got {value!r}")
    return int(value)


def _require_pos_number(value: Any, ctx: str) -> float:
    if not _is_real_number(value) or float(value) <= 0.0:
        raise ConfigError(f"{ctx} must be positive finite number, got {value!r}")
    return float(value)


def _require_non_neg_number(value: Any, ctx: str) -> float:
    if not _is_real_number(value) or float(value) < 0.0:
        raise ConfigError(f"{ctx} must be non-negative finite number, got {value!r}")
    return float(value)


def _require_non_empty_str(value: Any, ctx: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ConfigError(f"{ctx} must be a non-empty string, got {value!r}")
    return value


def _check_keys(
    obj: dict[str, Any],
    *,
    required: frozenset[str],
    allowed: frozenset[str],
    ctx: str,
) -> None:
    keys = set(obj)
    unknown = keys - allowed
    if unknown:
        raise ConfigError(f"{ctx}: unknown keys {sorted(unknown)}")
    missing = required - keys
    if missing:
        raise ConfigError(f"{ctx}: missing required keys {sorted(missing)}")


def _validate_pinned_version(value: Any, ctx: str, *, require_semver: bool = False) -> str:
    s = _require_non_empty_str(value, ctx).strip()
    lower_tokens = set(re.split(r"[^a-zA-Z0-9]+", s.lower()))
    if lower_tokens & _FLOATING_VERSION_TOKENS:
        raise ConfigError(f"{ctx} must be an exact pinned version, not {value!r}")
    if any(ch in s for ch in ("^", "~", "*", "<", ">", "=", "|", ",", ";", "&", "(", ")", "[", "]", "{", "}")) or re.search(
        r"(?:^|\.)x(?:\.|$)", s, re.IGNORECASE
    ):
        raise ConfigError(f"{ctx} must be an exact pinned version without range operators, got {value!r}")
    if require_semver and not _SEMVER_PINNED_RE.match(s):
        raise ConfigError(f"{ctx} must be exact X.Y.Z version string, got {value!r}")
    version_tokens = re.findall(r"\d+\.\d+(?:\.\d+)?", s)
    if len(version_tokens) != 1:
        raise ConfigError(f"{ctx} must specify exactly one concrete version number, got {value!r}")
    return s


def _validate_sha256_hex(value: Any, ctx: str) -> str:
    if not isinstance(value, str) or not _SHA256_HEX_RE.match(value):
        raise ConfigError(f"{ctx} must be 64-character lowercase hex sha256, got {value!r}")
    return value


def _validate_os_image_digest(value: Any, ctx: str) -> str:
    if not isinstance(value, str) or not _DIGEST_RE.match(value):
        raise ConfigError(f"{ctx} must be sha256:<64-hex> or 64-hex digest, got {value!r}")
    return value


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
    _check_keys(
        data,
        required=ALLOWED_ENVIRONMENT_TOP_KEYS,
        allowed=ALLOWED_ENVIRONMENT_TOP_KEYS,
        ctx="environment",
    )
    if _require_pos_int(data["schema_version"], "environment.schema_version") != 1:
        raise ConfigError("environment.schema_version must equal 1")
    _require_non_empty_str(data["pool_id"], "environment.pool_id")

    status = data["status"]
    if status not in ("pending_dedicated_runner", "active"):
        raise ConfigError(f"invalid environment.status: {status!r}")

    iso = data["isolation_requirements"]
    if not isinstance(iso, dict):
        raise ConfigError("environment.isolation_requirements must be an object")
    _check_keys(
        iso,
        required=ALLOWED_ISOLATION_KEYS,
        allowed=ALLOWED_ISOLATION_KEYS,
        ctx="environment.isolation_requirements",
    )
    min_cores = _require_pos_int(
        iso["min_exclusive_physical_cores"], "isolation_requirements.min_exclusive_physical_cores"
    )
    if min_cores < 4:
        raise ConfigError("min_exclusive_physical_cores must be integer >= 4")
    min_ram_gib = _require_pos_int(iso["min_ram_gib"], "isolation_requirements.min_ram_gib")
    if min_ram_gib < 16:
        raise ConfigError("min_ram_gib must be integer >= 16")
    if _require_pos_int(iso["tokio_worker_threads"], "isolation_requirements.tokio_worker_threads") != 2:
        raise ConfigError("tokio_worker_threads must equal integer 2")
    steal_cap = _require_pos_number(iso["max_cpu_steal_pct"], "isolation_requirements.max_cpu_steal_pct")
    if steal_cap > 1.0:
        raise ConfigError("isolation_requirements.max_cpu_steal_pct must be <= 1.0")
    bg_cap = _require_pos_number(iso["max_background_cpu_pct"], "isolation_requirements.max_background_cpu_pct")
    if bg_cap > 5.0:
        raise ConfigError("isolation_requirements.max_background_cpu_pct must be <= 5.0")
    if iso["allow_target_cpu_native"] is not False:
        raise ConfigError("allow_target_cpu_native must be false")
    if iso["network_egress"] != "disabled":
        raise ConfigError("network_egress must be 'disabled'")
    if iso["loopback_only"] is not True:
        raise ConfigError("loopback_only must be true")

    core_alloc = iso["core_allocation"]
    if not isinstance(core_alloc, dict):
        raise ConfigError("isolation_requirements.core_allocation must be an object")
    _check_keys(
        core_alloc,
        required=ALLOWED_CORE_ALLOC_KEYS,
        allowed=ALLOWED_CORE_ALLOC_KEYS,
        ctx="isolation_requirements.core_allocation",
    )
    if _require_pos_int(core_alloc["client_and_runtime"], "core_allocation.client_and_runtime") != 2:
        raise ConfigError("core_allocation.client_and_runtime must equal 2")
    if _require_pos_int(core_alloc["mock"], "core_allocation.mock") != 1:
        raise ConfigError("core_allocation.mock must equal 1")
    if _require_pos_int(core_alloc["sampler"], "core_allocation.sampler") != 1:
        raise ConfigError("core_allocation.sampler must equal 1")
    if core_alloc["smt_siblings_isolated"] is not True:
        raise ConfigError("core_allocation.smt_siblings_isolated must be true")

    toolchain = data["toolchain"]
    if not isinstance(toolchain, dict):
        raise ConfigError("environment.toolchain must be an object")
    _check_keys(
        toolchain,
        required=ALLOWED_TOOLCHAIN_KEYS,
        allowed=ALLOWED_TOOLCHAIN_KEYS,
        ctx="environment.toolchain",
    )
    if toolchain["require_pinned_versions"] is not True:
        raise ConfigError("environment.toolchain.require_pinned_versions must be true")

    _validate_pinned_version(toolchain["rust_toolchain"], "toolchain.rust_toolchain", require_semver=True)
    for semver_key in ("llvm_version", "node_version", "python_version"):
        if toolchain[semver_key] is not None:
            _validate_pinned_version(toolchain[semver_key], f"toolchain.{semver_key}", require_semver=True)
    for ver_key in ("c_compiler", "linker"):
        if toolchain[ver_key] is not None:
            _validate_pinned_version(toolchain[ver_key], f"toolchain.{ver_key}")
    for plat_key in ("ndk_version", "xcode_version"):
        if toolchain[plat_key] is not None and toolchain[plat_key] != "not_applicable":
            _validate_pinned_version(toolchain[plat_key], f"toolchain.{plat_key}")
    if toolchain["cargo_lock_sha256"] is not None:
        _validate_sha256_hex(toolchain["cargo_lock_sha256"], "toolchain.cargo_lock_sha256")
    if toolchain["dependency_lock_digests"] is not None:
        digests = toolchain["dependency_lock_digests"]
        if not isinstance(digests, dict) or not digests:
            raise ConfigError("toolchain.dependency_lock_digests must be a non-empty object when provided")
        for lock_name, digest in digests.items():
            _require_non_empty_str(lock_name, "toolchain.dependency_lock_digests key")
            _validate_sha256_hex(digest, f"toolchain.dependency_lock_digests[{lock_name!r}]")

    if status == "pending_dedicated_runner":
        if data["machine"] is not None:
            raise ConfigError("pending_dedicated_runner must keep machine=null")
        return

    # Active environment must attest all RFC-0043 §7.1 toolchain locks and concrete machine fields.
    for required_active_tc in (
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
        if toolchain[required_active_tc] is None:
            raise ConfigError(f"active environment requires toolchain.{required_active_tc} to be attested")

    machine = data["machine"]
    if not isinstance(machine, dict):
        raise ConfigError("active environment must provide machine attestation object")
    _check_keys(
        machine,
        required=ALLOWED_MACHINE_KEYS,
        allowed=ALLOWED_MACHINE_KEYS,
        ctx="environment.machine",
    )
    for str_field in (
        "machine_id",
        "cpu_model",
        "microcode",
        "kernel_version",
        "cpu_governor",
        "turbo_policy",
        "build_target",
    ):
        _require_non_empty_str(machine[str_field], f"machine.{str_field}")
    _validate_os_image_digest(machine["os_image_digest"], "machine.os_image_digest")
    stepping = machine["cpu_stepping"]
    if not (_is_strict_int(stepping) and stepping >= 0) and not (isinstance(stepping, str) and stepping.strip()):
        raise ConfigError(f"machine.cpu_stepping must be non-empty string or non-negative integer, got {stepping!r}")

    phys_cores = _require_pos_int(machine["physical_cores"], "machine.physical_cores")
    if phys_cores < min_cores:
        raise ConfigError(
            f"machine.physical_cores={phys_cores} < min_exclusive_physical_cores={min_cores}"
        )
    if not isinstance(machine["smt_enabled"], bool):
        raise ConfigError("machine.smt_enabled must be boolean")

    topo = machine["smt_topology"]
    if not isinstance(topo, dict):
        raise ConfigError("machine.smt_topology must be an object")
    _check_keys(
        topo,
        required=ALLOWED_SMT_TOPOLOGY_KEYS,
        allowed=ALLOWED_SMT_TOPOLOGY_KEYS,
        ctx="machine.smt_topology",
    )
    tpc = _require_pos_int(topo["threads_per_core"], "machine.smt_topology.threads_per_core")
    if machine["smt_enabled"] and tpc < 2:
        raise ConfigError("machine.smt_topology.threads_per_core must be >= 2 when smt_enabled is true")
    if not machine["smt_enabled"] and tpc != 1:
        raise ConfigError("machine.smt_topology.threads_per_core must equal 1 when smt_enabled is false")
    logical_cpus = _require_pos_int(topo["logical_cpus"], "machine.smt_topology.logical_cpus")
    if logical_cpus != phys_cores * tpc:
        raise ConfigError(
            f"machine.smt_topology.logical_cpus={logical_cpus} != physical_cores*threads_per_core ({phys_cores * tpc})"
        )

    affinity = topo["core_affinity"]
    if not isinstance(affinity, dict):
        raise ConfigError("machine.smt_topology.core_affinity must be an object")
    _check_keys(
        affinity,
        required=ALLOWED_CORE_AFFINITY_KEYS,
        allowed=ALLOWED_CORE_AFFINITY_KEYS,
        ctx="machine.smt_topology.core_affinity",
    )
    if affinity["smt_siblings_isolated"] is not True:
        raise ConfigError("machine.smt_topology.core_affinity.smt_siblings_isolated must be true")

    expected_counts = {
        "client_and_runtime_physical_cores": core_alloc["client_and_runtime"],
        "mock_physical_cores": core_alloc["mock"],
        "sampler_physical_cores": core_alloc["sampler"],
    }
    assigned_cores: list[int] = []
    for role_key, expected_len in expected_counts.items():
        cores = affinity[role_key]
        if not isinstance(cores, list) or len(cores) != expected_len:
            raise ConfigError(f"core_affinity.{role_key} must list exactly {expected_len} physical core(s)")
        for core_id in cores:
            cid = _require_non_neg_int(core_id, f"core_affinity.{role_key}[]")
            if cid >= phys_cores:
                raise ConfigError(f"core_affinity.{role_key} core {cid} >= physical_cores={phys_cores}")
            assigned_cores.append(cid)
    if len(set(assigned_cores)) != len(assigned_cores):
        raise ConfigError("machine.smt_topology.core_affinity physical cores must be mutually disjoint")

    ram_bytes = _require_pos_int(machine["ram_bytes"], "machine.ram_bytes")
    if ram_bytes < min_ram_gib * _GIB:
        raise ConfigError(f"machine.ram_bytes={ram_bytes} < required {min_ram_gib} GiB")

    if _require_non_neg_int(machine["swap_active_bytes"], "machine.swap_active_bytes") != 0:
        raise ConfigError("machine.swap_active_bytes must be 0 before measurement")

    if "native" in machine["build_target"].lower():
        raise ConfigError(f"machine.build_target must be generic fixed target, got {machine['build_target']!r}")
    if not isinstance(machine["rustflags"], str):
        raise ConfigError("machine.rustflags must be a string")
    if re.search(r"(?:target-cpu|cpu|-march|-mcpu)\s*=\s*native\b", machine["rustflags"], re.IGNORECASE):
        raise ConfigError("machine.rustflags must not enable native host CPU tuning")


def _validate_budget_spec(
    sid: str,
    metric_name: str,
    spec: Any,
    *,
    concurrency_levels: list[int] | None,
    case_ids: set[str] | None,
) -> None:
    ctx = f"scenario {sid}: budgets.{metric_name}"
    if not isinstance(spec, dict):
        raise ConfigError(f"{ctx} must be an object")

    if metric_name in ("p50_ns", "p95_ns", "ttft_p95_ns"):
        req = frozenset({"max_regression_pct", "min_abs_delta_ns", "max_abs_ns"})
        _check_keys(spec, required=req, allowed=req, ctx=ctx)
        _require_pos_number(spec["max_regression_pct"], f"{ctx}.max_regression_pct")
        _require_non_neg_int(spec["min_abs_delta_ns"], f"{ctx}.min_abs_delta_ns")
        if spec["max_abs_ns"] is not None:
            _require_pos_int(spec["max_abs_ns"], f"{ctx}.max_abs_ns")
        return

    if metric_name in ("throughput_events_per_s", "throughput_bytes_per_s"):
        req = frozenset({"max_drop_pct"})
        _check_keys(spec, required=req, allowed=req, ctx=ctx)
        drop_pct = _require_pos_number(spec["max_drop_pct"], f"{ctx}.max_drop_pct")
        if drop_pct > 100.0:
            raise ConfigError(f"{ctx}.max_drop_pct must be <= 100")
        return

    if metric_name == "retained_rss_bytes":
        req = frozenset({"max_regression_pct", "min_abs_delta_bytes", "max_abs_bytes"})
        _check_keys(spec, required=req, allowed=req, ctx=ctx)
        _require_pos_number(spec["max_regression_pct"], f"{ctx}.max_regression_pct")
        _require_non_neg_int(spec["min_abs_delta_bytes"], f"{ctx}.min_abs_delta_bytes")
        _require_pos_int(spec["max_abs_bytes"], f"{ctx}.max_abs_bytes")
        return

    if metric_name == "peak_rss_bytes":
        req = frozenset({"max_regression_pct", "min_abs_delta_bytes", "max_abs_bytes"})
        allowed = req | frozenset({"max_abs_bytes_by_concurrency"})
        _check_keys(spec, required=req, allowed=allowed, ctx=ctx)
        _require_pos_number(spec["max_regression_pct"], f"{ctx}.max_regression_pct")
        _require_non_neg_int(spec["min_abs_delta_bytes"], f"{ctx}.min_abs_delta_bytes")
        _require_pos_int(spec["max_abs_bytes"], f"{ctx}.max_abs_bytes")
        if "max_abs_bytes_by_concurrency" in spec:
            by_conc = spec["max_abs_bytes_by_concurrency"]
            if not isinstance(by_conc, dict) or not by_conc:
                raise ConfigError(f"{ctx}.max_abs_bytes_by_concurrency must be a non-empty object")
            for k, v in by_conc.items():
                if not isinstance(k, str) or not k.isdigit() or int(k) <= 0 or str(int(k)) != k:
                    raise ConfigError(
                        f"{ctx}.max_abs_bytes_by_concurrency key {k!r} must be positive integer string"
                    )
                _require_pos_int(v, f"{ctx}.max_abs_bytes_by_concurrency[{k!r}]")
            if concurrency_levels is not None:
                expected_keys = {str(c) for c in concurrency_levels}
                if set(by_conc) != expected_keys:
                    raise ConfigError(
                        f"{ctx}.max_abs_bytes_by_concurrency keys {sorted(by_conc)} must match "
                        f"concurrency_levels {sorted(expected_keys)}"
                    )
        return

    if metric_name == "rss_slope_bytes_per_1k":
        req = frozenset({"max_abs_bytes_per_1k"})
        _check_keys(spec, required=req, allowed=req, ctx=ctx)
        _require_pos_int(spec["max_abs_bytes_per_1k"], f"{ctx}.max_abs_bytes_per_1k")
        return

    if metric_name == "paired_delta_p50_ns":
        req = frozenset({"max_abs_delta_ns_by_case"})
        _check_keys(spec, required=req, allowed=req, ctx=ctx)
        by_case = spec["max_abs_delta_ns_by_case"]
        if not isinstance(by_case, dict) or not by_case:
            raise ConfigError(f"{ctx}.max_abs_delta_ns_by_case must be a non-empty object")
        for cid, cap in by_case.items():
            _require_non_empty_str(cid, f"{ctx}.max_abs_delta_ns_by_case key")
            _require_pos_int(cap, f"{ctx}.max_abs_delta_ns_by_case[{cid!r}]")
        if case_ids is None or set(by_case) != case_ids:
            raise ConfigError(
                f"{ctx}.max_abs_delta_ns_by_case keys {sorted(by_case)} must match scenario.cases {sorted(case_ids or set())}"
            )
        return

    raise ConfigError(f"{ctx}: unhandled metric {metric_name!r}")


def validate_scenarios_config(data: Any) -> dict[str, dict[str, Any]]:
    """Validate `bench/scenarios.json` and return a map keyed by scenario id."""
    if not isinstance(data, dict):
        raise ConfigError("scenarios config must be a JSON object")
    _check_keys(
        data,
        required=ALLOWED_SCENARIOS_TOP_KEYS,
        allowed=ALLOWED_SCENARIOS_TOP_KEYS,
        ctx="scenarios",
    )
    if _require_pos_int(data["schema_version"], "scenarios.schema_version") != 1:
        raise ConfigError("scenarios.schema_version must equal 1")

    protocol = data["protocol"]
    if not isinstance(protocol, dict):
        raise ConfigError("scenarios.protocol must be an object")
    _check_keys(
        protocol,
        required=ALLOWED_PROTOCOL_KEYS,
        allowed=ALLOWED_PROTOCOL_KEYS,
        ctx="scenarios.protocol",
    )
    for int_key in (
        "independent_pairs",
        "max_rerun_pairs",
        "bootstrap_iterations",
        "default_seed",
        "rss_slope_tail_windows",
        "max_rss_slope_bytes_per_1k_requests",
    ):
        _require_pos_int(protocol[int_key], f"protocol.{int_key}")
    if protocol["max_rerun_pairs"] < protocol["independent_pairs"]:
        raise ConfigError("protocol.max_rerun_pairs must be >= independent_pairs")
    if protocol["schedule"] != "AB_BA_seeded":
        raise ConfigError(f"protocol.schedule must equal 'AB_BA_seeded', got {protocol['schedule']!r}")
    conf = _require_pos_number(protocol["bootstrap_confidence"], "protocol.bootstrap_confidence")
    if conf >= 1.0:
        raise ConfigError("protocol.bootstrap_confidence must be < 1.0")
    for pct_key in ("max_base_p50_mad_cv_pct", "max_b0_drift_pct"):
        pct_val = _require_pos_number(protocol[pct_key], f"protocol.{pct_key}")
        if pct_val > 100.0:
            raise ConfigError(f"protocol.{pct_key} must be <= 100.0")
    floor_ratio = _require_pos_number(
        protocol["anchor_throughput_floor_ratio"], "protocol.anchor_throughput_floor_ratio"
    )
    if floor_ratio > 1.0:
        raise ConfigError("protocol.anchor_throughput_floor_ratio must be <= 1.0")

    scenarios_raw = data["scenarios"]
    if not isinstance(scenarios_raw, list) or not scenarios_raw:
        raise ConfigError("scenarios.scenarios must be a non-empty list")

    by_id: dict[str, dict[str, Any]] = {}
    for idx, entry in enumerate(scenarios_raw):
        if not isinstance(entry, dict):
            raise ConfigError(f"scenarios[{idx}] must be an object")
        sid = entry.get("id")
        if not isinstance(sid, str) or not sid.strip():
            raise ConfigError(f"scenarios[{idx}].id must be a non-empty string")
        if sid in by_id:
            raise ConfigError(f"duplicate scenario.id: {sid!r}")
        _check_keys(
            entry,
            required=REQUIRED_SCENARIO_KEYS,
            allowed=ALLOWED_SCENARIO_KEYS,
            ctx=f"scenario {sid}",
        )
        if entry["phase"] not in ALLOWED_PHASES:
            raise ConfigError(f"scenario {sid}: invalid phase {entry['phase']!r}")
        if entry["status"] not in ALLOWED_SCENARIO_STATUSES:
            raise ConfigError(f"scenario {sid}: invalid status {entry['status']!r}")

        for int_field in POSITIVE_INT_SCENARIO_FIELDS:
            if int_field in entry:
                _require_pos_int(entry[int_field], f"scenario {sid}.{int_field}")
        if "requires_b0_calibration" in entry and not isinstance(entry["requires_b0_calibration"], bool):
            raise ConfigError(f"scenario {sid}.requires_b0_calibration must be boolean")

        backends = entry["backends"]
        if (
            not isinstance(backends, list)
            or not backends
            or not all(isinstance(b, str) and b.strip() for b in backends)
            or len(set(backends)) != len(backends)
        ):
            raise ConfigError(f"scenario {sid}: backends must be non-empty list of unique non-empty strings")

        concurrency_levels: list[int] | None = None
        if "concurrency_levels" in entry:
            raw_levels = entry["concurrency_levels"]
            if not isinstance(raw_levels, list) or not raw_levels:
                raise ConfigError(f"scenario {sid}: concurrency_levels must be a non-empty list")
            concurrency_levels = [
                _require_pos_int(c, f"scenario {sid}.concurrency_levels[]") for c in raw_levels
            ]
            if len(set(concurrency_levels)) != len(concurrency_levels):
                raise ConfigError(f"scenario {sid}: duplicate values in concurrency_levels")

        case_ids: set[str] | None = None
        if "cases" in entry:
            raw_cases = entry["cases"]
            if not isinstance(raw_cases, list) or not raw_cases:
                raise ConfigError(f"scenario {sid}: cases must be a non-empty list")
            case_ids = set()
            for c_idx, case in enumerate(raw_cases):
                if not isinstance(case, dict):
                    raise ConfigError(f"scenario {sid}: cases[{c_idx}] must be an object")
                _check_keys(
                    case,
                    required=frozenset({"id"}),
                    allowed=ALLOWED_CASE_KEYS,
                    ctx=f"scenario {sid}.cases[{c_idx}]",
                )
                cid = _require_non_empty_str(case["id"], f"scenario {sid}.cases[{c_idx}].id")
                if cid in case_ids:
                    raise ConfigError(f"scenario {sid}: duplicate case id {cid!r}")
                case_ids.add(cid)
                for c_int in ("request_bytes", "response_bytes", "stream_events", "payload_bytes_per_event"):
                    if c_int in case:
                        _require_pos_int(case[c_int], f"scenario {sid}.cases[{c_idx}].{c_int}")
                if ("request_bytes" in case) != ("response_bytes" in case):
                    raise ConfigError(
                        f"scenario {sid}.cases[{c_idx}]: request_bytes and response_bytes must be paired"
                    )
                if ("stream_events" in case) != ("payload_bytes_per_event" in case):
                    raise ConfigError(
                        f"scenario {sid}.cases[{c_idx}]: stream_events and payload_bytes_per_event must be paired"
                    )
                if "request_bytes" not in case and "stream_events" not in case:
                    raise ConfigError(
                        f"scenario {sid}.cases[{c_idx}]: case must specify payload or stream dimensions"
                    )

        if "comparison_mode" in entry:
            if entry["comparison_mode"] != "paired_delta":
                raise ConfigError(f"scenario {sid}: unsupported comparison_mode {entry['comparison_mode']!r}")
            if "reference_backend" not in entry:
                raise ConfigError(f"scenario {sid}: comparison_mode='paired_delta' requires reference_backend")
        if "reference_backend" in entry:
            _require_non_empty_str(entry["reference_backend"], f"scenario {sid}.reference_backend")
            if entry.get("comparison_mode") != "paired_delta":
                raise ConfigError(f"scenario {sid}: reference_backend requires comparison_mode='paired_delta'")

        budgets = entry["budgets"]
        if not isinstance(budgets, dict):
            raise ConfigError(f"scenario {sid}: budgets must be an object")
        unknown_metrics = set(budgets) - ALLOWED_BUDGET_METRICS
        if unknown_metrics:
            raise ConfigError(f"scenario {sid}: unknown budget metrics {sorted(unknown_metrics)}")

        if "rss_slope_bytes_per_1k" in budgets:
            if "slope_tail_windows" not in entry:
                raise ConfigError(f"scenario {sid}: rss_slope_bytes_per_1k requires slope_tail_windows")
            if "memory_windows" in entry and entry["memory_windows"] < entry["slope_tail_windows"]:
                raise ConfigError(f"scenario {sid}: memory_windows < slope_tail_windows")

        for metric_name, spec in budgets.items():
            _validate_budget_spec(
                sid,
                metric_name,
                spec,
                concurrency_levels=concurrency_levels,
                case_ids=case_ids,
            )
        by_id[sid] = entry
    return by_id


def _validate_run_bundle(
    data: Any,
    label: str,
    scenarios_by_id: dict[str, dict[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any]]:
    if not isinstance(data, dict):
        raise ConfigError(f"{label}: run bundle must be a JSON object")
    _check_keys(
        data,
        required=REQUIRED_TOP_LEVEL_RUN_KEYS,
        allowed=ALLOWED_TOP_LEVEL_RUN_KEYS,
        ctx=label,
    )
    if _require_pos_int(data["schema_version"], f"{label}.schema_version") != 1:
        raise ConfigError(f"{label}: schema_version must equal 1")

    scenario_id = data["scenario"]
    if not isinstance(scenario_id, str) or scenario_id not in scenarios_by_id:
        raise ConfigError(f"{label}: unknown scenario {scenario_id!r}")
    scenario = scenarios_by_id[scenario_id]

    backend = data["backend"]
    if not isinstance(backend, str) or backend not in scenario["backends"]:
        raise ConfigError(
            f"{label}: backend {backend!r} not registered for scenario {scenario_id!r}"
        )

    for timeout_key in ("operation_timeout_ms", "run_timeout_ms"):
        tval = _require_pos_int(data[timeout_key], f"{label}.{timeout_key}")
        if tval != scenario[timeout_key]:
            raise ConfigError(
                f"{label}: {timeout_key}={tval} diverges from registered scenario "
                f"value {scenario[timeout_key]}"
            )

    _require_non_empty_str(data["recipe_hash"], f"{label}.recipe_hash")
    if "source_sha" in data:
        _require_non_empty_str(data["source_sha"], f"{label}.source_sha")

    concurrency = data.get("concurrency", 1)
    concurrency = _require_pos_int(concurrency, f"{label}.concurrency")
    if "concurrency_levels" in scenario:
        if concurrency not in scenario["concurrency_levels"]:
            raise ConfigError(
                f"{label}: concurrency={concurrency} not in registered levels {scenario['concurrency_levels']}"
            )
    elif concurrency != 1:
        raise ConfigError(f"{label}: concurrency={concurrency} invalid for single-concurrency scenario")

    if "cases" in scenario:
        valid_case_ids = {c["id"] for c in scenario["cases"]}
        if "case" not in data:
            raise ConfigError(f"{label}: scenario {scenario_id!r} requires 'case' in {sorted(valid_case_ids)}")
        case_id = data["case"]
        if not isinstance(case_id, str) or case_id not in valid_case_ids:
            raise ConfigError(f"{label}: case {case_id!r} not in {sorted(valid_case_ids)}")
    elif "case" in data:
        raise ConfigError(f"{label}: unexpected 'case' for scenario {scenario_id!r}")

    runs = data["runs"]
    if not isinstance(runs, list) or not runs:
        raise ConfigError(f"{label}: runs must be a non-empty list")

    required_budgets = scenario["budgets"]
    slope_tail = scenario.get("slope_tail_windows")

    expected_metric_keys: set[str] = set()
    for m_name in required_budgets:
        if m_name == "rss_slope_bytes_per_1k":
            continue
        if m_name == "paired_delta_p50_ns":
            expected_metric_keys.update({"stdio_p50_ns", "in_process_p50_ns"})
        else:
            expected_metric_keys.add(m_name)
    if "idle_rss_max_bytes" in scenario:
        expected_metric_keys.add("idle_rss_bytes")
    if "shutdown_max_ms" in scenario:
        expected_metric_keys.add("shutdown_ms")
    if "max_queued_payload_bytes" in scenario:
        expected_metric_keys.add("max_queued_payload_bytes")
    allowed_metric_keys = expected_metric_keys | {"p99_ns"}

    seen_pairs: set[int] = set()
    memory_windows = scenario.get("memory_windows")
    requires_b0 = bool(scenario.get("requires_b0_calibration", False))
    for idx, run in enumerate(runs):
        if not isinstance(run, dict):
            raise ConfigError(f"{label}: run[{idx}] must be an object")
        _check_keys(
            run,
            required=REQUIRED_RUN_ENTRY_KEYS,
            allowed=ALLOWED_RUN_ENTRY_KEYS,
            ctx=f"{label}: run[{idx}]",
        )
        pair_idx = _require_non_neg_int(run["pair_index"], f"{label}: run[{idx}].pair_index")
        if pair_idx in seen_pairs:
            raise ConfigError(f"{label}: duplicate pair_index {pair_idx}")
        seen_pairs.add(pair_idx)

        for int_field in (
            "samples",
            "timed_out_operations",
            "correctness_failures",
            "leaked_handles",
        ):
            _require_non_neg_int(run[int_field], f"{label}: run[{idx}].{int_field}")
        if not isinstance(run["run_timed_out"], bool):
            raise ConfigError(f"{label}: run[{idx}].run_timed_out must be boolean")

        metrics = run["metrics"]
        if not isinstance(metrics, dict):
            raise ConfigError(f"{label}: run[{idx}].metrics must be an object")
        unknown_run_metrics = set(metrics) - allowed_metric_keys
        if unknown_run_metrics:
            raise ConfigError(
                f"{label}: run[{idx}].metrics has unknown keys {sorted(unknown_run_metrics)}"
            )

        if "rss_slope_bytes_per_1k" in required_budgets or "rss_windows_bytes" in run:
            windows = run.get("rss_windows_bytes")
            if not isinstance(windows, list):
                raise ConfigError(
                    f"{label}: run[{idx}] missing rss_windows_bytes for slope check"
                )
            if "rss_slope_bytes_per_1k" in required_budgets:
                if _is_strict_int(memory_windows):
                    if len(windows) != memory_windows:
                        raise ConfigError(
                            f"{label}: run[{idx}].rss_windows_bytes length {len(windows)} "
                            f"!= required memory_windows {memory_windows}"
                        )
                elif not _is_strict_int(slope_tail) or len(windows) < slope_tail:
                    raise ConfigError(
                        f"{label}: run[{idx}].rss_windows_bytes length {len(windows)} "
                        f"< required tail {slope_tail}"
                    )
            for w_idx, w in enumerate(windows):
                _require_pos_int(w, f"{label}: run[{idx}].rss_windows_bytes[{w_idx}]")

        for metric_name in expected_metric_keys:
            if metric_name not in metrics:
                raise ConfigError(f"{label}: run[{idx}].metrics missing {metric_name!r}")
            mval = metrics[metric_name]
            if metric_name in ("throughput_events_per_s", "throughput_bytes_per_s"):
                _require_pos_number(mval, f"{label}: run[{idx}].metrics.{metric_name}")
            elif metric_name == "max_queued_payload_bytes":
                _require_non_neg_int(mval, f"{label}: run[{idx}].metrics.{metric_name}")
            else:
                _require_pos_int(mval, f"{label}: run[{idx}].metrics.{metric_name}")

        if "p99_ns" in metrics:
            _require_pos_int(metrics["p99_ns"], f"{label}: run[{idx}].metrics.p99_ns")

        if requires_b0 and "b0_drift_pct" not in run:
            raise ConfigError(f"{label}: run[{idx}] missing required b0_drift_pct")
        if "b0_drift_pct" in run and not _is_real_number(run["b0_drift_pct"]):
            raise ConfigError(f"{label}: run[{idx}].b0_drift_pct must be finite number")
        for non_neg_pct in ("cpu_steal_pct", "background_cpu_pct"):
            _require_non_neg_number(run[non_neg_pct], f"{label}: run[{idx}].{non_neg_pct}")

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
    def _safe_ratio(delta: float, ref: float) -> float:
        if ref > 0.0:
            return delta / ref
        return float("inf") if delta > 0.0 else 0.0

    n = len(ref_vals)
    ref_mean = sum(ref_vals) / n
    head_mean = sum(head_vals) / n
    if higher_is_worse:
        point_abs = head_mean - ref_mean
    else:
        point_abs = ref_mean - head_mean
    point_ratio = _safe_ratio(point_abs, ref_mean)

    ratios: list[float] = []
    abs_deltas: list[float] = []
    indices = range(n)
    for _ in range(iterations):
        sample_idx = rng.choices(indices, k=n)
        s_ref = sum(ref_vals[i] for i in sample_idx) / n
        s_head = sum(head_vals[i] for i in sample_idx) / n
        if higher_is_worse:
            d_abs = s_head - s_ref
        else:
            d_abs = s_ref - s_head
        ratios.append(_safe_ratio(d_abs, s_ref))
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
    environment_data: Any | None = None,
    seed: int | None = None,
) -> dict[str, Any]:
    """Run all RFC-0043 §7 checks and return a structured result dictionary."""
    failure_codes: list[str] = []

    def add_failure(code: str) -> None:
        if code not in failure_codes:
            failure_codes.append(code)

    try:
        if environment_data is not None:
            validate_environment_config(environment_data)
            if environment_data["status"] != "active":
                raise ConfigError(
                    "environment must have status='active' with attested machine and toolchain for formal comparison"
                )
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
        fields = ["scenario", "backend", "concurrency", "operation_timeout_ms", "run_timeout_ms"]
        if "case" in a or "case" in b:
            fields.append("case")
        for field in fields:
            if a.get(field) != b.get(field):
                add_failure("BENCH_CONFIG_INVALID")
                errors.append(f"{b_name}.{field}={b.get(field)!r} does not match base.{field}={a.get(field)!r}")
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
    if not failure_codes:
        base_pairs = [r["pair_index"] for r in base["runs"]]
        head_pairs = [r["pair_index"] for r in head["runs"]]
        if base_pairs != head_pairs:
            add_failure("BENCH_CONFIG_INVALID")
            errors.append("base and head runs must align by identical pair_index sequence")
        if anchor is not None and [r["pair_index"] for r in anchor["runs"]] != base_pairs:
            add_failure("BENCH_CONFIG_INVALID")
            errors.append("anchor runs must align with base/head by identical pair_index sequence")

    if failure_codes:
        res = {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": failure_codes,
            "errors": errors,
            "scenario": base["scenario"],
            "backend": base["backend"],
            "metrics": {},
        }
        if "case" in base:
            res["case"] = base["case"]
        return res

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

    # 2. Environment stability: B0 drift, CPU steal / background, and base P50 MAD CV <= 3%.
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
    base_mad_cv_pct: float | None = None
    if "p50_ns" in budgets:
        base_series = [float(r["metrics"]["p50_ns"]) for r in base["runs"]]
        base_mad_cv_pct = _robust_mad_cv_pct(base_series)
        if base_mad_cv_pct > protocol["max_base_p50_mad_cv_pct"]:
            add_failure("BENCH_ENV_UNSTABLE")
            errors.append(
                f"base p50_ns robust MAD/median dispersion "
                f"{base_mad_cv_pct:.2f}% > {protocol['max_base_p50_mad_cv_pct']}%"
            )

    if any(code in failure_codes for code in ("BENCH_TIMEOUT", "BENCH_CORRECTNESS_FAILURE")):
        res = {
            "schema_version": 1,
            "status": "FAIL",
            "failure_codes": failure_codes,
            "errors": errors,
            "scenario": head["scenario"],
            "backend": head["backend"],
            "base_dispersion_pct": base_mad_cv_pct,
            "metrics": {},
        }
        if "case" in head:
            res["case"] = head["case"]
        return res

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

        if metric_name == "paired_delta_p50_ns":
            case_id = head["case"]
            limit_ns = float(spec["max_abs_delta_ns_by_case"][case_id])
            head_deltas = [
                float(r["metrics"]["stdio_p50_ns"] - r["metrics"]["in_process_p50_ns"])
                for r in head["runs"]
            ]
            base_deltas = [
                float(r["metrics"]["stdio_p50_ns"] - r["metrics"]["in_process_p50_ns"])
                for r in base["runs"]
            ]
            worst_delta = max(head_deltas)
            passed = worst_delta <= limit_ns
            if not passed:
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(
                    f"head paired_delta_p50_ns ({case_id}) max={worst_delta:.0f} > guardrail={limit_ns:.0f}"
                )
            metric_reports[metric_name] = {
                "status": "PASS" if passed else "FAIL",
                "case": case_id,
                "worst_delta_ns": worst_delta,
                "median_delta_ns": float(statistics.median(head_deltas)),
                "mean_delta_ns": sum(head_deltas) / len(head_deltas),
                "base_mean_delta_ns": sum(base_deltas) / len(base_deltas),
                "limit_delta_ns": limit_ns,
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

    # 3b. Enforce scenario-level §8 absolute caps (idle RSS, shutdown time, queued payload).
    for sc_cap_key, metric_key in (
        ("idle_rss_max_bytes", "idle_rss_bytes"),
        ("shutdown_max_ms", "shutdown_ms"),
        ("max_queued_payload_bytes", "max_queued_payload_bytes"),
    ):
        if sc_cap_key in scenario:
            cap_val = float(scenario[sc_cap_key])
            head_cap_vals = [float(r["metrics"][metric_key]) for r in head["runs"]]
            worst_val = max(head_cap_vals)
            passed = worst_val <= cap_val
            if not passed:
                add_failure("PERF_GUARDRAIL_EXCEEDED")
                errors.append(
                    f"head {metric_key} max={worst_val:.0f} > scenario {sc_cap_key}={cap_val:.0f}"
                )
            metric_reports[metric_key] = {
                "status": "PASS" if passed else "FAIL",
                "worst_value": worst_val,
                "limit": cap_val,
            }

    overall = "PASS" if not failure_codes else "FAIL"
    report = {
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
    if "case" in head:
        report["case"] = head["case"]
    return report


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
        env_raw = load_json_strict(args.environment) if args.environment is not None else None
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
        environment_data=env_raw,
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
