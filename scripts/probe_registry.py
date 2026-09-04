#!/usr/bin/env python3
"""Probe every registry base_url for reachability (unauthenticated).

Sends `GET {base_url}/models` with no credentials and classifies the outcome:
alive (any HTTP status came back — 401/403/404 all prove a server is there),
server-error (5xx), dead (DNS failure, refused connection, timeout), skipped
(a templated base_url the caller has to fill in). Prints the dead and
server-error rows as Markdown; a clean run prints nothing but the summary.

Read-only: it never touches provider_registry.json. A dead row is a prompt for
a human to check the vendor, not a licence to delete the row.

Usage:
    python3 scripts/probe_registry.py
    python3 scripts/probe_registry.py --only groq,deepseek --json
"""

import argparse
import json
import socket
import sys
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "aimux-providers" / "src" / "provider_registry.json"


def probe(entry, timeout):
    """Return (name, verdict, detail)."""
    name, base = entry["name"], entry["base_url"]
    # Same placeholder forms provider.rs::base_url_has_placeholder rejects.
    if "{" in base or "<" in base:
        return name, "skipped", "templated base_url"
    url = base.rstrip("/") + "/models"
    req = urllib.request.Request(url, method="GET", headers={"User-Agent": "aimux-probe"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 — registry URLs
            return name, "alive", f"HTTP {resp.status}"
    except urllib.error.HTTPError as e:
        # A status is a status: the host answered, so the base URL resolves.
        return name, ("server-error" if e.code >= 500 else "alive"), f"HTTP {e.code}"
    except (urllib.error.URLError, socket.timeout, OSError) as e:
        reason = getattr(e, "reason", e)
        return name, "dead", f"{type(reason).__name__ if isinstance(reason, Exception) else 'error'}: {reason}"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--timeout", type=float, default=8, help="per-request timeout in seconds (default 8)")
    ap.add_argument("--concurrency", type=int, default=16, help="parallel requests (default 16)")
    ap.add_argument("--only", help="comma-separated provider names to probe instead of all")
    ap.add_argument("--json", action="store_true", help="also print a JSON summary")
    args = ap.parse_args()

    entries = json.loads(REGISTRY.read_text(encoding="utf-8"))
    if args.only:
        wanted = {n.strip() for n in args.only.split(",") if n.strip()}
        entries = [e for e in entries if e["name"] in wanted]
        unknown = wanted - {e["name"] for e in entries}
        if unknown:
            # Native-protocol providers (openai, anthropic, ...) are not registry
            # rows, so a name can be real and still not be probeable here.
            print(f"not registry rows, skipped: {', '.join(sorted(unknown))}", file=sys.stderr)
        if not entries:
            print("--only matched no registry rows", file=sys.stderr)
            return 1

    with ThreadPoolExecutor(max_workers=max(1, args.concurrency)) as pool:
        results = sorted(pool.map(lambda e: probe(e, args.timeout), entries))

    counts = {}
    for _, verdict, _ in results:
        counts[verdict] = counts.get(verdict, 0) + 1
    bad = [r for r in results if r[1] in ("dead", "server-error")]

    print(f"## Unreachable registry base URLs — {len(bad)} of {len(results)} probed")
    print()
    if bad:
        print("| name | verdict | detail |")
        print("|---|---|---|")
        for name, verdict, detail in bad:
            print(f"| `{name}` | {verdict} | {detail} |")
    else:
        print("_none._")
    print()
    print("_" + ", ".join(f"{k}: {v}" for k, v in sorted(counts.items())) + "._")

    if args.json:
        print()
        print(json.dumps({"counts": counts, "results": [dict(zip(("name", "verdict", "detail"), r)) for r in results]}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
