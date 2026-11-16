#!/usr/bin/env python3
"""Sync, diff, and offline-check `aimux-providers/src/provider_registry.json` (RFC-0033).

Usage examples:
    python3 scripts/sync_registry.py --report --source models-dev --snapshot registry/models_dev_snapshot.json --output report.json
    python3 scripts/sync_registry.py --apply --source models-dev --snapshot INPUT.json --output report.json
    python3 scripts/sync_registry.py --check --source models-dev --snapshot registry/models_dev_snapshot.json
    python3 scripts/sync_registry.py --report --source models-dev --fetch --output report.json
    python3 scripts/sync_registry.py --report --source litellm --checkout PATH --output report.json
"""

from __future__ import annotations

import argparse
import gzip
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
import registry_sanitize as rs  # noqa: E402

SCHEMA_VERSION = 1
ADAPTER_VERSION = "1"
DEFAULT_MODELS_DEV_URL = "https://models.dev/api.json"
DEFAULT_TIMEOUT_SECS = 60

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"
DEFAULT_ALIASES = ROOT / "aimux-providers" / "src" / "provider_aliases.json"
DEFAULT_RECONCILE = ROOT / "registry" / "reconcile.json"
DEFAULT_PACKAGES_RS = ROOT / "aimux-providers" / "src" / "default_providers.rs"

_IDENTIFIER_RE = re.compile(r"^[a-z][a-z0-9_]*$")
_ENV_NAME_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
_PLACEHOLDER_RE = re.compile(r"\{([^{}]*)\}")
_UPSTREAM_TEMPLATE_VAR_RE = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")

_ROW_ALLOWED_KEYS = {
    "name",
    "display",
    "base_url",
    "env_var",
    "auth",
    "base_url_env",
    "params",
    "profile",
}
_ROW_OPTIONAL_NONNULL_STRINGS = ("env_var", "auth", "base_url_env")
_PARAM_ALLOWED_KEYS = {"name", "env", "default", "derive"}
_DERIVE_ALLOWED_KEYS = {"from", "map", "otherwise"}
_PROFILE_ALLOWED_KEYS = {
    "max_tokens_key",
    "convert_usage",
    "user_agent",
    "structured_output_models",
}


class RegistryInputError(Exception):
    """Raised when an input file, snapshot, or upstream schema violates expectations."""


def load_json_file(path: Path, label: str) -> object:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise RegistryInputError(f"{label}: failed to read: {exc}") from exc
    try:
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise RegistryInputError(f"{label}: invalid JSON: {exc}") from exc


def load_native_packages(packages_rs: Path) -> set[str]:
    if not packages_rs.exists():
        return set()
    try:
        src = packages_rs.read_text(encoding="utf-8")
    except OSError as exc:
        raise RegistryInputError(f"packages source: failed to read: {exc}") from exc
    match = re.search(r"const\s+PACKAGES\s*:\s*\[&str;\s*\d+\]\s*=\s*\[(.*?)\];", src, re.S)
    if not match:
        raise RegistryInputError(f"{packages_rs}: missing PACKAGES list")
    return set(re.findall(r'"([^"]+)"', match.group(1)))


