#!/usr/bin/env python3
"""Credential-free, SSRF-hardened reachability probe for approved registry endpoints (RFC-0033 §4).

Usage:
    python3 scripts/probe_registry.py --allowlist registry/probe_allowlist.json --output probe-report.json
"""

from __future__ import annotations

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait as futures_wait
from http.client import HTTPMessage, parse_headers
import io
import json
import math
import socket
import ssl
import sys
import time
from pathlib import Path
from typing import Callable
from urllib.parse import unquote, urljoin

sys.path.insert(0, str(Path(__file__).resolve().parent))
import registry_sanitize as rs  # noqa: E402

SCHEMA_VERSION = 1
ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ALLOWLIST = ROOT / "registry" / "probe_allowlist.json"
DEFAULT_REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"

DEFAULT_TIMEOUT_SECS = 8.0
DEFAULT_CONCURRENCY = 8
DEFAULT_TOTAL_BUDGET_SECS = 90.0
DEFAULT_MAX_HEADER_BYTES = 16 * 1024
DEFAULT_MAX_BODY_BYTES = 256 * 1024
DEFAULT_MAX_HEADERS = 64
DEFAULT_MAX_REDIRECTS = 3


class ProbeInputError(Exception):
    """Raised when allowlist or registry files fail validation."""


class LimitExceededError(Exception):
    """Raised when response headers or body exceed configured positive caps."""


class ProbeTlsError(Exception):
    """Raised on TLS verification/handshake errors."""


class ProbeConnectError(Exception):
    """Raised on TCP connect or HTTP framing failures."""


def _default_resolver(host: str, port: int) -> list[str]:
    infos = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    ips: list[str] = []
    seen: set[str] = set()
    for info in infos:
        sockaddr = info[4]
        ip = sockaddr[0]
        if ip not in seen:
            seen.add(ip)
            ips.append(ip)
    return ips


def _read_status_and_headers(reader: io.BufferedReader, max_header_bytes: int) -> tuple[int, HTTPMessage]:
    raw_lines: list[bytes] = []
    total = 0
    while True:
        line = reader.readline(max_header_bytes - total + 2)
        if not line:
            if not raw_lines:
                raise ProbeConnectError("empty HTTP response")
            break
        total += len(line)
        if total > max_header_bytes or len(raw_lines) > DEFAULT_MAX_HEADERS + 1:
            raise LimitExceededError("response headers exceeded byte/count limit")
        raw_lines.append(line)
        if line in (b"\r\n", b"\n"):
            break

    status_line = raw_lines[0].decode("latin-1").strip()
    parts = status_line.split(None, 2)
    if len(parts) < 2 or not parts[0].startswith("HTTP/") or not parts[1].isascii() or not parts[1].isdigit():
        raise ProbeConnectError("malformed HTTP status line")
    status = int(parts[1])
    if status < 100 or status > 599:
        raise ProbeConnectError("invalid HTTP status code")

    header_bytes = b"".join(raw_lines[1:-1])
    headers = parse_headers(io.BytesIO(header_bytes))
    return status, headers


def _drain_bounded_body(reader: io.BufferedReader, headers: HTTPMessage, max_body_bytes: int) -> None:
    content_length = headers.get("Content-Length")
    if content_length is not None:
        try:
            length = int(content_length.strip())
        except ValueError as exc:
            raise ProbeConnectError("invalid Content-Length header") from exc
        if length < 0:
            raise ProbeConnectError("negative Content-Length header")
        if length > max_body_bytes:
            raise LimitExceededError("response Content-Length exceeded limit")
        remaining = length
        while remaining > 0:
            chunk = reader.read(min(remaining, 16384))
            if not chunk:
                break
            remaining -= len(chunk)
        return

    # No Content-Length: read bounded chunks, never retaining payload or Set-Cookie.
    seen = 0
    while True:
        chunk = reader.read(min(16384, max_body_bytes - seen + 1))
        if not chunk:
            break
        seen += len(chunk)
        if seen > max_body_bytes:
            raise LimitExceededError("response body exceeded limit")


