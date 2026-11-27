#!/usr/bin/env python3
"""Glob-based artifact size budget and archive hygiene gate (RFC-0043)."""

from __future__ import annotations

import argparse
import fnmatch
import gzip
import hashlib
import json
import os
import posixpath
import re
import stat
import subprocess
import sys
import tarfile
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BUDGETS = REPO_ROOT / "quality" / "size-budgets.json"
MAX_EXCEPTION_DAYS = 14
VALID_METRICS = ("raw_bytes", "packed_bytes", "unpacked_bytes")
ZIP_EXTS, TAR_EXTS, NATIVE_EXTS = (".zip", ".whl", ".jar"), (".tar.gz", ".tgz", ".crate"), (".so", ".dylib", ".dll", ".pyd")
IOS_SLICES = (
    "aimux_ffi.xcframework/ios-arm64/aimux_ffi.framework/aimux_ffi",
    "aimux_ffi.xcframework/ios-arm64_x86_64-simulator/aimux_ffi.framework/aimux_ffi",
)


class BudgetError(ValueError):
    """Schema, integrity, or CLI error."""


def _is_pos_int(v: Any) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and v > 0


def _strip_single_v(v: str) -> str:
    s = v.strip()
    return s[1:] if s.startswith("v") else s


def load_json_strict(path: Path) -> Any:
    def reject_dups(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        out: dict[str, Any] = {}
        for k, v in pairs:
            if k in out:
                raise BudgetError(f"{path}: duplicate JSON key {k!r}")
            out[k] = v
        return out

    def reject_const(tok: str) -> Any:
        raise BudgetError(f"{path}: invalid JSON constant {tok!r}")

    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_dups, parse_constant=reject_const)
    except (OSError, json.JSONDecodeError) as exc:
        raise BudgetError(f"{path}: failed to load JSON: {exc}") from exc


def _parse_iso_utc(raw: Any, label: str) -> datetime:
    if not isinstance(raw, str) or not raw.strip():
        raise BudgetError(f"{label}: expected ISO-8601 timestamp")
    s = raw.strip()
    try:
        dt = datetime.fromisoformat(s[:-1] + "+00:00" if s.endswith("Z") else s)
    except ValueError as exc:
        raise BudgetError(f"{label}: invalid ISO-8601 timestamp {raw!r}") from exc
    if dt.tzinfo is None:
        raise BudgetError(f"{label}: timestamp must include timezone: {raw!r}")
    return dt.astimezone(timezone.utc)


def _validate_threshold(spec: Any, label: str) -> dict[str, int | None]:
    if not isinstance(spec, dict) or not _is_pos_int(spec.get("ceiling")):
        raise BudgetError(f"{label}: ceiling must be a positive integer")
    target = spec.get("target")
    if target is not None and (not _is_pos_int(target) or target > spec["ceiling"]):
        raise BudgetError(f"{label}: target must be a positive integer <= ceiling")
    return {"ceiling": spec["ceiling"], "target": target}