def _validate_registry_row(idx: int, row: object, seen_names: set[str]) -> tuple[dict, str]:
    if not isinstance(row, dict):
        raise RegistryInputError(f"registry row #{idx}: expected JSON object")
    unknown = set(row.keys()) - _ROW_ALLOWED_KEYS
    if unknown:
        raise RegistryInputError(f"registry row #{idx}: unknown fields {sorted(unknown)}")
    for req in ("name", "display", "base_url"):
        val = row.get(req)
        if not isinstance(val, str) or not val:
            raise RegistryInputError(f"registry row #{idx}: missing or invalid `{req}`")
    name = row["name"]
    if not _IDENTIFIER_RE.match(name):
        raise RegistryInputError(f"registry row '{name}': invalid name")
    if name in seen_names:
        raise RegistryInputError(f"registry row '{name}': duplicate registry name")
    seen_names.add(name)

    for key in _ROW_OPTIONAL_NONNULL_STRINGS:
        if key in row:
            val = row[key]
            if val is None or not isinstance(val, str):
                raise RegistryInputError(f"registry row '{name}': `{key}` must be a non-null string")

    auth = row.get("auth", "api_key")
    if auth not in ("api_key", "none"):
        raise RegistryInputError(f"registry row '{name}': unknown auth {auth!r}")

    env_var = row.get("env_var")
    if auth == "api_key":
        if not env_var:
            raise RegistryInputError(f"registry row '{name}': `api_key` requires non-empty `env_var`")
        if not _ENV_NAME_RE.match(env_var):
            raise RegistryInputError(f"registry row '{name}': invalid `env_var` {env_var!r}")
        key_env_names = [env_var]
        key_mode = "required"
    else:
        if env_var is not None:
            if not _ENV_NAME_RE.match(env_var):
                raise RegistryInputError(f"registry row '{name}': invalid optional `env_var` {env_var!r}")
            key_env_names = [env_var]
            key_mode = "optional"
        else:
            key_env_names = []
            key_mode = "none"

    config_envs: set[str] = set()
    base_url_env = row.get("base_url_env")
    if base_url_env is not None:
        if not _ENV_NAME_RE.match(base_url_env):
            raise RegistryInputError(f"registry row '{name}': invalid `base_url_env` {base_url_env!r}")
        config_envs.add(base_url_env)

    params = row.get("params", [])
    if not isinstance(params, list):
        raise RegistryInputError(f"registry row '{name}': `params` must be an array")
    param_names: set[str] = set()
    for param in params:
        if not isinstance(param, dict):
            raise RegistryInputError(f"registry row '{name}': param must be object")
        if set(param.keys()) - _PARAM_ALLOWED_KEYS:
            raise RegistryInputError(f"registry row '{name}': unknown param field")
        pname = param.get("name")
        if not isinstance(pname, str) or not _IDENTIFIER_RE.match(pname) or pname in param_names:
            raise RegistryInputError(f"registry row '{name}': invalid/duplicate param name {pname!r}")
        param_names.add(pname)
        if "env" in param:
            penv = param["env"]
            if not isinstance(penv, list) or not all(isinstance(v, str) and _ENV_NAME_RE.match(v) for v in penv):
                raise RegistryInputError(f"registry row '{name}': invalid param `{pname}` env list")
            config_envs.update(penv)
        if "default" in param and (param["default"] is None or not isinstance(param["default"], str)):
            raise RegistryInputError(f"registry row '{name}': param `{pname}` default must be string")
        if "derive" in param:
            derive = param["derive"]
            if not isinstance(derive, dict) or set(derive.keys()) - _DERIVE_ALLOWED_KEYS:
                raise RegistryInputError(f"registry row '{name}': invalid derive object in `{pname}`")
            if "env" in param or "default" in param:
                raise RegistryInputError(f"registry row '{name}': derived param `{pname}` has env/default")

    profile = row.get("profile", {})
    if not isinstance(profile, dict) or (set(profile.keys()) - _PROFILE_ALLOWED_KEYS):
        raise RegistryInputError(f"registry row '{name}': invalid `profile` object")
    for key, val in profile.items():
        if val is None:
            raise RegistryInputError(f"registry row '{name}': profile.`{key}` cannot be null")

    raw_base_url = row["base_url"]
    if not raw_base_url.isascii() or any(ch in raw_base_url for ch in ("<", ">", "$")):
        raise RegistryInputError(f"registry row '{name}': invalid characters in base_url")
    if set(_PLACEHOLDER_RE.findall(raw_base_url)) != param_names:
        raise RegistryInputError(f"registry row '{name}': URL placeholders differ from params")

    entry = {
        "id": name,
        "source_ref": f"aimux-providers/src/provider_registry.json#{name}",
        "auth": auth,
        "key_mode": key_mode,
        "key_env_names": sorted(set(key_env_names)),
        "config_env_names": sorted(config_envs),
        "url_info": rs.sanitize_url(raw_base_url),
        "raw_base_url": raw_base_url,  # Only kept inside memory; stripped before output.
        "raw_env_var": env_var,
        "note": None,
        "status": None,
    }
    return entry, name


def load_local_registry(registry_path: Path) -> tuple[list[dict], dict[str, dict]]:
    raw = load_json_file(registry_path, str(registry_path))
    if not isinstance(raw, list):
        raise RegistryInputError(f"{registry_path}: expected JSON array")
    seen: set[str] = set()
    by_id: dict[str, dict] = {}
    for idx, row in enumerate(raw):
        entry, name = _validate_registry_row(idx, row, seen)
        by_id[name] = entry
    return raw, by_id


def load_aliases(aliases_path: Path, valid_targets: set[str]) -> dict[str, str]:
    raw = load_json_file(aliases_path, str(aliases_path))
    if not isinstance(raw, dict):
        raise RegistryInputError(f"{aliases_path}: expected JSON object")
    aliases: dict[str, str] = {}
    targets_seen: set[str] = set()
    for src, dst in raw.items():
        if not isinstance(src, str) or not src or not isinstance(dst, str) or not dst:
            raise RegistryInputError(f"{aliases_path}: invalid alias entry {src!r} -> {dst!r}")
        if src in valid_targets:
            raise RegistryInputError(f"{aliases_path}: alias source {src!r} collides with built-in provider id")
        if dst not in valid_targets:
            raise RegistryInputError(f"{aliases_path}: alias target {dst!r} does not exist")
        if dst in targets_seen:
            raise RegistryInputError(f"{aliases_path}: duplicate alias target {dst!r}")
        targets_seen.add(dst)
        aliases[src] = dst
    return aliases