def _default_http_exchange(
    ip: str,
    port: int,
    scheme: str,
    host: str,
    path: str,
    headers: dict[str, str],
    timeout: float,
    max_header_bytes: int,
    max_body_bytes: int,
) -> tuple[int, dict[str, str]]:
    """Connect directly to pre-validated IP with SNI, ignoring proxy env and .netrc."""
    raw_sock = None
    sock = None
    try:
        try:
            raw_sock = socket.create_connection((ip, port), timeout=timeout)
            raw_sock.settimeout(timeout)
        except (TimeoutError, socket.timeout) as exc:
            raise exc
        except OSError as exc:
            raise ProbeConnectError(f"connect failed: {type(exc).__name__}") from exc

        if scheme == "https":
            ctx = ssl.create_default_context()
            try:
                sock = ctx.wrap_socket(raw_sock, server_hostname=host)
            except (TimeoutError, socket.timeout) as exc:
                raise exc
            except ssl.SSLError as exc:
                raise ProbeTlsError(f"tls error: {type(exc).__name__}") from exc
            except OSError as exc:
                raise ProbeConnectError(f"tls io error: {type(exc).__name__}") from exc
        else:
            sock = raw_sock

        req_lines = [f"GET {path} HTTP/1.1"]
        for name, value in headers.items():
            req_lines.append(f"{name}: {value}")
        req_lines.append("")
        req_lines.append("")
        try:
            payload = "\r\n".join(req_lines).encode("ascii")
        except UnicodeEncodeError as exc:
            raise ProbeConnectError("non-ASCII HTTP request target") from exc

        try:
            sock.sendall(payload)
            reader = sock.makefile("rb")
            try:
                status, resp_headers = _read_status_and_headers(reader, max_header_bytes)
                # Only Location is inspected for redirect handling; Set-Cookie & arbitrary headers ignored.
                location = resp_headers.get("Location")
                if status not in (204, 304) and not (100 <= status < 200):
                    _drain_bounded_body(reader, resp_headers, max_body_bytes)
                out_headers = {"location": location} if location is not None else {}
                return status, out_headers
            finally:
                reader.close()
        except ssl.SSLError as exc:
            raise ProbeTlsError(f"tls io error: {type(exc).__name__}") from exc
    finally:
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass
        elif raw_sock is not None:
            try:
                raw_sock.close()
            except OSError:
                pass


def load_allowlist(path: Path) -> list[dict]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ProbeInputError(f"{path}: failed to load allowlist: {exc}") from exc
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise ProbeInputError(f"{path}: expected schema_version == 1")
    if set(data.keys()) - {"schema_version", "entries"}:
        raise ProbeInputError(f"{path}: unknown top-level allowlist keys")
    raw_entries = data.get("entries")
    if not isinstance(raw_entries, list):
        raise ProbeInputError(f"{path}: `entries` must be a list")
    entries: list[dict] = []
    seen_ids: set[str] = set()
    allowed_keys = {"id", "origin", "base_path", "source"}
    for idx, item in enumerate(raw_entries):
        if not isinstance(item, dict) or set(item.keys()) != allowed_keys:
            raise ProbeInputError(f"{path}: entry #{idx} must have exact keys {sorted(allowed_keys)}")
        for k in allowed_keys:
            if not isinstance(item[k], str) or (k != "base_path" and not item[k]):
                raise ProbeInputError(f"{path}: entry #{idx} invalid `{k}`")
        if item["id"] in seen_ids:
            raise ProbeInputError(f"{path}: duplicate allowlist id {item['id']!r}")
        seen_ids.add(item["id"])
        entries.append(item)
    return entries


def load_registry_urls(path: Path | None) -> dict[str, str]:
    if path is None:
        return {}
    try:
        rows = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ProbeInputError(f"{path}: failed to read registry: {exc}") from exc
    if not isinstance(rows, list):
        raise ProbeInputError(f"{path}: registry must be JSON array")
    urls: dict[str, str] = {}
    for row in rows:
        if isinstance(row, dict) and isinstance(row.get("name"), str) and isinstance(row.get("base_url"), str):
            urls[row["name"]] = row["base_url"]
    return urls


def classify_http_status(status: int) -> str:
    if 200 <= status <= 299:
        return "reachable"
    if status in (401, 403):
        return "reachable-auth-or-policy"
    if status in (404, 405):
        return "reachable-path-unsupported"
    if status == 429:
        return "reachable-rate-limited"
    if 400 <= status <= 499:
        return "reachable-client-error"
    if 500 <= status <= 599:
        return "server-error"
    return "redirect-unverified"


