#!/usr/bin/env python3
"""Shared URL sanitization, SSRF/IP checks, and escaping for RFC-0033 scripts.

Implements the RFC-0033 §3.2 global redaction contract and §4.2 IP/host validation
for `scripts/sync_registry.py` and `scripts/probe_registry.py`.
"""

from __future__ import annotations

import hashlib
import ipaddress
import re
from urllib.parse import unquote, urlsplit

REDACTED = "[REDACTED]"
REDACTED_URL = "[REDACTED_URL]"

_CONTROL_RE = re.compile(r"[\x00-\x1f\x7f]")
_TEMPLATE_RE = re.compile(r"\$\{[^{}]*\}|\{[^{}]*\}")
_PATH_CREDS_RE = re.compile(r"(@|%40|://|;|:[^/]*:)")


def strip_control(value: str) -> str:
    """Replace control characters so untrusted strings never forge terminal or Markdown structure."""
    return _CONTROL_RE.sub(" ", value)


def escape_markdown(value: str) -> str:
    """Escape untrusted text for GitHub Markdown (no HTML, scripts, links, or @mentions)."""
    clean = strip_control(value)
    out: list[str] = []
    for ch in clean:
        if ch == "&":
            out.append("&amp;")
        elif ch == "<":
            out.append("&lt;")
        elif ch == ">":
            out.append("&gt;")
        elif ch == "@":
            out.append("&#64;")
        elif ch in "\\`*_{}[]()#+-.!|~":
            out.append("\\" + ch)
        else:
            out.append(ch)
    return "".join(out)


def _lowercase_static_host(host: str) -> str:
    """Lowercase ASCII letters outside `{...}` / `${...}` template placeholders."""
    parts: list[str] = []
    last = 0
    for match in _TEMPLATE_RE.finditer(host):
        parts.append(host[last : match.start()].lower())
        parts.append(match.group(0))
        last = match.end()
    parts.append(host[last:].lower())
    return "".join(parts)


def _strip_default_port(scheme: str, netloc_host_port: str) -> tuple[str, int | None] | None:
    """Split `host[:port]` (possibly templated or IPv6), drop default port, return None on invalid port."""
    if netloc_host_port.startswith("["):
        end = netloc_host_port.find("]")
        if end == -1:
            return None
        host = netloc_host_port[: end + 1]
        rest = netloc_host_port[end + 1 :]
        if not rest:
            return host, None
        if not rest.startswith(":"):
            return None
        port_text = rest[1:]
    elif ":" in netloc_host_port:
        host, port_text = netloc_host_port.rsplit(":", 1)
        if ":" in host:
            return None
    else:
        return netloc_host_port, None

    if not host:
        return None
    if _TEMPLATE_RE.fullmatch(port_text):
        return f"{host}:{port_text}", None
    if not port_text.isdigit():
        return None
    port = int(port_text)
    if port < 1 or port > 65535:
        return None
    default_port = 80 if scheme == "http" else 443
    if port == default_port:
        return host, None
    return f"{host}:{port}", port


def _invalid_sensitive(components: list[str] | None = None) -> dict:
    return {
        "sanitized": REDACTED_URL,
        "canonical": None,
        "scheme": None,
        "host": None,
        "port": None,
        "path": None,
        "origin": None,
        "sensitive": True,
        "templated": False,
        "redacted_components": sorted(set(components or ["url"])),
        "kind": "invalid-sensitive-url",
    }


def _path_has_secret_like_segment(path: str) -> bool:
    if not path:
        return False
    decoded_once = unquote(path)
    decoded_twice = unquote(decoded_once)
    for candidate in (path, decoded_once, decoded_twice):
        if _CONTROL_RE.search(candidate) or "\\" in candidate or any(ch.isspace() for ch in candidate):
            return True
        if _PATH_CREDS_RE.search(candidate):
            return True
        cleaned = _TEMPLATE_RE.sub("", candidate)
        if re.search(r"/[^/]+:[^/]+", cleaned):
            return True
    return False


_LEADING_TEMPLATE_RE = re.compile(r"^(\$\{[A-Za-z0-9_]+\}|\{[A-Za-z0-9_]+\})(?=$|[/?#])")