def load_reconcile(reconcile_path: Path | None, valid_ids: set[str]) -> dict:
    if reconcile_path is None or not reconcile_path.exists():
        return {"schema_version": 1, "overrides": [], "entries": {}}
    raw = load_json_file(reconcile_path, str(reconcile_path))
    if not isinstance(raw, dict):
        raise RegistryInputError(f"{reconcile_path}: expected JSON object")
    allowed_top = {"schema_version", "overrides", "entries"}
    if set(raw.keys()) - allowed_top:
        raise RegistryInputError(f"{reconcile_path}: unknown top-level keys {sorted(set(raw.keys()) - allowed_top)}")
    if raw.get("schema_version") != 1:
        raise RegistryInputError(f"{reconcile_path}: expected schema_version == 1")

    overrides_raw = raw.get("overrides", [])
    if not isinstance(overrides_raw, list):
        raise RegistryInputError(f"{reconcile_path}: `overrides` must be array")
    seen_override_keys: set[tuple[str, str, str]] = set()
    overrides: list[dict] = []
    allowed_override_keys = {"id", "upstream", "field", "local_value", "upstream_value", "reason", "source_url"}
    for idx, item in enumerate(overrides_raw):
        if not isinstance(item, dict) or (set(item.keys()) - allowed_override_keys):
            raise RegistryInputError(f"{reconcile_path}: invalid override item #{idx}")
        for req in ("id", "upstream", "field", "local_value", "upstream_value", "reason"):
            if req not in item:
                raise RegistryInputError(f"{reconcile_path}: override #{idx} missing `{req}`")
        oid = item["id"]
        upstream = item["upstream"]
        field = item["field"]
        reason = item["reason"]
        if not isinstance(oid, str) or oid not in valid_ids:
            raise RegistryInputError(f"{reconcile_path}: override #{idx} references unknown id {oid!r}")
        if upstream not in ("models-dev", "litellm"):
            raise RegistryInputError(f"{reconcile_path}: override #{idx} invalid upstream {upstream!r}")
        if field not in ("base_url", "env_var"):
            raise RegistryInputError(f"{reconcile_path}: override #{idx} invalid field {field!r}")
        if not isinstance(reason, str) or not reason.strip():
            raise RegistryInputError(f"{reconcile_path}: override #{idx} requires non-empty `reason`")
        if field == "base_url":
            for side in ("local_value", "upstream_value"):
                val = item[side]
                if not isinstance(val, str) or rs.sanitize_url(val)["sensitive"]:
                    raise RegistryInputError(
                        f"{reconcile_path}: override #{idx} `{side}` must be a non-sensitive URL string"
                    )
        else:
            for side in ("local_value", "upstream_value"):
                val = item[side]
                if side == "local_value" and val is not None and not isinstance(val, str):
                    raise RegistryInputError(f"{reconcile_path}: override #{idx} invalid local env_var")
                if side == "upstream_value" and not isinstance(val, list) and not isinstance(val, str) and val is not None:
                    raise RegistryInputError(f"{reconcile_path}: override #{idx} invalid upstream_value")
        key = (oid, upstream, field)
        if key in seen_override_keys:
            raise RegistryInputError(f"{reconcile_path}: duplicate override for {key}")
        seen_override_keys.add(key)
        overrides.append(item)

    entries_raw = raw.get("entries", {})
    if not isinstance(entries_raw, dict):
        raise RegistryInputError(f"{reconcile_path}: `entries` must be object")
    allowed_entry_keys = {"note", "status", "source_url"}
    entries: dict[str, dict] = {}
    for eid, meta in entries_raw.items():
        if eid not in valid_ids:
            raise RegistryInputError(f"{reconcile_path}: entry references unknown id {eid!r}")
        if not isinstance(meta, dict) or (set(meta.keys()) - allowed_entry_keys):
            raise RegistryInputError(f"{reconcile_path}: invalid entry metadata for {eid!r}")
        note = meta.get("note")
        status = meta.get("status")
        if not isinstance(note, str) or not note.strip():
            raise RegistryInputError(f"{reconcile_path}: entry {eid!r} must have non-empty `note`")
        if status is not None and status != "unreachable":
            raise RegistryInputError(f"{reconcile_path}: entry {eid!r} invalid status {status!r}")
        entries[eid] = {"note": note, "status": status, "source_url": meta.get("source_url")}

    return {"schema_version": 1, "overrides": overrides, "entries": entries}