def _host_is_literal_ip(host: str) -> bool:
    import ipaddress

    try:
        ipaddress.ip_address(host.strip("[]"))
        return True
    except ValueError:
        return False


def _validate_and_resolve(
    url_info: dict,
    resolver: Callable[[str, int], list[str]],
) -> tuple[str | None, list[str]]:
    """Return `(block_reason_or_none, validated_ips)`."""
    if url_info["sensitive"]:
        return "blocked-sensitive-url", []
    if url_info["templated"]:
        return "skipped-template", []
    if url_info["kind"] != "static" or not url_info["host"] or not url_info["scheme"]:
        return "blocked-invalid-url", []

    host = url_info["host"]
    port = int(url_info["port"])

    if _host_is_literal_ip(host):
        clean_ip = host.strip("[]")
        if not rs.is_public_ip(clean_ip):
            return "blocked-non-public-ip", []
        return None, [clean_ip]

    if rs.is_forbidden_hostname(host):
        return "blocked-non-public-ip", []

    try:
        resolved = resolver(host, port)
    except OSError:
        return "dns-error", []

    if not resolved:
        return "dns-error", []

    for ip in resolved:
        if not rs.is_public_ip(ip):
            return "blocked-non-public-ip", []
    return None, resolved


def _is_target_approved(
    url_info: dict,
    initial_origin: str,
    initial_base_path: str,
) -> bool:
    """Verify redirect hop target stays on the approved initial origin and base `/models` path."""
    if url_info["kind"] != "static" or url_info["sensitive"]:
        return False
    if url_info["origin"] != initial_origin:
        return False
    path = url_info["path"] or "/"
    lowered_path = path.lower()
    if "%2f" in lowered_path or "%5c" in lowered_path or "\\" in path:
        return False
    decoded_segments = unquote(path).split("/")
    if any(seg in (".", "..") for seg in decoded_segments):
        return False
    norm_base = initial_base_path.rstrip("/")
    expected_prefix = (norm_base + "/models") if norm_base else "/models"
    return path == expected_prefix or path.startswith(expected_prefix + "/")


