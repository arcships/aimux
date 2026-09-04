#!/usr/bin/env python3
"""Generate aimux-providers/data/models/<provider>.json — the L2 model catalogue.

One file per registry provider that an upstream catalogue knows about, plus a
`.manifest.json` recording counts and a content hash. models.dev is primary;
anya2a fills in only the providers models.dev does not carry. Hand-written
corrections live in `aimux-providers/data/models.overrides.json` and are
applied after normalisation.

Usage:
    python3 scripts/gen_models.py                       # fetch upstream, write
    python3 scripts/gen_models.py --offline \
        --models-dev api.json --anya2a all.json         # write from snapshots
    python3 scripts/gen_models.py --check                # verify committed files
"""

import argparse
import hashlib
import json
import re
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"
CATALOGUE_RS = ROOT / "aimux-providers" / "src" / "catalogue.rs"
DATA = ROOT / "aimux-providers" / "data"
OUT_DIR = DATA / "models"
OVERRIDES = DATA / "models.overrides.json"
MANIFEST = OUT_DIR / ".manifest.json"

MODELS_DEV_URL = "https://models.dev/api.json"
ANYA2A_URL = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/all.json"

GENERATED = "scripts/gen_models.py — do not edit; see aimux-providers/data/models.overrides.json"
SCHEMA_VERSION = 1


def registry_names() -> set[str]:
    return {e["name"] for e in json.loads(REGISTRY.read_text(encoding="utf-8"))}


def provider_aliases() -> dict[str, str]:
    """Read PROVIDER_ALIASES straight out of catalogue.rs.

    Parsing the Rust table keeps the anya2a id -> registry name mapping in one
    place; duplicating the 38 pairs here would be a second thing to forget.
    """
    src = CATALOGUE_RS.read_text(encoding="utf-8")
    table = src.split("PROVIDER_ALIASES", 1)[1]
    table = table[table.index("[") : table.index("];")]
    return dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', table))


def normalize_provider_name(anya2a_id: str, aliases: dict[str, str]) -> str:
    return aliases.get(anya2a_id, anya2a_id.replace("-", "_"))


def _get(obj, *path):
    for key in path:
        if not isinstance(obj, dict):
            return None
        obj = obj.get(key)
    return obj


def _cost(raw):
    c = raw.get("cost") or {}
    return {k: c.get(k) for k in ("input", "output", "cache_read", "cache_write")}


def from_models_dev(raw: dict) -> dict:
    return {
        "id": raw.get("id"),
        "name": raw.get("name"),
        "context": _get(raw, "limit", "context"),
        "max_output": _get(raw, "limit", "output"),
        "cost": _cost(raw),
        "modalities": {
            "input": _get(raw, "modalities", "input"),
            "output": _get(raw, "modalities", "output"),
        },
        "reasoning": raw.get("reasoning"),
        "tool_call": raw.get("tool_call"),
        "release_date": raw.get("release_date"),
        "knowledge": raw.get("knowledge"),
    }


def from_anya2a(raw: dict) -> dict:
    m = from_models_dev(raw)
    # anya2a widens `reasoning` into an object; everything else lines up.
    m["reasoning"] = _get(raw, "reasoning", "supported")
    m["name"] = raw.get("name") or raw.get("display_name")
    return m


def apply_overrides(provider: str, models: list[dict], overrides: dict) -> list[dict]:
    table = overrides.get(provider)
    if not table:
        return models
    out = []
    for m in models:
        patch = table.get(m["id"])
        if patch is None:
            out.append(m)
            continue
        if patch.get("_drop"):
            continue
        out.append({**m, **{k: v for k, v in patch.items() if not k.startswith("_")}})
    return out


def dumps(obj) -> str:
    return json.dumps(obj, indent=2, ensure_ascii=False, sort_keys=False) + "\n"


def provider_files() -> list[Path]:
    """Every generated provider file, name-sorted. `.manifest.json` is not one."""
    return sorted((p for p in OUT_DIR.glob("*.json") if not p.name.startswith(".")), key=lambda p: p.name)


def content_hash(files: list[Path]) -> str:
    h = hashlib.sha256()
    for path in sorted(files, key=lambda p: p.name):
        h.update(path.read_bytes())
    return h.hexdigest()


def fetch(url):
    # models.dev answers 403 to urllib's default User-Agent.
    req = urllib.request.Request(url, headers={"User-Agent": "aimux-provider-sync"})
    with urllib.request.urlopen(req, timeout=60) as resp:  # noqa: S310 — https literals above
        return json.loads(resp.read().decode("utf-8"))