def fetch_models_dev(url: str, timeout: int = DEFAULT_TIMEOUT_SECS) -> dict:
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "aimux-registry-sync/1.0",
            "Accept": "application/json",
            "Accept-Encoding": "gzip",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = response.read()
            if response.headers.get("Content-Encoding", "").lower() == "gzip":
                payload = gzip.decompress(payload)
    except (urllib.error.URLError, OSError, gzip.BadGzipFile) as exc:
        raise RegistryInputError(f"models.dev fetch failed: {exc}") from exc
    try:
        data = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RegistryInputError(f"models.dev response invalid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise RegistryInputError("models.dev response must be a JSON object")
    return data


def _unwrap_models_dev_providers(data: object) -> dict:
    if not isinstance(data, dict):
        raise RegistryInputError("models.dev snapshot must be a JSON object")
    if "providers" in data and "schema_version" in data:
        if data.get("schema_version") != 1 or not isinstance(data["providers"], dict):
            raise RegistryInputError("models.dev snapshot wrapper has invalid schema")
        return data["providers"]
    return data


def slim_models_dev_snapshot(data: object, source_url: str = DEFAULT_MODELS_DEV_URL) -> dict:
    providers_in = _unwrap_models_dev_providers(data)
    providers_out: dict[str, dict] = {}
    for key in sorted(providers_in):
        item = providers_in[key]
        if not isinstance(item, dict):
            raise RegistryInputError(f"models.dev provider {key!r}: expected object")
        pid = item.get("id", key)
        if not isinstance(pid, str) or not pid or pid != key:
            raise RegistryInputError(f"models.dev provider {key!r}: invalid `id` {pid!r}")
        entry: dict[str, object] = {"id": pid}
        if "name" in item:
            if item["name"] is not None and not isinstance(item["name"], str):
                raise RegistryInputError(f"models.dev provider {key!r}: `name` must be string")
            if item["name"] is not None:
                entry["name"] = item["name"]
        if "api" in item:
            if item["api"] is not None and not isinstance(item["api"], str):
                raise RegistryInputError(f"models.dev provider {key!r}: `api` must be string")
            if item["api"] is not None:
                entry["api"] = item["api"]
        if "env" in item:
            env = item["env"]
            if env is not None and (not isinstance(env, list) or not all(isinstance(v, str) for v in env)):
                raise RegistryInputError(f"models.dev provider {key!r}: `env` must be string list")
            if env is not None:
                entry["env"] = sorted(set(env))
        if "npm" in item and item["npm"] is not None:
            if not isinstance(item["npm"], str):
                raise RegistryInputError(f"models.dev provider {key!r}: `npm` must be string")
            entry["npm"] = item["npm"]
        providers_out[key] = entry
    return {
        "schema_version": 1,
        "source_url": rs.sanitize_url(source_url)["sanitized"],
        "providers": providers_out,
    }


def parse_models_dev(data: object) -> tuple[list[dict], list[dict]]:
    """Parse models.dev JSON, strictly checking compared fields and sanitizing URLs immediately."""
    providers_in = _unwrap_models_dev_providers(data)
    records: list[dict] = []
    errors: list[dict] = []
    for key in sorted(providers_in):
        item = providers_in[key]
        if not isinstance(item, dict):
            errors.append({"upstream": "models-dev", "id": key, "error": "provider entry is not an object"})
            continue
        pid = item.get("id")
        if pid is None:
            pid = key
        elif not isinstance(pid, str) or not pid:
            errors.append({"upstream": "models-dev", "id": key, "error": "invalid provider id"})
            continue
        api = item.get("api")
        if "api" in item and api is not None and not isinstance(api, str):
            errors.append({"upstream": "models-dev", "id": pid, "error": "`api` must be a string"})
            continue
        env_raw = item.get("env")
        if "env" in item and env_raw is not None:
            if not isinstance(env_raw, list) or not all(isinstance(v, str) and v for v in env_raw):
                errors.append({"upstream": "models-dev", "id": pid, "error": "`env` must be a list of strings"})
                continue
        env_list = None if env_raw is None else sorted(set(env_raw))
        url_info = rs.sanitize_url(api)
        raw_env_vars_in_api = set(_UPSTREAM_TEMPLATE_VAR_RE.findall(api)) if isinstance(api, str) else set()
        records.append(
            {
                "upstream": "models-dev",
                "id": pid,
                "url_info": url_info,
                "raw_api": api if isinstance(api, str) else None,
                "api_template_vars": raw_env_vars_in_api,
                "env": env_list,
                "npm": item.get("npm") if isinstance(item.get("npm"), str) else None,
                "candidate": False,
            }
        )
    if errors:
        raise RegistryInputError(f"models.dev schema errors: {errors[0]['id']}: {errors[0]['error']}")
    return records, errors


def parse_litellm_checkout(checkout: Path) -> tuple[list[dict], list[dict]]:
    """Statically scan LiteLLM `litellm/llms/*/chat/transformation.py` without importing or executing code."""
    llms_dir = checkout / "litellm" / "llms"
    if not llms_dir.is_dir():
        raise RegistryInputError(f"{checkout}: missing `litellm/llms` directory")
    skip_dirs = {"deprecated_providers", "base_llm", "aiohttp_openai", "custom_httpx", "a2a", "empower"}
    records: list[dict] = []
    for pdir in sorted(llms_dir.iterdir()):
        if not pdir.is_dir() or pdir.name in skip_dirs:
            continue
        trans = pdir / "chat" / "transformation.py"
        if not trans.is_file():
            chat_dir = pdir / "chat"
            if not chat_dir.is_dir():
                continue
            py_files = sorted(chat_dir.glob("*.py"))
            if not py_files:
                continue
            trans = py_files[0]
        text = trans.read_text(encoding="utf-8", errors="replace")
        raw_api = None
        for pat in (
            r'(?:DEFAULT_API_BASE|api_base)\s*[:=]\s*["\'](https?://[^"\']+)["\']',
            r'self\.api_base\s*=\s*["\'](https?://[^"\']+)["\']',
        ):
            m = re.search(pat, text)
            if m:
                raw_api = m.group(1)
                break
        envs = sorted(
            set(
                re.findall(
                    r'(?:os\.getenv|os\.environ\.get)\(["\']([A-Z][A-Z0-9_]*)["\']',
                    text,
                )
            )
        )
        records.append(
            {
                "upstream": "litellm",
                "id": pdir.name,
                "url_info": rs.sanitize_url(raw_api),
                "raw_api": raw_api,
                "api_template_vars": set(),
                "env": envs or None,
                "npm": None,
                "candidate": True,
            }
        )
    return records, []


def _in_memory_url_equal(raw_a: str, info_a: dict, raw_b: str, info_b: dict) -> bool:
    """Compare two URLs in memory without leaking raw secret fields out of this helper."""
    if info_a["kind"] != "static" and info_b["kind"] != "static":
        return False
    if info_a["sensitive"] != info_b["sensitive"]:
        return False
    if not info_a["sensitive"]:
        return info_a["canonical"] == info_b["canonical"]
    try:
        pa = urlsplit(raw_a)
        pb = urlsplit(raw_b)
        port_a = pa.port or (80 if pa.scheme.lower() == "http" else 443)
        port_b = pb.port or (80 if pb.scheme.lower() == "http" else 443)
        host_a = (pa.hostname or "").encode("idna").decode("ascii").lower()
        host_b = (pb.hostname or "").encode("idna").decode("ascii").lower()
    except (ValueError, UnicodeError):
        return False
    return (
        pa.scheme.lower() == pb.scheme.lower()
        and host_a == host_b
        and port_a == port_b
        and (pa.username or "") == (pb.username or "")
        and (pa.password or "") == (pb.password or "")
        and pa.path == pb.path
        and pa.query == pb.query
        and pa.fragment == pb.fragment
    )


def _upstream_key_envs(up_rec: dict, local_entry: dict) -> list[str] | None:
    if up_rec["env"] is None:
        return None
    excluded = set(local_entry["config_env_names"]) | up_rec["api_template_vars"]
    filtered = [name for name in up_rec["env"] if name not in excluded]
    return sorted(filtered)


def _override_matches(override: dict, local_val: object, upstream_val: object) -> bool:
    ov_local = override["local_value"]
    ov_up = override["upstream_value"]
    if override["field"] == "base_url":
        ov_l = rs.sanitize_url(ov_local)["sanitized"]
        ov_u = rs.sanitize_url(ov_up)["sanitized"]
        return ov_l == local_val and ov_u == upstream_val
    # env_var: allow string or sorted list in upstream_value
    if isinstance(ov_up, list):
        ov_up_norm = sorted(ov_up)
    elif isinstance(ov_up, str):
        ov_up_norm = [ov_up]
    else:
        ov_up_norm = None
    up_norm = sorted(upstream_val) if isinstance(upstream_val, list) else upstream_val
    return ov_local == local_val and ov_up_norm == up_norm


def compare_registries(
    *,
    local_by_id: dict[str, dict],
    native_packages: set[str],
    aliases: dict[str, str],
    reconcile: dict,
    upstream_records: list[dict],
    source_meta: list[dict],
    mode: str,
) -> tuple[dict, list[dict]]:
    """Compare local registry entries against upstream observations and reconcile rules."""
    all_builtin_ids = set(local_by_id.keys()) | native_packages
    entries_meta = reconcile.get("entries", {})
    overrides = reconcile.get("overrides", [])
    overrides_by_key = {(o["id"], o["upstream"], o["field"]): o for o in overrides}
    matched_override_keys: set[tuple[str, str, str]] = set()

    any_redacted = False
    for entry in local_by_id.values():
        if entry["url_info"]["sensitive"]:
            any_redacted = True
    for rec in upstream_records:
        if rec["url_info"]["sensitive"]:
            any_redacted = True

    missing_upstream: list[dict] = []
    base_url_diffs: list[dict] = []
    key_env_diffs: list[dict] = []
    incomparable: list[dict] = []
    manual_deviations: dict[str, dict] = {}
    proposed_updates: list[dict] = []

    matched_presets: set[str] = set()
    matched_native: set[str] = set()

    for pid, meta in sorted(entries_meta.items()):
        manual_deviations[pid] = {
            "id": pid,
            "note": meta["note"],
            "status": meta["status"],
            "source_url": meta.get("source_url"),
            "diffs": [],
        }

    for up_rec in upstream_records:
        up_name = up_rec["upstream"]
        up_id = up_rec["id"]
        target = aliases.get(up_id, up_id if up_id in all_builtin_ids else None)

        if target is None:
            missing_upstream.append(
                {
                    "upstream": up_name,
                    "id": up_id,
                    "scope": "built-in registry + native packages",
                    "endpoint": up_rec["url_info"]["sanitized"],
                    "key_env_names": up_rec["env"] or [],
                    "candidate": up_rec["candidate"],
                    "npm": up_rec["npm"],
                }
            )
            continue

        if target not in local_by_id:
            matched_native.add(target)
            continue

        matched_presets.add(target)
        local_entry = local_by_id[target]
        manual = entries_meta.get(target)

        # ── base_url comparison ─────────────────────────────────────────────
        l_url = local_entry["url_info"]
        u_url = up_rec["url_info"]
        if u_url["kind"] == "missing":
            incomparable.append(
                {
                    "upstream": up_name,
                    "upstream_id": up_id,
                    "id": target,
                    "field": "base_url",
                    "comparison": "unknown",
                    "reason": "upstream has no base URL",
                    "local": l_url["sanitized"],
                    "upstream_value": None,
                }
            )
        elif l_url["sensitive"] or u_url["sensitive"]:
            # Never persist exceptions or auto-apply invalid-sensitive URLs.
            raw_eq = _in_memory_url_equal(
                local_entry["raw_base_url"], l_url, up_rec["raw_api"] or "", u_url
            )
            row_diff = {
                "upstream": up_name,
                "upstream_id": up_id,
                "id": target,
                "field": "base_url",
                "comparison": "equal" if raw_eq else "different",
                "local": l_url["sanitized"],
                "upstream_value": u_url["sanitized"],
                "local_kind": l_url["kind"],
                "upstream_kind": u_url["kind"],
                "redacted_components": sorted(
                    set(l_url["redacted_components"]) | set(u_url["redacted_components"])
                ),
                "reconcile_status": "sensitive-blocked",
            }
            if raw_eq:
                incomparable.append(row_diff)
            else:
                base_url_diffs.append(row_diff)
        elif l_url["kind"] == "templated" or u_url["kind"] == "templated":
            incomparable.append(
                {
                    "upstream": up_name,
                    "upstream_id": up_id,
                    "id": target,
                    "field": "base_url",
                    "comparison": "unknown",
                    "reason": "templated endpoint cannot be expanded offline",
                    "local": l_url["sanitized"],
                    "upstream_value": u_url["sanitized"],
                }
            )
        else:
            raw_eq = _in_memory_url_equal(
                local_entry["raw_base_url"], l_url, up_rec["raw_api"] or "", u_url
            )
            if not raw_eq:
                okey = (target, up_name, "base_url")
                ov = overrides_by_key.get(okey)
                acknowledged = bool(
                    ov and _override_matches(ov, l_url["sanitized"], u_url["sanitized"])
                )
                if acknowledged:
                    matched_override_keys.add(okey)
                diff_row = {
                    "upstream": up_name,
                    "upstream_id": up_id,
                    "id": target,
                    "field": "base_url",
                    "comparison": "different",
                    "local": l_url["sanitized"],
                    "upstream_value": u_url["sanitized"],
                    "candidate": up_rec["candidate"],
                    "reconcile_status": "acknowledged" if acknowledged else ("manual-note" if manual else "pending"),
                }
                if acknowledged or manual is not None:
                    box = manual_deviations.setdefault(
                        target,
                        {
                            "id": target,
                            "note": manual["note"] if manual else ov["reason"],
                            "status": manual["status"] if manual else None,
                            "source_url": (manual or ov).get("source_url"),
                            "diffs": [],
                        },
                    )
                    box["diffs"].append(diff_row)
                else:
                    base_url_diffs.append(diff_row)
                    if (
                        not up_rec["candidate"]
                        and u_url["kind"] == "static"
                        and up_rec["raw_api"]
                        and up_rec["raw_api"].isascii()
                        and not any(c in up_rec["raw_api"] for c in ("<", ">", "$", "{", "}"))
                    ):
                        proposed_updates.append(
                            {
                                "id": target,
                                "field": "base_url",
                                "upstream": up_name,
                                "upstream_id": up_id,
                                "old_value": l_url["sanitized"],
                                "new_value": u_url["canonical"],
                                "raw_new_value": up_rec["raw_api"],
                            }
                        )

        # ── key_env comparison ──────────────────────────────────────────────
        up_key_envs = _upstream_key_envs(up_rec, local_entry)
        if up_key_envs is None:
            incomparable.append(
                {
                    "upstream": up_name,
                    "upstream_id": up_id,
                    "id": target,
                    "field": "env_var",
                    "comparison": "unknown",
                    "reason": "upstream omits env list",
                    "local": local_entry["key_env_names"],
                    "upstream_value": None,
                }
            )
            continue

        local_keys = local_entry["key_env_names"]
        env_equal = (not local_keys and not up_key_envs) or (
            len(local_keys) == 1 and local_keys[0] in up_key_envs
        )
        if not env_equal:
            local_env_val = local_entry["raw_env_var"]
            okey = (target, up_name, "env_var")
            ov = overrides_by_key.get(okey)
            acknowledged = bool(ov and _override_matches(ov, local_env_val, up_key_envs))
            if acknowledged:
                matched_override_keys.add(okey)
            diff_row = {
                "upstream": up_name,
                "upstream_id": up_id,
                "id": target,
                "field": "env_var",
                "comparison": "different",
                "local": local_keys,
                "local_key_mode": local_entry["key_mode"],
                "upstream_value": up_key_envs,
                "candidate": up_rec["candidate"],
                "reconcile_status": "acknowledged" if acknowledged else ("manual-note" if manual else "pending"),
            }
            if acknowledged or manual is not None:
                box = manual_deviations.setdefault(
                    target,
                    {
                        "id": target,
                        "note": manual["note"] if manual else ov["reason"],
                        "status": manual["status"] if manual else None,
                        "source_url": (manual or ov).get("source_url"),
                        "diffs": [],
                    },
                )
                box["diffs"].append(diff_row)
            else:
                key_env_diffs.append(diff_row)
                if (
                    not up_rec["candidate"]
                    and len(up_key_envs) == 1
                    and _ENV_NAME_RE.match(up_key_envs[0])
                ):
                    proposed_updates.append(
                        {
                            "id": target,
                            "field": "env_var",
                            "upstream": up_name,
                            "upstream_id": up_id,
                            "old_value": local_env_val,
                            "new_value": up_key_envs[0],
                            "raw_new_value": up_key_envs[0],
                        }
                    )

    # Check any stale overrides in reconcile.json
    stale_overrides: list[dict] = []
    for o in overrides:
        okey = (o["id"], o["upstream"], o["field"])
        if okey not in matched_override_keys:
            stale_overrides.append(
                {
                    "id": o["id"],
                    "upstream": o["upstream"],
                    "field": o["field"],
                    "override_local_value": (
                        rs.sanitize_url(o["local_value"])["sanitized"]
                        if o["field"] == "base_url"
                        else o["local_value"]
                    ),
                    "override_upstream_value": (
                        rs.sanitize_url(o["upstream_value"])["sanitized"]
                        if o["field"] == "base_url"
                        else o["upstream_value"]
                    ),
                    "reason": o["reason"],
                }
            )

    missing_upstream.sort(key=lambda r: (r["upstream"], r["id"]))
    base_url_diffs.sort(key=lambda r: (r["upstream"], r["id"], r["field"]))
    key_env_diffs.sort(key=lambda r: (r["upstream"], r["id"], r["field"]))
    incomparable.sort(key=lambda r: (r["upstream"], r["id"], r["field"]))
    stale_overrides.sort(key=lambda r: (r["upstream"], r["id"], r["field"]))
    manual_list = [manual_deviations[k] for k in sorted(manual_deviations)]
    for item in manual_list:
        item["diffs"].sort(key=lambda d: (d["upstream"], d["field"]))

    sanitized_local_for_hash = [
        {
            "id": e["id"],
            "auth": e["auth"],
            "endpoint": e["url_info"]["sanitized"],
            "key_env_names": e["key_env_names"],
            "config_env_names": e["config_env_names"],
        }
        for _, e in sorted(local_by_id.items())
    ]
    sanitized_upstream_for_hash = [
        {
            "upstream": r["upstream"],
            "id": r["id"],
            "endpoint": r["url_info"]["sanitized"],
            "env": r["env"],
        }
        for r in sorted(upstream_records, key=lambda x: (x["upstream"], x["id"]))
    ]
    input_hash_kind = "sanitized" if any_redacted else "original"
    input_hash = rs.sha256_canonical_json(
        {
            "local": sanitized_local_for_hash,
            "upstream": sanitized_upstream_for_hash,
            "aliases": aliases,
            "reconcile": reconcile,
        }
    )

    report = {
        "schema_version": SCHEMA_VERSION,
        "mode": mode,
        "adapter_version": ADAPTER_VERSION,
        "comparison_policy": {
            "normalize_scheme_host_case": True,
            "normalize_default_port": True,
            "preserve_path_and_trailing_slash": True,
            "synthesize_v1": False,
            "evaluate_templates": False,
            "exact_id_and_explicit_aliases_only": True,
        },
        "sources": source_meta,
        "input_hash_kind": input_hash_kind,
        "input_hash": input_hash,
        "coverage": {
            "local_presets": len(local_by_id),
            "native_packages": len(native_packages),
            "upstream_providers": len(upstream_records),
            "matched_presets": len(matched_presets),
            "matched_native": len(matched_native),
            "unmapped_upstream": len(missing_upstream),
        },
        "counts": {
            "missing_upstream_providers": len(missing_upstream),
            "base_url_diffs": len(base_url_diffs),
            "key_env_diffs": len(key_env_diffs),
            "manual_deviations": len(manual_list),
            "stale_overrides": len(stale_overrides),
            "incomparable": len(incomparable),
            "applied_updates": 0,
            "pending_review": len(base_url_diffs) + len(key_env_diffs) + len(stale_overrides),
        },
        "missing_upstream_providers": missing_upstream,
        "base_url_diffs": base_url_diffs,
        "key_env_diffs": key_env_diffs,
        "manual_deviations": manual_list,
        "stale_overrides": stale_overrides,
        "incomparable": incomparable,
        "applied_updates": [],
        "tool_errors": [],
    }
    return report, proposed_updates


def apply_updates(registry_path: Path, raw_rows: list[dict], proposed_updates: list[dict]) -> list[dict]:
    """Write back approved `base_url` / `env_var` updates to `provider_registry.json`."""
    row_by_name = {r["name"]: r for r in raw_rows}
    applied: list[dict] = []
    # Deduplicate by (id, field), preferring primary models-dev source
    selected: dict[tuple[str, str], dict] = {}
    for upd in sorted(proposed_updates, key=lambda u: (u["id"], u["field"], u["upstream"])):
        selected.setdefault((upd["id"], upd["field"]), upd)

    for (rid, field), upd in sorted(selected.items()):
        row = row_by_name[rid]
        if field == "base_url":
            row["base_url"] = upd["raw_new_value"]
        elif field == "env_var":
            row["env_var"] = upd["raw_new_value"]
        applied.append(
            {
                "id": rid,
                "field": field,
                "upstream": upd["upstream"],
                "upstream_id": upd["upstream_id"],
                "old_value": upd["old_value"],
                "new_value": upd["new_value"],
            }
        )
    if applied:
        registry_path.write_text(
            json.dumps(raw_rows, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    return applied


def render_markdown(report: dict) -> str:
    """Render Markdown strictly from the sanitized report object (RFC-0033 §5)."""
    c = report["counts"]
    lines = [
        "### Registry Sync Report (RFC-0033)",
        "",
        f"- **Mode**: `{report['mode']}` · **Hash ({report['input_hash_kind']})**: `{report['input_hash'][:12]}`",
        (
            f"- **Pending review**: `{c['pending_review']}` "
            f"(base_url: `{c['base_url_diffs']}`, env_var: `{c['key_env_diffs']}`, stale: `{c['stale_overrides']}`)"
        ),
        (
            f"- **Missing upstream**: `{c['missing_upstream_providers']}` · "
            f"**Manual deviations**: `{c['manual_deviations']}` · "
            f"**Incomparable**: `{c['incomparable']}`"
        ),
        "",
    ]
    if report["base_url_diffs"]:
        lines.extend(["#### Base URL Diffs", "", "| Upstream | Preset | Local | Upstream Value |", "|---|---|---|---|"])
        for d in report["base_url_diffs"]:
            lines.append(
                f"| {rs.escape_markdown(d['upstream'])}:{rs.escape_markdown(d['upstream_id'])} "
                f"| {rs.escape_markdown(d['id'])} "
                f"| {rs.escape_markdown(str(d['local']))} "
                f"| {rs.escape_markdown(str(d['upstream_value']))} |"
            )
        lines.append("")
    if report["key_env_diffs"]:
        lines.extend(["#### Key Env Diffs", "", "| Upstream | Preset | Local | Upstream Value |", "|---|---|---|---|"])
        for d in report["key_env_diffs"]:
            lines.append(
                f"| {rs.escape_markdown(d['upstream'])}:{rs.escape_markdown(d['upstream_id'])} "
                f"| {rs.escape_markdown(d['id'])} "
                f"| {rs.escape_markdown(', '.join(d['local']) or '(none)')} "
                f"| {rs.escape_markdown(', '.join(d['upstream_value']))} |"
            )
        lines.append("")
    if report["stale_overrides"]:
        lines.extend(["#### Stale Reconcile Overrides", "", "| Upstream | Preset | Field | Reason |", "|---|---|---|---|"])
        for s in report["stale_overrides"]:
            lines.append(
                f"| {rs.escape_markdown(s['upstream'])} | {rs.escape_markdown(s['id'])} "
                f"| {rs.escape_markdown(s['field'])} | {rs.escape_markdown(s['reason'])} |"
            )
        lines.append("")
    if report["missing_upstream_providers"]:
        lines.extend(["#### Missing Upstream Providers (Not Added Automatically)", "", "| Upstream | Upstream ID | Endpoint | Key Env |", "|---|---|---|---|"])
        for m in report["missing_upstream_providers"][:50]:
            ep = rs.escape_markdown(str(m["endpoint"] or "(none)"))
            envs = rs.escape_markdown(", ".join(m["key_env_names"]) or "(none)")
            lines.append(
                f"| {rs.escape_markdown(m['upstream'])} | {rs.escape_markdown(m['id'])} | {ep} | {envs} |"
            )
        if len(report["missing_upstream_providers"]) > 50:
            lines.append(f"\n_... and {len(report['missing_upstream_providers']) - 50} more omitted in summary._\n")
    return "\n".join(lines).rstrip() + "\n"


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="RFC-0033 registry sync and offline check")
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--report", action="store_true", help="Emit structured JSON diff report")
    mode.add_argument("--apply", action="store_true", help="Update existing preset URLs/env_vars outside reconcile overrides")
    mode.add_argument("--check", action="store_true", help="Offline verify registry matches snapshot + reconcile.json")
    p.add_argument("--source", choices=["models-dev", "litellm"], default=None, help="Upstream source")
    src_in = p.add_mutually_exclusive_group()
    src_in.add_argument("--snapshot", type=Path, help="Offline models.dev JSON snapshot path")
    src_in.add_argument("--fetch", action="store_true", help="Fetch live models.dev JSON")
    p.add_argument("--checkout", type=Path, help="Local LiteLLM git checkout path (static scan only)")
    p.add_argument("--write-snapshot", type=Path, help="Write slimmed deterministic models.dev snapshot JSON")
    p.add_argument("--output", type=Path, help="Write JSON report to PATH (stdout if omitted in --report)")
    p.add_argument("--markdown-output", type=Path, help="Write Markdown report view to PATH")
    p.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    p.add_argument("--aliases", type=Path, default=DEFAULT_ALIASES)
    p.add_argument("--reconcile", type=Path, default=DEFAULT_RECONCILE)
    p.add_argument("--packages-rs", type=Path, default=DEFAULT_PACKAGES_RS)
    p.add_argument("--fetch-url", default=DEFAULT_MODELS_DEV_URL)
    p.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT_SECS)
    return p


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else 2

    if not (args.report or args.apply or args.check or args.write_snapshot):
        print("specify one of --report, --apply, --check, or --write-snapshot", file=sys.stderr)
        return 2
    if args.check and args.fetch:
        print("--check runs offline and cannot be combined with --fetch", file=sys.stderr)
        return 2
    if (args.report or args.apply or args.check) and args.source is None:
        print("--source is required with --report, --apply, or --check", file=sys.stderr)
        return 2
    if args.source == "models-dev" and not args.snapshot and not args.fetch:
        print("--source models-dev requires --snapshot or --fetch", file=sys.stderr)
        return 2
    if args.source == "litellm" and not args.checkout:
        print("--source litellm requires --checkout PATH", file=sys.stderr)
        return 2
    if args.write_snapshot and args.source not in (None, "models-dev"):
        print("--write-snapshot only applies to models-dev", file=sys.stderr)
        return 2
    if args.write_snapshot and not args.snapshot and not args.fetch:
        print("--write-snapshot requires --snapshot or --fetch", file=sys.stderr)
        return 2
    if args.timeout <= 0:
        print("--timeout must be positive", file=sys.stderr)
        return 2

    try:
        raw_rows, local_by_id = load_local_registry(args.registry)
        native_packages = load_native_packages(args.packages_rs)
        all_builtin = set(local_by_id.keys()) | native_packages
        aliases = load_aliases(args.aliases, all_builtin)
        reconcile = load_reconcile(args.reconcile, set(local_by_id.keys()))

        upstream_records: list[dict] = []
        source_meta: list[dict] = []
        if args.source == "models-dev" or args.write_snapshot:
            if args.fetch:
                raw_md = fetch_models_dev(args.fetch_url, timeout=args.timeout)
                src_desc = rs.sanitize_url(args.fetch_url)["sanitized"]
            else:
                raw_md = load_json_file(args.snapshot, str(args.snapshot))
                src_desc = str(args.snapshot.name)
            if args.write_snapshot:
                slim = slim_models_dev_snapshot(raw_md, source_url=args.fetch_url)
                args.write_snapshot.parent.mkdir(parents=True, exist_ok=True)
                args.write_snapshot.write_text(
                    json.dumps(slim, indent=2, ensure_ascii=False, sort_keys=True) + "\n",
                    encoding="utf-8",
                )
            recs, _ = parse_models_dev(raw_md)
            upstream_records.extend(recs)
            source_meta.append({"source": "models-dev", "location": src_desc, "provider_count": len(recs)})
        if args.source == "litellm":
            recs, _ = parse_litellm_checkout(args.checkout)
            upstream_records.extend(recs)
            source_meta.append({"source": "litellm", "location": str(args.checkout.name), "provider_count": len(recs)})

        if not (args.report or args.apply or args.check):
            return 0

        mode = "apply" if args.apply else ("check" if args.check else "report")
        report, proposed = compare_registries(
            local_by_id=local_by_id,
            native_packages=native_packages,
            aliases=aliases,
            reconcile=reconcile,
            upstream_records=upstream_records,
            source_meta=source_meta,
            mode=mode,
        )
        if args.apply:
            applied = apply_updates(args.registry, raw_rows, proposed)
            report["applied_updates"] = applied
            report["counts"]["applied_updates"] = len(applied)

        rendered = json.dumps(report, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(rendered, encoding="utf-8")
        elif args.report or args.apply:
            sys.stdout.write(rendered)

        if args.markdown_output:
            args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
            args.markdown_output.write_text(render_markdown(report), encoding="utf-8")

        if args.check:
            pending = report["counts"]["pending_review"]
            if pending > 0:
                print(
                    (
                        f"registry check failed: {report['counts']['base_url_diffs']} base_url diffs, "
                        f"{report['counts']['key_env_diffs']} env_var diffs, "
                        f"{report['counts']['stale_overrides']} stale reconcile overrides"
                    ),
                    file=sys.stderr,
                )
                return 1
        return 0
    except RegistryInputError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"io error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