def probe_entry(
    entry: dict,
    *,
    registry_urls: dict[str, str],
    resolver: Callable[[str, int], list[str]],
    http_exchange: Callable[..., tuple[int, dict[str, str]]],
    timeout: float,
    max_header_bytes: int,
    max_body_bytes: int,
    max_redirects: int,
) -> dict:
    pid = entry["id"]
    started = time.monotonic()

    allow_raw = entry["origin"] + entry["base_path"]
    allow_info = rs.sanitize_url(allow_raw)
    if allow_info["sensitive"]:
        return {
            "id": pid,
            "target": allow_info["sanitized"],
            "outcome": "blocked-sensitive-url",
            "http_status": None,
            "redirects": 0,
            "latency_ms": 0,
        }
    if allow_info["templated"]:
        return {
            "id": pid,
            "target": allow_info["sanitized"],
            "outcome": "skipped-template",
            "http_status": None,
            "redirects": 0,
            "latency_ms": 0,
        }

    # Verify registry's current base_url matches approved allowlist entry if registry supplied.
    if registry_urls:
        local_raw = registry_urls.get(pid)
        if local_raw is None:
            return {
                "id": pid,
                "target": allow_info["sanitized"],
                "outcome": "blocked-unapproved",
                "http_status": None,
                "redirects": 0,
                "latency_ms": 0,
            }
        local_info = rs.sanitize_url(local_raw)
        if local_info["sensitive"]:
            return {
                "id": pid,
                "target": local_info["sanitized"],
                "outcome": "blocked-sensitive-url",
                "http_status": None,
                "redirects": 0,
                "latency_ms": 0,
            }
        if local_info["templated"]:
            return {
                "id": pid,
                "target": local_info["sanitized"],
                "outcome": "skipped-template",
                "http_status": None,
                "redirects": 0,
                "latency_ms": 0,
            }
        if local_info["canonical"] != allow_info["canonical"]:
            return {
                "id": pid,
                "target": local_info["sanitized"],
                "outcome": "blocked-unapproved",
                "http_status": None,
                "redirects": 0,
                "latency_ms": 0,
            }

    probe_raw = allow_raw.rstrip("/") + "/models"
    current_info = rs.sanitize_url(probe_raw)
    current_url = current_info["sanitized"]
    visited = {current_url}
    redirect_count = 0

    while True:
        block_reason, ips = _validate_and_resolve(current_info, resolver)
        if block_reason is not None:
            return {
                "id": pid,
                "target": current_info["sanitized"],
                "outcome": block_reason,
                "http_status": None,
                "redirects": redirect_count,
                "latency_ms": int((time.monotonic() - started) * 1000),
            }

        scheme = current_info["scheme"]
        host = current_info["host"]
        port = int(current_info["port"])
        path = current_info["path"] or "/"
        default_port = 80 if scheme == "http" else 443
        host_fmt = f"[{host}]" if ":" in host else host
        host_header = f"{host_fmt}:{port}" if port != default_port else host_fmt

        fixed_headers = {
            "Host": host_header,
            "User-Agent": "aimux-registry-probe/1.0",
            "Accept": "application/json",
            "Accept-Encoding": "identity",
            "Connection": "close",
        }

        try:
            status, resp_headers = http_exchange(
                ips[0],
                port,
                scheme,
                host,
                path,
                fixed_headers,
                timeout,
                max_header_bytes,
                max_body_bytes,
            )
        except (TimeoutError, socket.timeout):
            outcome = "timeout"
            http_status = None
            break
        except LimitExceededError:
            outcome = "limit-exceeded"
            http_status = None
            break
        except ProbeTlsError:
            outcome = "tls-error"
            http_status = None
            break
        except (ProbeConnectError, OSError):
            outcome = "connect-error"
            http_status = None
            break
        except Exception:
            outcome = "connect-error"
            http_status = None
            break

        if status in (301, 302, 303, 307, 308):
            location = resp_headers.get("location")
            if not location or redirect_count >= max_redirects:
                outcome = "redirect-unverified"
                http_status = status
                break
            try:
                next_raw = urljoin(current_url, location)
            except ValueError:
                return {
                    "id": pid,
                    "target": current_info["sanitized"],
                    "final_target": rs.REDACTED_URL,
                    "outcome": "blocked-invalid-url",
                    "http_status": status,
                    "redirects": redirect_count,
                    "latency_ms": int((time.monotonic() - started) * 1000),
                }
            next_info = rs.sanitize_url(next_raw)
            if next_info["sensitive"]:
                return {
                    "id": pid,
                    "target": current_info["sanitized"],
                    "final_target": next_info["sanitized"],
                    "outcome": "blocked-sensitive-url",
                    "http_status": status,
                    "redirects": redirect_count,
                    "latency_ms": int((time.monotonic() - started) * 1000),
                }
            if scheme == "https" and next_info["scheme"] == "http":
                return {
                    "id": pid,
                    "target": current_info["sanitized"],
                    "final_target": next_info["sanitized"],
                    "outcome": "blocked-https-downgrade",
                    "http_status": status,
                    "redirects": redirect_count,
                    "latency_ms": int((time.monotonic() - started) * 1000),
                }
            if not _is_target_approved(
                next_info, allow_info["origin"], entry["base_path"]
            ):
                return {
                    "id": pid,
                    "target": current_info["sanitized"],
                    "final_target": next_info["sanitized"],
                    "outcome": "blocked-cross-origin-redirect",
                    "http_status": status,
                    "redirects": redirect_count,
                    "latency_ms": int((time.monotonic() - started) * 1000),
                }
            if next_info["sanitized"] in visited:
                return {
                    "id": pid,
                    "target": current_info["sanitized"],
                    "final_target": next_info["sanitized"],
                    "outcome": "blocked-redirect-loop",
                    "http_status": status,
                    "redirects": redirect_count,
                    "latency_ms": int((time.monotonic() - started) * 1000),
                }
            visited.add(next_info["sanitized"])
            redirect_count += 1
            current_info = next_info
            current_url = next_info["sanitized"]
            continue

        outcome = classify_http_status(status)
        http_status = status
        break

    return {
        "id": pid,
        "target": current_info["sanitized"],
        "outcome": outcome,
        "http_status": http_status,
        "redirects": redirect_count,
        "latency_ms": int((time.monotonic() - started) * 1000),
    }


def _not_run_result(entry: dict) -> tuple[dict, bool]:
    rem_info = rs.sanitize_url(entry["origin"] + entry["base_path"].rstrip("/") + "/models")
    res = {
        "id": entry["id"],
        "target": rem_info["sanitized"],
        "outcome": "not-run",
        "http_status": None,
        "redirects": 0,
        "latency_ms": 0,
    }
    return res, bool(rem_info["sensitive"])