def validate_config(raw: Any, *, now: datetime | None = None) -> dict[str, Any]:
    if not isinstance(raw, dict) or raw.get("schema_version") != 1:
        raise BudgetError("size-budgets.json must be an object with schema_version == 1")
    rules = raw.get("rules")
    if not isinstance(rules, list) or not rules:
        raise BudgetError("rules must be a non-empty list")

    if now is None:
        now_utc = datetime.now(timezone.utc)
    else:
        if now.tzinfo is None:
            raise BudgetError("now must be timezone-aware")
        now_utc = now.astimezone(timezone.utc)
    rules_by_cat: dict[str, dict[str, Any]] = {}
    for i, rule in enumerate(rules):
        if not isinstance(rule, dict):
            raise BudgetError(f"rules[{i}] must be an object")
        cat = rule.get("category")
        if not isinstance(cat, str) or not cat.strip() or cat in rules_by_cat:
            raise BudgetError(f"rules[{i}]: invalid or duplicate category {cat!r}")
        globs = rule.get("globs")
        if not isinstance(globs, list) or not globs or not all(isinstance(g, str) and g.strip() for g in globs):
            raise BudgetError(f"rules[{cat}]: globs must be a non-empty list of strings")
        metrics = {m: _validate_threshold(rule[m], f"rules[{cat}].{m}") for m in VALID_METRICS if m in rule}
        if not metrics:
            raise BudgetError(f"rules[{cat}]: must define at least one metric ceiling")
        rules_by_cat[cat] = {"category": cat, "globs": globs, "metrics": metrics}

    aggregates, agg_names = [], set()
    for i, agg in enumerate(raw.get("aggregates", [])):
        if not isinstance(agg, dict):
            raise BudgetError(f"aggregates[{i}] must be an object")
        name, cats, metric = agg.get("name"), agg.get("categories"), agg.get("metric")
        if not isinstance(name, str) or not name.strip() or name in agg_names:
            raise BudgetError(f"aggregates[{i}]: invalid or duplicate name {name!r}")
        if not isinstance(cats, list) or not cats or any(c not in rules_by_cat for c in cats):
            raise BudgetError(f"aggregates[{name}]: invalid categories")
        if metric not in VALID_METRICS:
            raise BudgetError(f"aggregates[{name}]: invalid metric {metric!r}")
        th = _validate_threshold(agg, f"aggregates[{name}]")
        agg_names.add(name)
        aggregates.append({"name": name, "categories": cats, "metric": metric, **th})

    exceptions, valid_targets = [], set(rules_by_cat) | agg_names
    max_exp = now_utc + timedelta(days=MAX_EXCEPTION_DAYS)
    for i, exc in enumerate(raw.get("exceptions", [])):
        if not isinstance(exc, dict):
            raise BudgetError(f"exceptions[{i}] must be an object")
        if exc.get("category") not in valid_targets or exc.get("metric") not in VALID_METRICS or not _is_pos_int(exc.get("temp_cap_bytes")):
            raise BudgetError(f"exceptions[{i}]: invalid category, metric, or temp_cap_bytes")
        for req in ("reason", "issue", "approver", "commit_or_version"):
            if not isinstance(exc.get(req), str) or not exc[req].strip():
                raise BudgetError(f"exceptions[{i}]: missing {req!r}")
        exp_dt = _parse_iso_utc(exc.get("expires_at"), f"exceptions[{i}].expires_at")
        if exp_dt <= now_utc:
            raise BudgetError(f"exceptions[{i}] ({exc['category']}): expired at {exc['expires_at']}")
        if exp_dt > max_exp:
            raise BudgetError(f"exceptions[{i}] ({exc['category']}): expires_at exceeds {MAX_EXCEPTION_DAYS}-day maximum lifetime")
        exceptions.append(exc)

    return {"rules": list(rules_by_cat.values()), "rules_by_cat": rules_by_cat, "aggregates": aggregates, "exceptions": exceptions}


def _norm_member_path(raw: str, archive: Path) -> str:
    if not raw or "\\" in raw or raw.startswith("/") or (len(raw) >= 2 and raw[1] == ":"):
        raise BudgetError(f"{archive}: unsafe archive path {raw!r}")
    parts = [p for p in raw.rstrip("/").split("/") if p not in ("", ".")]
    if not parts or any(p == ".." for p in parts):
        raise BudgetError(f"{archive}: path traversal in archive member {raw!r}")
    return posixpath.normpath("/".join(parts))


def _check_path_collisions(members: dict[str, int], dirs: set[str], archive: Path) -> None:
    overlap = dirs & set(members)
    if overlap:
        raise BudgetError(f"{archive}: archive path collides as both file and directory: {sorted(overlap)[0]!r}")
    for path_str in set(members) | dirs:
        parent = posixpath.dirname(path_str)
        while parent:
            if parent in members:
                raise BudgetError(f"{archive}: archive file collides with parent path {parent!r}")
            parent = posixpath.dirname(parent)