def _sanitize_templated_url(raw: str) -> dict:
    sep = raw.find("://")
    redacted: list[str] = []
    if sep == -1:
        lead = _LEADING_TEMPLATE_RE.match(raw)
        if not lead:
            return _invalid_sensitive(["url"])
        scheme = None
        host_port_out = lead.group(1)
        port = None
        tail = raw[lead.end() :]
        userinfo_prefix = ""
    else:
        scheme = raw[:sep].lower()
        if scheme not in ("http", "https"):
            return _invalid_sensitive(["url"])
        rest = raw[sep + 3 :]
        slash_idx = rest.find("/")
        q_idx = rest.find("?")
        f_idx = rest.find("#")
        auth_end_candidates = [i for i in (slash_idx, q_idx, f_idx) if i != -1]
        auth_end = min(auth_end_candidates) if auth_end_candidates else len(rest)
        authority = rest[:auth_end]
        tail = rest[auth_end:]

        if "@" in authority:
            _, host_port = authority.rsplit("@", 1)
            redacted.append("userinfo")
            userinfo_prefix = f"{REDACTED}@"
        else:
            host_port = authority
            userinfo_prefix = ""

        normalized_host_port = _strip_default_port(scheme, _lowercase_static_host(host_port))
        if normalized_host_port is None:
            return _invalid_sensitive(["url"])
        host_port_out, port = normalized_host_port

    # Extract path, query, fragment presence from tail without leaking raw values.
    q_pos = tail.find("?")
    f_pos = tail.find("#")
    path_end_candidates = [i for i in (q_pos, f_pos) if i != -1]
    path_end = min(path_end_candidates) if path_end_candidates else len(tail)
    path = tail[:path_end]
    if _path_has_secret_like_segment(path):
        return _invalid_sensitive(["path"])

    has_query = q_pos != -1 and (f_pos == -1 or q_pos < f_pos)
    has_fragment = f_pos != -1

    prefix = f"{scheme}://{userinfo_prefix}" if scheme else userinfo_prefix
    out = f"{prefix}{host_port_out}{path}"
    if has_query:
        redacted.append("query")
        out += f"?{REDACTED}"
    if has_fragment:
        redacted.append("fragment")
        out += f"#{REDACTED}"

    sensitive = bool(redacted)
    return {
        "sanitized": out,
        "canonical": None,
        "scheme": scheme,
        "host": host_port_out.split(":", 1)[0] if not host_port_out.startswith("[") else host_port_out,
        "port": port,
        "path": path,
        "origin": None,
        "sensitive": sensitive,
        "templated": True,
        "redacted_components": sorted(set(redacted)),
        "kind": "invalid-sensitive-url" if sensitive else "templated",
    }


def sanitize_url(raw: object) -> dict:
    """Parse and sanitize a URL per RFC-0033 §3.2 before any report/diff/log object sees it.

    - Normalizes scheme/host case and drops default port (`:80` / `:443`).
    - Preserves exact path and trailing slash (never adds/strips `/v1` or trailing `/`).
    - Replaces userinfo, entire query, and fragment with fixed `[REDACTED]`.
    - Returns `[REDACTED_URL]` when malformed, non-HTTP(S), control-char polluted, or path-credentialed.
    """
    if raw is None:
        return {
            "sanitized": None,
            "canonical": None,
            "scheme": None,
            "host": None,
            "port": None,
            "path": None,
            "origin": None,
            "sensitive": False,
            "templated": False,
            "redacted_components": [],
            "kind": "missing",
        }
    if not isinstance(raw, str):
        return _invalid_sensitive(["url"])
    if not raw or _CONTROL_RE.search(raw) or raw != raw.strip():
        return _invalid_sensitive(["url"])
    # Reject backslash confusion (`https://good.com\@evil.com`) and whitespace anywhere in URL.
    if "\\" in raw or any(ch.isspace() for ch in raw):
        return _invalid_sensitive(["url"])

    if _TEMPLATE_RE.search(raw):
        return _sanitize_templated_url(raw)

    try:
        parsed = urlsplit(raw)
        port = parsed.port
        hostname = parsed.hostname
    except ValueError:
        return _invalid_sensitive(["url"])

    if port is not None and (port < 1 or port > 65535):
        return _invalid_sensitive(["url"])

    scheme = parsed.scheme.lower()
    if scheme not in ("http", "https") or not hostname or not parsed.netloc:
        return _invalid_sensitive(["url"])

    redacted: list[str] = []
    # Check raw `@` before host (covers empty userinfo like `https://@host`).
    sep = raw.find("://")
    after_scheme = raw[sep + 3 :] if sep != -1 else raw
    slash_idx = after_scheme.find("/")
    q_idx = after_scheme.find("?")
    f_idx = after_scheme.find("#")
    auth_end_candidates = [i for i in (slash_idx, q_idx, f_idx) if i != -1]
    raw_authority = after_scheme[: min(auth_end_candidates) if auth_end_candidates else len(after_scheme)]

    has_userinfo = parsed.username is not None or parsed.password is not None or "@" in raw_authority
    has_query = bool(parsed.query) or (q_idx != -1 and (f_idx == -1 or q_idx < f_idx))
    has_fragment = bool(parsed.fragment) or (f_idx != -1)

    if _path_has_secret_like_segment(parsed.path):
        return _invalid_sensitive(["path"])

    try:
        host_ascii = hostname.encode("idna").decode("ascii").lower()
    except UnicodeError:
        return _invalid_sensitive(["url"])
    if not host_ascii or "%" in host_ascii:
        return _invalid_sensitive(["url"])

    # IPv6 hosts must stay bracketed when reconstructed.
    try:
        ip_obj = ipaddress.ip_address(host_ascii)
        host_fmt = f"[{ip_obj.compressed}]" if ip_obj.version == 6 else str(ip_obj)
    except ValueError:
        host_fmt = host_ascii

    default_port = 80 if scheme == "http" else 443
    explicit_nondefault_port = port if (port is not None and port != default_port) else None
    host_port = f"{host_fmt}:{explicit_nondefault_port}" if explicit_nondefault_port else host_fmt
    origin = f"{scheme}://{host_port}"

    userinfo_prefix = ""
    if has_userinfo:
        redacted.append("userinfo")
        userinfo_prefix = f"{REDACTED}@"

    out = f"{scheme}://{userinfo_prefix}{host_port}{parsed.path}"
    if has_query:
        redacted.append("query")
        out += f"?{REDACTED}"
    if has_fragment:
        redacted.append("fragment")
        out += f"#{REDACTED}"

    sensitive = bool(redacted)
    return {
        "sanitized": out,
        "canonical": None if sensitive else out,
        "scheme": scheme,
        "host": host_ascii,
        "port": explicit_nondefault_port or default_port,
        "path": parsed.path,
        "origin": origin,
        "sensitive": sensitive,
        "templated": False,
        "redacted_components": sorted(set(redacted)),
        "kind": "invalid-sensitive-url" if sensitive else "static",
    }