def _record_redacted(res: dict) -> bool:
    for field in ("target", "final_target"):
        val = res.get(field)
        if val == rs.REDACTED_URL or rs.REDACTED in str(val or ""):
            return True
    return False


def run_probe(
    entries: list[dict],
    *,
    registry_urls: dict[str, str],
    resolver: Callable[[str, int], list[str]] = _default_resolver,
    http_exchange: Callable[..., tuple[int, dict[str, str]]] = _default_http_exchange,
    timeout: float = DEFAULT_TIMEOUT_SECS,
    concurrency: int = DEFAULT_CONCURRENCY,
    total_budget: float = DEFAULT_TOTAL_BUDGET_SECS,
    max_header_bytes: int = DEFAULT_MAX_HEADER_BYTES,
    max_body_bytes: int = DEFAULT_MAX_BODY_BYTES,
    max_redirects: int = DEFAULT_MAX_REDIRECTS,
) -> dict:
    started = time.monotonic()
    deadline = started + total_budget
    sorted_entries = sorted(entries, key=lambda e: e["id"])
    results: list[dict] = []
    any_redacted = False
    budget_exhausted = False

    # Process in bounded waves and enforce total_budget inside each wave.
    pool = ThreadPoolExecutor(max_workers=concurrency)
    try:
        idx = 0
        n = len(sorted_entries)
        while idx < n:
            if time.monotonic() >= deadline:
                budget_exhausted = True
                for rem in sorted_entries[idx:]:
                    res, redacted = _not_run_result(rem)
                    any_redacted = any_redacted or redacted
                    results.append(res)
                break
            batch = sorted_entries[idx : idx + concurrency]
            idx += concurrency
            pending = {
                pool.submit(
                    probe_entry,
                    entry,
                    registry_urls=registry_urls,
                    resolver=resolver,
                    http_exchange=http_exchange,
                    timeout=timeout,
                    max_header_bytes=max_header_bytes,
                    max_body_bytes=max_body_bytes,
                    max_redirects=max_redirects,
                ): entry
                for entry in batch
            }
            while pending:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    budget_exhausted = True
                    break
                done, _ = futures_wait(
                    pending.keys(),
                    timeout=remaining,
                    return_when=FIRST_COMPLETED,
                )
                if not done:
                    budget_exhausted = True
                    break
                for fut in done:
                    entry = pending.pop(fut)
                    try:
                        res = fut.result()
                    except Exception:
                        res, _ = _not_run_result(entry)
                        res["outcome"] = "connect-error"
                    if _record_redacted(res):
                        any_redacted = True
                    results.append(res)

            if budget_exhausted:
                for fut, entry in pending.items():
                    fut.cancel()
                    res, redacted = _not_run_result(entry)
                    any_redacted = any_redacted or redacted
                    results.append(res)
                for rem in sorted_entries[idx:]:
                    res, redacted = _not_run_result(rem)
                    any_redacted = any_redacted or redacted
                    results.append(res)
                break
    finally:
        pool.shutdown(wait=not budget_exhausted, cancel_futures=True)

    results.sort(key=lambda r: r["id"])
    counts: dict[str, int] = {}
    for r in results:
        counts[r["outcome"]] = counts.get(r["outcome"], 0) + 1

    sanitized_entries = [
        {
            "id": e["id"],
            "endpoint": rs.sanitize_url(e["origin"] + e["base_path"])["sanitized"],
            "source": e["source"],
        }
        for e in sorted_entries
    ]
    input_hash_kind = "sanitized" if any_redacted else "original"
    input_hash = rs.sha256_canonical_json(sanitized_entries)

    return {
        "schema_version": SCHEMA_VERSION,
        "mode": "probe",
        "repo_commit": rs.get_git_commit(ROOT),
        "collected_at": rs.utc_now_iso(),
        "policy": {
            "credential_free": True,
            "ignore_env_proxies": True,
            "ignore_netrc": True,
            "tls_verify": True,
            "timeout_secs": timeout,
            "concurrency": concurrency,
            "total_budget_secs": total_budget,
            "max_header_bytes": max_header_bytes,
            "max_body_bytes": max_body_bytes,
            "max_redirects": max_redirects,
        },
        "input_hash_kind": input_hash_kind,
        "input_hash": input_hash,
        "duration_ms": int((time.monotonic() - started) * 1000),
        "total": len(results),
        "counts": dict(sorted(counts.items())),
        "results": results,
    }