def inspect_zip(path: Path) -> tuple[int, dict[str, int]]:
    members: dict[str, int] = {}
    dirs: set[str] = set()
    try:
        with zipfile.ZipFile(path, "r") as zf:
            bad = zf.testzip()
            if bad is not None:
                raise BudgetError(f"{path}: corrupted zip member {bad!r} (CRC/data error)")
            for info in zf.infolist():
                mode = info.external_attr >> 16
                file_type = stat.S_IFMT(mode)
                if file_type == stat.S_IFLNK:
                    raise BudgetError(f"{path}: symlink entry not allowed: {info.filename!r}")
                if file_type not in (0, stat.S_IFREG, stat.S_IFDIR):
                    raise BudgetError(f"{path}: unsupported non-regular zip member {info.filename!r}")
                is_dir = info.is_dir() or file_type == stat.S_IFDIR or bool(info.external_attr & 0x10)
                if is_dir:
                    if file_type == stat.S_IFREG:
                        raise BudgetError(f"{path}: conflicting file/directory mode on zip member {info.filename!r}")
                    if info.file_size > 0:
                        raise BudgetError(f"{path}: directory entry has non-zero payload: {info.filename!r}")
                    norm = _norm_member_path(info.filename, path)
                    if norm in dirs:
                        raise BudgetError(f"{path}: duplicate archive directory {norm!r}")
                    dirs.add(norm)
                    continue
                norm = _norm_member_path(info.filename, path)
                if norm in members:
                    raise BudgetError(f"{path}: duplicate archive member {norm!r}")
                members[norm] = info.file_size
    except (zipfile.BadZipFile, zlib_error()) as exc:
        raise BudgetError(f"{path}: corrupted zip archive: {exc}") from exc
    if not members:
        raise BudgetError(f"{path}: archive contains no files")
    _check_path_collisions(members, dirs, path)
    return sum(members.values()), members


def zlib_error() -> type[Exception]:
    import zlib
    return zlib.error


def inspect_tar(path: Path) -> tuple[int, dict[str, int]]:
    members: dict[str, int] = {}
    dirs: set[str] = set()
    try:
        is_gz = path.name.lower().endswith((".tar.gz", ".tgz", ".crate"))
        raw_fh = gzip.open(path, "rb") if is_gz else path.open("rb")
        with raw_fh:
            with tarfile.open(fileobj=raw_fh, mode="r:", errorlevel=2) as tf:
                for m in tf.getmembers():
                    if m.issym() or m.islnk():
                        raise BudgetError(f"{path}: symlink/hardlink not allowed: {m.name!r}")
                    if m.isdir():
                        if m.name not in (".", "./"):
                            norm = _norm_member_path(m.name, path)
                            if norm in dirs:
                                raise BudgetError(f"{path}: duplicate archive directory {norm!r}")
                            dirs.add(norm)
                        continue
                    if not m.isreg():
                        raise BudgetError(f"{path}: unsupported special tar member {m.name!r}")
                    norm = _norm_member_path(m.name, path)
                    if norm in members:
                        raise BudgetError(f"{path}: duplicate archive member {norm!r}")
                    ef = tf.extractfile(m)
                    if ef is None:
                        raise BudgetError(f"{path}: unable to read tar member {norm!r}")
                    read_total = 0
                    for chunk in iter(lambda: ef.read(1 << 16), b""):
                        read_total += len(chunk)
                    if read_total != m.size:
                        raise BudgetError(f"{path}: truncated tar member {norm!r} ({read_total} != {m.size})")
                    members[norm] = m.size
                raw_fh.seek(tf.offset)
            # Drain remaining tar padding to EOF: require >= 2 zero end-of-archive blocks (1024 B)
            # aligned to 512 B, and reject any non-zero bytes (corrupt header / trailing garbage).
            trailing_bytes = 0
            for chunk in iter(lambda: raw_fh.read(1 << 16), b""):
                if any(chunk):
                    raise BudgetError(f"{path}: corrupt trailing tar header or data after offset {tf.offset}")
                trailing_bytes += len(chunk)
            if trailing_bytes < 1024 or trailing_bytes % 512 != 0:
                raise BudgetError(
                    f"{path}: truncated tar archive (expected >= 1024 zero end-of-archive bytes, got {trailing_bytes})"
                )
    except (tarfile.TarError, EOFError, OSError) as exc:
        raise BudgetError(f"{path}: corrupted tar archive: {exc}") from exc
    if not members:
        raise BudgetError(f"{path}: archive contains no files")
    _check_path_collisions(members, dirs, path)
    return sum(members.values()), members


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _match_rule(rel_path: str, rules: list[dict[str, Any]]) -> dict[str, Any] | None:
    norm = PurePosixPath(rel_path).as_posix().lstrip("./")
    base = PurePosixPath(norm).name
    for rule in rules:
        for pat in rule["globs"]:
            if fnmatch.fnmatchcase(norm, pat) or ("/" not in pat and fnmatch.fnmatchcase(base, pat)):
                return rule
    return None