_CGNAT = ipaddress.ip_network("100.64.0.0/10")
_BENCHMARK_V4 = ipaddress.ip_network("198.18.0.0/15")
_TEST_NETS_V4 = (
    ipaddress.ip_network("192.0.2.0/24"),
    ipaddress.ip_network("198.51.100.0/24"),
    ipaddress.ip_network("203.0.113.0/24"),
)
_IETF_PROT_V4 = ipaddress.ip_network("192.0.0.0/24")
_NAT64_WELL_KNOWN = ipaddress.ip_network("64:ff9b::/96")
_NAT64_LOCAL = ipaddress.ip_network("64:ff9b:1::/48")
_DISCARD_V6 = ipaddress.ip_network("100::/64")
_DOC_V6 = ipaddress.ip_network("2001:db8::/32")
_BENCH_V6 = ipaddress.ip_network("2001:2::/48")


def is_public_ip(raw_ip: str) -> bool:
    """Return True iff `raw_ip` is a globally routable unicast IP (RFC-0033 §4.2).

    Explicitly rejects loopback, private, link-local, metadata, CGNAT, multicast,
    unspecified, reserved, IPv4-mapped IPv6, 6to4/Teredo/NAT64 transition, and scoped IDs.
    """
    if not raw_ip or "%" in raw_ip:
        return False
    try:
        ip = ipaddress.ip_address(raw_ip)
    except ValueError:
        return False
    if (
        ip.is_private
        or ip.is_loopback
        or ip.is_link_local
        or ip.is_multicast
        or ip.is_reserved
        or ip.is_unspecified
        or not ip.is_global
    ):
        return False
    if isinstance(ip, ipaddress.IPv4Address):
        if ip in _CGNAT or ip in _BENCHMARK_V4 or ip in _IETF_PROT_V4:
            return False
        if any(ip in net for net in _TEST_NETS_V4):
            return False
        return True
    # IPv6 checks: block site-local, mapped IPv4, compatible IPv4 (::/96), transition & special ranges.
    if ip.is_site_local or ip.ipv4_mapped is not None or ip.sixtofour is not None or ip.teredo is not None:
        return False
    if int(ip) < (1 << 32):
        return False
    if ip in _NAT64_WELL_KNOWN or ip in _NAT64_LOCAL or ip in _DISCARD_V6 or ip in _DOC_V6 or ip in _BENCH_V6:
        return False
    return True


def is_forbidden_hostname(hostname: str) -> bool:
    """Return True for localhost / internal / link-local pseudo-TLDs before DNS lookup."""
    host = hostname.rstrip(".").lower()
    if not host:
        return True
    if host == "localhost" or host.endswith(".localhost"):
        return True
    if host.endswith(".local") or host.endswith(".internal") or host.endswith(".home.arpa"):
        return True
    return False


def parse_json_strict(text: str) -> object:
    """Parse JSON while rejecting duplicate object keys."""
    import json

    def _no_dups(pairs: list[tuple[str, object]]) -> dict[str, object]:
        out: dict[str, object] = {}
        for k, v in pairs:
            if k in out:
                raise ValueError(f"duplicate JSON key {strip_control(str(k))[:80]!r}")
            out[k] = v
        return out

    return json.loads(text, object_pairs_hook=_no_dups)


def sha256_canonical_json(obj: object) -> str:
    """Compute SHA-256 over deterministic UTF-8 compact JSON."""
    import json

    raw = json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def get_git_commit(repo_path: object) -> str | None:
    """Return the HEAD commit SHA for `repo_path` if it is a git checkout, else None."""
    import subprocess

    try:
        res = subprocess.run(
            ["git", "-C", str(repo_path), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    sha = res.stdout.strip()
    if res.returncode == 0 and sha and all(c in "0123456789abcdefABCDEF" for c in sha):
        return sha
    return None


def utc_now_iso() -> str:
    """Return current UTC timestamp in RFC 3339 format."""
    from datetime import datetime, timezone

    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