def render_markdown(report: dict) -> str:
    commit = report.get("repo_commit") or "unknown"
    meta_parts = [
        f"**Total**: `{report['total']}`",
        f"**Commit**: `{rs.escape_markdown(str(commit)[:12])}`",
        f"**Hash ({report['input_hash_kind']})**: `{report['input_hash'][:12]}`",
    ]
    if report.get("collected_at"):
        meta_parts.append(f"**Collected**: `{rs.escape_markdown(str(report['collected_at']))}`")
    if report.get("duration_ms") is not None:
        meta_parts.append(f"**Duration**: `{report['duration_ms']}ms`")
    lines = [
        "### Registry Credential-Free Probe Report (RFC-0033)",
        "",
        "- " + " · ".join(meta_parts),
        "",
        "| Outcome | Count |",
        "|---|---:|",
    ]
    for k, v in sorted(report["counts"].items()):
        lines.append(f"| {rs.escape_markdown(k)} | {v} |")
    lines.append("")
    issues = [
        r
        for r in report["results"]
        if not r["outcome"].startswith("reachable")
    ]
    if issues:
        lines.extend(["#### Non-Reachable / Blocked / Failed Targets", "", "| Provider | Target | Outcome | Status |", "|---|---|---|---:|"])
        for r in issues[:80]:
            st = str(r["http_status"]) if r["http_status"] is not None else "-"
            lines.append(
                f"| {rs.escape_markdown(r['id'])} | {rs.escape_markdown(str(r['target']))} "
                f"| {rs.escape_markdown(r['outcome'])} | {st} |"
            )
        if len(issues) > 80:
            lines.append(f"\n_... and {len(issues) - 80} more omitted in summary._\n")
    return "\n".join(lines).rstrip() + "\n"


def _positive_int(val: str) -> int:
    n = int(val)
    if n <= 0:
        raise argparse.ArgumentTypeError("must be positive integer")
    return n


def _nonneg_int(val: str) -> int:
    n = int(val)
    if n < 0:
        raise argparse.ArgumentTypeError("must be non-negative integer")
    return n


def _positive_float(val: str) -> float:
    x = float(val)
    if not math.isfinite(x) or x <= 0:
        raise argparse.ArgumentTypeError("must be finite positive number")
    return x


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="RFC-0033 credential-free reachability probe")
    p.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST)
    p.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    p.add_argument("--no-registry-check", action="store_true", help="Probe allowlist directly without comparing local registry file")
    p.add_argument("--output", type=Path, help="Write JSON report to PATH (stdout if omitted)")
    p.add_argument("--markdown-output", type=Path, help="Write Markdown report view to PATH")
    p.add_argument("--timeout", type=_positive_float, default=DEFAULT_TIMEOUT_SECS)
    p.add_argument("--concurrency", type=_positive_int, default=DEFAULT_CONCURRENCY)
    p.add_argument("--total-budget", type=_positive_float, default=DEFAULT_TOTAL_BUDGET_SECS)
    p.add_argument("--max-header-bytes", type=_positive_int, default=DEFAULT_MAX_HEADER_BYTES)
    p.add_argument("--max-body-bytes", type=_positive_int, default=DEFAULT_MAX_BODY_BYTES)
    p.add_argument("--max-redirects", type=_nonneg_int, default=DEFAULT_MAX_REDIRECTS)
    return p


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else 2

    try:
        entries = load_allowlist(args.allowlist)
        reg_urls = {} if args.no_registry_check else load_registry_urls(args.registry)
        report = run_probe(
            entries,
            registry_urls=reg_urls,
            timeout=args.timeout,
            concurrency=args.concurrency,
            total_budget=args.total_budget,
            max_header_bytes=args.max_header_bytes,
            max_body_bytes=args.max_body_bytes,
            max_redirects=args.max_redirects,
        )
        rendered = json.dumps(report, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
        else:
            sys.stdout.write(rendered)
        if args.markdown_output:
            args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
            args.markdown_output.write_text(render_markdown(report), encoding="utf-8")
        return 0
    except ProbeInputError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"io error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