def _resolve_commit_info(source_sha: str | None, repo_root: Path = REPO_ROOT) -> tuple[str | None, str | None]:
    if not source_sha or not source_sha.strip():
        return None, None
    sha = source_sha.strip()
    try:
        rev = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", f"{sha}^{{commit}}"],
            cwd=repo_root, capture_output=True, text=True, check=False,
        )
        resolved = rev.stdout.strip().lower()
        if rev.returncode != 0 or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", resolved):
            return None, None
        show = subprocess.run(["git", "show", f"{resolved}:Cargo.toml"], cwd=repo_root, capture_output=True, text=True, check=False)
        if show.returncode != 0:
            return resolved, None
    except OSError:
        return None, None
    try:
        import tomllib  # type: ignore[import-not-found]
        ver = tomllib.loads(show.stdout).get("workspace", {}).get("package", {}).get("version")
        return resolved, (ver.strip() if isinstance(ver, str) and ver.strip() else None)
    except ImportError:
        pass
    in_pkg = False
    for line in show.stdout.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_pkg = s == "[workspace.package]"
        elif in_pkg and s.startswith("version"):
            m = re.match(r'''^version\s*=\s*(?:"([^"]+)"|'([^']+)')''', s)
            if m:
                return resolved, (m.group(1) or m.group(2)).strip()
    return resolved, None


def _exception_for(
    exceptions: list[dict[str, Any]], cat: str, rel: str, metric: str, resolved_sha: str | None, commit_ver: str | None
) -> dict[str, Any] | None:
    for exc in exceptions:
        if exc["category"] != cat or exc["metric"] != metric:
            continue
        pglob = exc.get("path_glob")
        if pglob and not (fnmatch.fnmatchcase(rel, pglob) or fnmatch.fnmatchcase(PurePosixPath(rel).name, pglob)):
            continue
        scope = exc.get("commit_or_version")
        if scope:
            s, s_low = scope.strip(), scope.strip().lower()
            sha_ok = bool(resolved_sha and len(s_low) >= 7 and all(c in "0123456789abcdef" for c in s_low) and resolved_sha.startswith(s_low))
            ver_ok = bool(commit_ver and _strip_single_v(s) == _strip_single_v(commit_ver))
            if not (sha_ok or ver_ok):
                continue
        return exc
    return None


def _collect_targets(paths: list[Path]) -> list[tuple[str, Path]]:
    found: list[tuple[str, Path]] = []
    for p in paths:
        if p.is_symlink() or not p.exists():
            raise BudgetError(f"{p}: missing path or disallowed symlink")
        if p.is_dir():
            if p.name.endswith(".xcframework"):
                found.append((p.name, p))
                continue
            for root, dirs, files in os.walk(p, followlinks=False):
                rp, keep = Path(root), []
                for d in sorted(dirs):
                    dp = rp / d
                    if dp.is_symlink():
                        raise BudgetError(f"{dp}: symlinks are not allowed")
                    if d.endswith(".xcframework"):
                        found.append((dp.relative_to(p).as_posix(), dp))
                    else:
                        keep.append(d)
                dirs[:] = keep
                for f in sorted(files):
                    fp = rp / f
                    if fp.is_symlink() or not fp.is_file():
                        raise BudgetError(f"{fp}: symlink or non-regular file not allowed")
                    found.append((fp.relative_to(p).as_posix(), fp))
        elif p.is_file():
            found.append((p.name, p))
        else:
            raise BudgetError(f"{p}: unsupported non-regular path")
    return found


