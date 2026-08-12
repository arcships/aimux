#!/usr/bin/env python3
"""Generate docs/api/providers.md — the full provider lookup reference.

Single source of truth:
- aimux-providers/src/provider_registry.json  (registry of OpenAI-compatible entries)
- aimux-providers/src/lib.rs                   (non-registry modules + categories)

Usage:
    python scripts/gen_providers_doc.py
    # writes docs/api/providers.md, prints the module count for verification
"""

import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"
LIB_RS = ROOT / "aimux-providers" / "src" / "lib.rs"
OUT = ROOT / "docs" / "api" / "providers.md"


def load_registry():
    with open(REGISTRY, encoding="utf-8") as f:
        return json.load(f)


def parse_lib_rs():
    """Return [(section_title, [module, ...]), ...] from lib.rs.

    A section header is a run of `//` comment lines (not `//!`) directly
    followed by `pub mod x;` lines. The leading group without a header gets
    the default title "Native protocol providers".
    """
    lines = LIB_RS.read_text(encoding="utf-8").splitlines()
    sections = []
    current = None
    pending = []
    i = 0
    while i < len(lines):
        s = lines[i].strip()
        if s == "" or s.startswith("//!"):
            i += 1
            continue
        if s.startswith("//"):
            # collect the whole comment block
            j = i
            block = []
            while j < len(lines) and lines[j].strip().startswith("//") and not lines[j].strip().startswith("//!"):
                block.append(lines[j].strip().lstrip("/").strip())
                j += 1
            # the next non-blank line must be `pub mod` to count as a header
            k = j
            while k < len(lines) and lines[k].strip() == "":
                k += 1
            if k < len(lines) and lines[k].strip().startswith("pub mod "):
                # registry machinery (provider / provider_name) has its own
                # doc comment block — never treat it as a section header
                first_mod = lines[k].strip().split()[2].rstrip(";")
                if first_mod in ("provider", "provider_name"):
                    i = j
                    continue
                if pending:
                    sections.append((current or "Native protocol providers", pending))
                    pending = []
                current = " ".join(block).rstrip(".")
                i = k
                continue
            i = j
            continue
        m = re.match(r"pub mod (\w+);", s)
        if m:
            pending.append(m.group(1))
            i += 1
            continue
        i += 1
    if pending:
        sections.append((current or "Native protocol providers", pending))
    return sections


def module_exports(module):
    """Return the public type names re-exported for a module, from lib.rs.

    Handles both `pub use module::{A, B, ...};` (possibly spanning lines,
    with nested braces) and `pub use module::Type;`.
    """
    text = LIB_RS.read_text(encoding="utf-8")
    exports = []
    pos = 0
    while True:
        m = re.search(rf"\bpub use {module}\s*::\s*", text[pos:])
        if not m:
            break
        start = pos + m.end()
        if start < len(text) and text[start] == "{":
            # balanced-brace capture
            depth = 0
            i = start
            while i < len(text):
                if text[i] == "{":
                    depth += 1
                elif text[i] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                i += 1
            body = text[start + 1 : i]
            pos = i + 1
        else:
            j = text.find(";", start)
            body = text[start:j]
            pos = j + 1
        for name in re.findall(r"\b([A-Z][A-Za-z0-9_]+)", body):
            if name not in exports:
                exports.append(name)
    return exports


def display_from_exports(exports):
    """Derive a human name: AnthropicProvider / AnthropicConfig -> Anthropic."""
    for prefix in ("ProviderConfig", "Provider", "Config", "Model"):
        for e in exports:
            if e.endswith(prefix):
                return e[: -len(prefix)]
    return None


def main():
    registry = load_registry()
    sections = parse_lib_rs()

    registry_names = {e["name"] for e in registry}
    lines = []
    w = lines.append

    # Overlap sanity check: a module name should not also be a registry entry.
    for _, mods in sections:
        for m in mods:
            if m in registry_names:
                print(f"WARNING: module `{m}` also exists in provider_registry.json")

    w("# aimux providers")
    w("")
    w("> **GENERATED** by `scripts/gen_providers_doc.py` — do not edit by hand.")
    w("> Regenerate with: `python scripts/gen_providers_doc.py`")
    w("")
    w(
        f"**{len(registry)} registry-backed OpenAI-compatible providers** "
        f"(construct via `provider(name, ...)` / `ProviderName`) + "
        f"**{sum(len(m) for _, m in sections) - 2} non-registry providers** "
        "(construct via the typed factories listed below)."
    )
    w("")
    w(f"## Registry-backed (OpenAI-compatible) — {len(registry)}")
    w("")
    w("| name | display | env var | base_url |")
    w("|------|---------|---------|----------|")
    for e in sorted(registry, key=lambda x: x["name"]):
        w(f"| `{e['name']}` | {e.get('display', '')} | `{e.get('env_var', '')}` | `{e.get('base_url', '')}` |")
    w("")
    w("## Typed factories (non-registry)")
    w("")
    w(
        "These providers are **not** name-addressable: `provider(\"anthropic\", ...)` "
        "fails with `NoSuchProvider`. Use the typed entry points below "
        "(Rust type names; per-binding constructors: see "
        "[reference.md](reference.md))."
    )
    w("")
    for title, mods in sections:
        mods = [m for m in mods if m not in ("provider", "provider_name")]
        if not mods:
            continue
        w(f"### {title}")
        w("")
        w("| module | typed entry points |")
        w("|--------|--------------------|")
        for mod in mods:
            exports = module_exports(mod)
            display = display_from_exports(exports)
            if display:
                cols = f"`{display}Config` / `{display}Provider`" if f"{display}Config" in exports or f"{display}Provider" in exports else ", ".join(f"`{e}`" for e in exports)
            else:
                cols = "`" + "`, `".join(exports) + "`" if exports else "—"
            w(f"| `{mod}` | {cols} |")
        w("")

    out = "\n".join(lines)
    OUT.write_text(out, encoding="utf-8")

    total = len(registry) + sum(len(m) for _, m in sections) - 2
    print(f"wrote {OUT}")
    print(f"registry={len(registry)}  non-registry modules={sum(len(m) for _, m in sections) - 2}  total={total}")


if __name__ == "__main__":
    main()
