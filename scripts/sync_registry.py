#!/usr/bin/env python3
"""Compare provider_registry.json (L1) against upstream catalogues.

Reports, as Markdown on stdout: providers upstream but missing from the
registry, and base_url / env_var disagreements for the names we share. It never
edits the registry — a human decides what to take. Rows carrying a `note` are
deliberate deviations and are skipped in the disagreement tables.

With --litellm, litellm's own `api_base` constants are scanned as a third
source (absorbs the retired extract_litellm_bases.py / scan_litellm_urls.py).

Usage:
    python3 scripts/sync_registry.py --report
    python3 scripts/sync_registry.py --report --offline \
        --models-dev api.json --anya2a all.json
    python3 scripts/sync_registry.py --report --litellm reference/litellm
"""

import argparse
import json
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"
CATALOGUE_RS = ROOT / "aimux-providers" / "src" / "catalogue.rs"

MODELS_DEV_URL = "https://models.dev/api.json"
ANYA2A_URL = "https://raw.githubusercontent.com/ThinkInAIXYZ/PublicProviderConf/refs/heads/dev/dist/all.json"

# api_base / env-var literals, in the order extract_litellm_bases.py tried them.
LITELLM_BASE_RE = re.compile(
    r'(?:DEFAULT_API_BASE|api_base)\s*[:=]\s*["\']f?(https?://[^"\']+)["\']'
)
LITELLM_ENV_RE = re.compile(
    r'(?:os\.getenv|os\.environ\.get)\(["\'](\w+_(?:API_KEY|TOKEN|KEY))["\']'
)


def fetch(url):
    # models.dev answers 403 to urllib's default User-Agent.
    req = urllib.request.Request(url, headers={"User-Agent": "aimux-provider-sync"})
    with urllib.request.urlopen(req, timeout=60) as resp:  # noqa: S310 — https literals above
        return json.loads(resp.read().decode("utf-8"))


def provider_aliases():
    """anya2a id -> registry name, read from catalogue.rs's PROVIDER_ALIASES."""
    src = CATALOGUE_RS.read_text(encoding="utf-8")
    table = src.split("PROVIDER_ALIASES", 1)[1]
    table = table[table.index("[") : table.index("];")]
    return dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', table))


def upstream_entries(models_dev, anya2a, litellm_dir):
    """registry name -> [(source, upstream id, base_url, [env, ...]), ...].

    Upstream ids are hyphenated where the registry is snake_case, and a few
    fold onto one registry name, so every id goes through the same
    normalize_provider_name the Rust catalogue uses.
    """
    aliases = provider_aliases()
    out = {}

    def add(pid, source, base_url, envs):
        name = aliases.get(pid, pid.replace("-", "_"))
        out.setdefault(name, []).append((source, pid, base_url, envs))

    for pid, p in sorted(models_dev.items()):
        add(pid, "models.dev", p.get("api"), p.get("env") or [])
    for pid, p in sorted((anya2a.get("providers") or {}).items()):
        add(pid, "anya2a", p.get("api"), [])
    if litellm_dir:
        for path in sorted(Path(litellm_dir).glob("litellm/llms/*/chat/transformation.py")):
            text = path.read_text(encoding="utf-8", errors="ignore")
            base = LITELLM_BASE_RE.search(text)
            env = LITELLM_ENV_RE.search(text)
            if base:
                add(path.parents[1].name, "litellm", base.group(1), [env.group(1)] if env else [])
    return out


def norm_url(url):
    return (url or "").rstrip("/").lower()


def table(header, rows):
    if not rows:
        return [f"_none._", ""]
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    out.append("")
    return out


def report(models_dev, anya2a, litellm_dir):
    registry = {e["name"]: e for e in json.loads(REGISTRY.read_text(encoding="utf-8"))}
    upstream = upstream_entries(models_dev, anya2a, litellm_dir)

    missing, base_diff, env_diff = [], [], []
    for name in sorted(upstream):
        entry = registry.get(name)
        if entry is None:
            for source, pid, base, envs in upstream[name]:
                missing.append(
                    [f"`{pid}`", source, f"`{base or '—'}`", ", ".join(f"`{e}`" for e in envs) or "—"]
                )
            continue
        if entry.get("note"):
            continue
        for source, pid, base, envs in upstream[name]:
            if base and norm_url(base) != norm_url(entry["base_url"]):
                base_diff.append([f"`{name}`", f"`{entry['base_url']}`", f"{source} (`{pid}`)", f"`{base}`"])
            if envs and entry["env_var"] not in envs:
                env_diff.append(
                    [f"`{name}`", f"`{entry['env_var']}`", f"{source} (`{pid}`)", ", ".join(f"`{e}`" for e in envs)]
                )

    lines = [f"## Missing from the registry — {len(missing)}", ""]
    lines += table(["upstream id", "source", "base_url", "env"], missing)
    lines += [f"## base_url disagreements — {len(base_diff)}", ""]
    lines += table(["name", "registry", "source", "upstream"], base_diff)
    lines += [f"## env_var disagreements — {len(env_diff)}", ""]
    lines += table(["name", "registry", "source", "upstream"], env_diff)
    lines += [
        f"_{len(registry)} registry rows; "
        f"{sum(1 for e in registry.values() if e.get('note'))} carry a `note` and are exempt "
        "from the disagreement tables._"
    ]
    print("\n".join(lines))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--report", action="store_true", required=True, help="print the Markdown report")
    ap.add_argument("--offline", action="store_true", help="read snapshots instead of fetching")
    ap.add_argument("--models-dev", type=Path, help="path to a models.dev api.json snapshot")
    ap.add_argument("--anya2a", type=Path, help="path to an anya2a all.json snapshot")
    ap.add_argument("--litellm", type=Path, help="path to a litellm checkout (adds a third source)")
    args = ap.parse_args()

    if args.offline:
        if not (args.models_dev and args.anya2a):
            ap.error("--offline requires --models-dev and --anya2a")
        models_dev = json.loads(args.models_dev.read_text(encoding="utf-8"))
        anya2a = json.loads(args.anya2a.read_text(encoding="utf-8"))
    else:
        models_dev, anya2a = fetch(MODELS_DEV_URL), fetch(ANYA2A_URL)
    report(models_dev, anya2a, args.litellm)
    return 0


if __name__ == "__main__":
    sys.exit(main())