def _measure_target(path: Path) -> tuple[dict[str, int], list[tuple[str, int]]]:
    metrics: dict[str, int] = {}
    slices: list[tuple[str, int]] = []
    lower = path.name.lower()

    if path.is_dir() and path.name.endswith(".xcframework"):
        files_map: dict[str, int] = {}
        for root, dirs, files in os.walk(path, followlinks=False):
            rp = Path(root)
            for d in dirs:
                if (rp / d).is_symlink():
                    raise BudgetError(f"{rp / d}: symlink not allowed in xcframework")
            for f in files:
                fp = rp / f
                if fp.is_symlink() or not fp.is_file():
                    raise BudgetError(f"{fp}: non-regular file in xcframework")
                files_map[f"{path.name}/{fp.relative_to(path).as_posix()}"] = fp.stat().st_size
        if files_map.get(f"{path.name}/Info.plist", 0) <= 0:
            raise BudgetError(f"{path}: missing or empty top-level Info.plist")
        for sl in IOS_SLICES:
            if files_map.get(sl, 0) <= 0:
                raise BudgetError(f"{path}: missing or empty xcframework slice {sl}")
            slices.append((sl, files_map[sl]))
        return {"unpacked_bytes": sum(files_map.values())}, slices

    packed = path.stat().st_size
    if lower.endswith(ZIP_EXTS):
        unpacked, members = inspect_zip(path)
        metrics = {"packed_bytes": packed, "unpacked_bytes": unpacked}
        if lower.endswith(".whl"):
            natives = [sz for name, sz in members.items() if name.lower().endswith(NATIVE_EXTS)]
            if not natives:
                raise BudgetError(f"{path}: Python wheel contains no native extension")
            metrics["raw_bytes"] = max(natives)
        if path.name == "aimux_ffi.xcframework.zip":
            if members.get("aimux_ffi.xcframework/Info.plist", 0) <= 0:
                raise BudgetError(f"{path}: missing or empty aimux_ffi.xcframework/Info.plist")
            for sl in IOS_SLICES:
                if members.get(sl, 0) <= 0:
                    raise BudgetError(f"{path}: missing or empty xcframework slice {sl}")
                slices.append((sl, members[sl]))
    elif lower.endswith(TAR_EXTS):
        unpacked, _ = inspect_tar(path)
        metrics = {"packed_bytes": packed, "unpacked_bytes": unpacked}
    else:
        metrics = {"raw_bytes": packed}
    return metrics, slices


def evaluate(
    config: dict[str, Any],
    paths: list[Path],
    *,
    category_override: str | None = None,
    allow_unmatched: bool = False,
    source_sha: str | None = None,
    compare_report: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], int]:
    rules, rules_by_cat, exceptions = config["rules"], config["rules_by_cat"], config["exceptions"]
    if category_override is not None and category_override not in rules_by_cat:
        raise BudgetError(f"unknown --category {category_override!r}")

    resolved_sha, commit_ver = _resolve_commit_info(source_sha) if exceptions else (None, None)
    compare_map = {(r["path"], r["metric"]): r["actual_bytes"] for r in (compare_report or {}).get("results", [])}
    results, failures, warnings, cat_totals = [], [], [], {}
    used_exception = False

    def record(label: str, cat: str, metric: str, actual: int, spec: dict[str, int | None], sha: str | None = None) -> None:
        nonlocal used_exception
        ceiling, target = int(spec["ceiling"]), spec.get("target")  # type: ignore[arg-type]
        base = compare_map.get((label, metric))
        status, exc_used = "PASS", None
        if actual > ceiling:
            exc = _exception_for(exceptions, cat, label, metric, resolved_sha, commit_ver)
            if exc and actual <= exc["temp_cap_bytes"]:
                status, used_exception = "PASS_WITH_EXCEPTION", True
                exc_used = {"issue": exc["issue"], "temp_cap_bytes": exc["temp_cap_bytes"], "expires_at": exc["expires_at"]}
            else:
                status = "FAIL"
        target_warning = status == "PASS" and target is not None and actual > target
        row = {
            "path": label, "category": cat, "metric": metric, "actual_bytes": actual,
            "ceiling_bytes": ceiling, "target_bytes": target, "target_warning": target_warning,
            "delta_vs_base_bytes": (actual - base) if base is not None else None, "status": status,
            **({"sha256": sha} if sha else {}), **({"exception": exc_used} if exc_used else {}),
        }
        results.append(row)
        (failures if status == "FAIL" else warnings if target_warning else []).append(row)

    matched = 0
    for rel, fp in _collect_targets(paths):
        rule = rules_by_cat[category_override] if category_override else _match_rule(rel, rules)
        if rule is None:
            if allow_unmatched:
                continue
            raise BudgetError(f"unmatched artifact {rel!r} (add a rule in size-budgets.json or pass --allow-unmatched)")
        matched += 1
        metrics, slices = _measure_target(fp)
        digest = sha256_file(fp) if fp.is_file() else None
        for m, spec in rule["metrics"].items():
            if m not in metrics:
                raise BudgetError(f"{rel}: metric {m!r} not available for category {rule['category']!r}")
            record(rel, rule["category"], m, metrics[m], spec, sha=digest)
            cat_totals[(rule["category"], m)] = cat_totals.get((rule["category"], m), 0) + metrics[m]
        if slices and "ios_slice" in rules_by_cat:
            sl_spec = rules_by_cat["ios_slice"]["metrics"]["raw_bytes"]
            for sl_rel, sl_bytes in slices:
                record(sl_rel, "ios_slice", "raw_bytes", sl_bytes, sl_spec)

    if matched == 0:
        raise BudgetError("zero artifacts matched for budget evaluation")
    for agg in config["aggregates"]:
        if all((c, agg["metric"]) in cat_totals for c in agg["categories"]):
            total = sum(cat_totals[(c, agg["metric"])] for c in agg["categories"])
            record(f"<aggregate:{agg['name']}>", agg["name"], agg["metric"], total, {"ceiling": agg["ceiling"], "target": agg["target"]})

    overall = "FAIL" if failures else ("PASS_WITH_EXCEPTION" if used_exception else "PASS")
    return {
        "schema_version": 1, "status": overall, "checked_count": len(results),
        "failure_count": len(failures), "warning_count": len(warnings), "results": results,
    }, (1 if failures else 0)


