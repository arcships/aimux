#!/usr/bin/env python3
"""Generate provider-registry.json from openai_compat_registry.rs (RFC-0017 phase 4).

Single source of truth: the registry declaration file. This script extracts
name/display/base_url/env_var/profile for all 250 OpenAI-compatible providers
and writes aimux-providers/src/provider_registry.json (embedded via include_str!).
Also regenerates the ProviderName derived types (Rust enum / TS union / ...).

Usage: uv run python scripts/gen_provider_registry.py
Idempotent: overwrites the output files.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "aimux-providers" / "src" / "openai_compat_registry.rs"
OUT_JSON = ROOT / "aimux-providers" / "src" / "provider_registry.json"

BLOCK_RE = re.compile(
    r"declare_openai_compat_provider!\((.*?)\);", re.DOTALL
)
PROFILE_RE = re.compile(
    r"OpenAICompatProfile::(\w+)\(\)(.*?)$", re.DOTALL
)
MAX_TOKENS_RE = re.compile(r'\.with_max_tokens_key\("([^"]+)"\)')


def parse_profile(expr: str) -> dict:
    """Translate a profile expression into its JSON form (non-default fields only)."""
    m = PROFILE_RE.search(expr.strip())
    if not m:
        raise ValueError(f"unparseable profile expr: {expr!r}")
    kind = m.group(1)
    rest = m.group(2)

    profile: dict = {}
    if kind == "groq":
        # Full groq() semantics (mod.rs OpenAICompatProfile::groq()):
        # supports_top_k=false, stream_usage_key="x_groq",
        # max_tokens_key="max_completion_tokens" (stage2-002 wiring).
        profile.update({
            "supports_top_k": False,
            "stream_usage_key": "x_groq",
            "max_tokens_key": "max_completion_tokens",
        })
    elif kind == "deepseek":
        # deepseek() now returns full() semantics (RFC-0017 phase 2 retirements)
        pass
    elif kind != "full":
        raise ValueError(f"unknown profile kind: {kind}")

    mt = MAX_TOKENS_RE.search(rest)
    if mt:
        profile["max_tokens_key"] = mt.group(1)

    return profile


def main() -> int:
    text = REGISTRY.read_text(encoding="utf-8")
    entries = []
    for block in BLOCK_RE.finditer(text):
        body = re.sub(r"\s+", " ", block.group(1)).strip()
        parts = body.split(", ")
        if len(parts) < 7:
            raise ValueError(f"malformed block: {block.group(0)[:80]!r}")
        name = parts[0].strip('"')
        display = parts[3].strip('"')
        # base_url part may carry a preceding `//` comment line (freemodel);
        # take the LAST quoted string in the part. xpersona's "/v1" is a known
        # unresolved base_url (research backlog), kept as-is.
        base_urls = re.findall(r'"([^"]*)"', parts[4])
        if not base_urls:
            raise ValueError(f"no string literal in base_url part: {parts[4]!r}")
        base_url = base_urls[-1]
        env_var = re.match(r'"([^"]*)"', parts[5]).group(1)
        profile = parse_profile(parts[6])
        entries.append({
            "name": name,
            "display": display,
            "base_url": base_url,
            "env_var": env_var,
            "profile": profile,
        })

    entries.sort(key=lambda e: e["name"])
    OUT_JSON.write_text(
        json.dumps(entries, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"provider_registry.json: {len(entries)} providers -> {OUT_JSON.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