def build(models_dev: dict, anya2a: dict) -> int:
    names = registry_names()
    aliases = provider_aliases()
    overrides = json.loads(OVERRIDES.read_text(encoding="utf-8")) if OVERRIDES.is_file() else {}

    # Several upstream ids fold onto one registry name (alibaba-cn -> alibaba,
    # stepfun-ai -> stepfun, ...). Merge their models by id, as
    # catalogue.rs::parse_anya2a_all already does.
    collected: dict[str, dict[str, dict]] = {}
    source_of: dict[str, str] = {}
    for pid in sorted(models_dev):
        name = normalize_provider_name(pid, aliases)
        if name not in names:
            continue
        source_of[name] = "models.dev"
        bucket = collected.setdefault(name, {})
        for raw in (models_dev[pid].get("models") or {}).values():
            model = from_models_dev(raw)
            bucket[model["id"]] = model
    for pid in sorted(anya2a.get("providers") or {}):
        name = normalize_provider_name(pid, aliases)
        if name not in names or name in source_of:
            continue
        source_of[name] = "anya2a"
        bucket = collected.setdefault(name, {})
        for raw in (anya2a["providers"][pid].get("models") or []):
            model = from_anya2a(raw)
            bucket[model["id"]] = model

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for stale in provider_files():
        stale.unlink()

    total_models = 0
    for name, bucket in sorted(collected.items()):
        models = apply_overrides(name, list(bucket.values()), overrides)
        models.sort(key=lambda m: m["id"] or "")
        total_models += len(models)
        (OUT_DIR / f"{name}.json").write_text(
            dumps(
                {
                    "_generated": GENERATED,
                    "provider": name,
                    "source": source_of[name],
                    "models": models,
                }
            ),
            encoding="utf-8",
        )

    files = provider_files()
    MANIFEST.write_text(
        dumps(
            {
                "schema_version": SCHEMA_VERSION,
                "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "sources": {"models.dev": MODELS_DEV_URL, "anya2a": ANYA2A_URL},
                "providers": len(files),
                "models": total_models,
                "hash": content_hash(files),
            }
        ),
        encoding="utf-8",
    )
    print(f"wrote {len(files)} providers / {total_models} models -> {OUT_DIR.relative_to(ROOT)}")
    return 0


def check() -> int:
    if not MANIFEST.is_file():
        print(f"missing {MANIFEST.relative_to(ROOT)} — run scripts/gen_models.py", file=sys.stderr)
        return 1
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    names = registry_names()
    files = provider_files()
    problems = []
    for path in files:
        doc = json.loads(path.read_text(encoding="utf-8"))
        if doc.get("_generated") != GENERATED:
            problems.append(f"{path.name}: missing or stale `_generated` header")
        if doc.get("provider") not in names:
            problems.append(f"{path.name}: provider {doc.get('provider')!r} is not in provider_registry.json")
    if manifest.get("providers") != len(files):
        problems.append(f"manifest says {manifest.get('providers')} providers, found {len(files)}")
    digest = content_hash(files)
    if digest != manifest.get("hash"):
        problems.append(f"content hash mismatch: files {digest}, manifest {manifest.get('hash')}")
    if problems:
        for p in problems:
            print(f"STALE: {p}", file=sys.stderr)
        print("model data is out of sync — run scripts/gen_models.py and commit the result", file=sys.stderr)
        return 1
    print(f"{len(files)} provider files up to date ({manifest.get('models')} models)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true", help="verify the committed files (no network)")
    ap.add_argument("--offline", action="store_true", help="read snapshots instead of fetching")
    ap.add_argument("--models-dev", type=Path, help="path to a models.dev api.json snapshot")
    ap.add_argument("--anya2a", type=Path, help="path to an anya2a all.json snapshot")
    args = ap.parse_args()

    if args.check:
        return check()
    if args.offline:
        if not (args.models_dev and args.anya2a):
            ap.error("--offline requires --models-dev and --anya2a")
        return build(
            json.loads(args.models_dev.read_text(encoding="utf-8")),
            json.loads(args.anya2a.read_text(encoding="utf-8")),
        )
    return build(fetch(MODELS_DEV_URL), fetch(ANYA2A_URL))


if __name__ == "__main__":
    sys.exit(main())