def _write_step_summary(summary_path: Path, report: dict[str, Any]) -> None:
    lines = [
        f"### Size Budget Gate: `{report['status']}`\n",
        "| Artifact | Category | Metric | Actual | Ceiling | Status |",
        "|---|---|---|---:|---:|---|",
    ]
    for r in report["results"]:
        note = f"WARN (target {r['target_bytes']:,})" if r.get("target_warning") else r["status"]
        if r.get("delta_vs_base_bytes") is not None:
            note += f" ({r['delta_vs_base_bytes']:+,} B)"
        lines.append(f"| `{r['path']}` | `{r['category']}` | `{r['metric']}` | {r['actual_bytes']:,} | {r['ceiling_bytes']:,} | {note} |")
    with summary_path.open("a", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n\n")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Check artifact size budgets and archive hygiene (RFC-0043).")
    p.add_argument("paths", nargs="*", type=Path)
    p.add_argument("--budgets", type=Path, default=DEFAULT_BUDGETS)
    p.add_argument("--validate-only", action="store_true")
    p.add_argument("--category", default=None)
    p.add_argument("--allow-unmatched", action="store_true")
    p.add_argument("--source-sha", default=None)
    p.add_argument("--compare", type=Path, default=None)
    p.add_argument("--out", type=Path, default=None)
    p.add_argument("--step-summary", default=None)
    args = p.parse_args(argv)

    try:
        cfg = validate_config(load_json_strict(args.budgets))
        if args.validate_only:
            print(json.dumps({"status": "PASS", "rules_count": len(cfg["rules"]), "aggregates_count": len(cfg["aggregates"]), "exceptions_count": len(cfg["exceptions"])}, indent=2))
            return 0
        if not args.paths:
            raise BudgetError("specify at least one artifact path or --validate-only")
        compare_rep = load_json_strict(args.compare) if args.compare else None
        report, rc = evaluate(
            cfg, args.paths, category_override=args.category, allow_unmatched=args.allow_unmatched,
            source_sha=args.source_sha, compare_report=compare_rep,
        )
    except BudgetError as exc:
        print(f"budget check error: {exc}", file=sys.stderr)
        return 2

    for r in report["results"]:
        delta = f" delta={r['delta_vs_base_bytes']:+,}" if r.get("delta_vs_base_bytes") is not None else ""
        warn = f" [target_warning > {r['target_bytes']:,}]" if r.get("target_warning") else ""
        print(f"{r['status']} {r['path']} ({r['category']}.{r['metric']}): {r['actual_bytes']:,} / {r['ceiling_bytes']:,} bytes{warn}{delta}")
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.step_summary:
        _write_step_summary(Path(args.step_summary), report)
    return rc


if __name__ == "__main__":
    sys.exit(main())
