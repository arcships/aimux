"""Convert pydantic-ai YAML cassettes to our JSON cassette format.

pydantic-ai records real HTTP interactions in YAML under
``tests/{models,cassettes,providers}/...``. Each file has an ``interactions``
list; each interaction carries a ``request`` (``uri``, ``method``,
``parsed_body``, ``headers``) and a ``response`` (``status.code``,
``parsed_body``, optional raw ``body`` string for SSE).

We split each interaction into one JSON cassette, keyed by the *real* provider
inferred from the request ``uri`` host — NOT the test directory name, because
pydantic-ai reuses some test dirs across providers (e.g. ``test_openrouter``
contains deepseek requests).

Usage: uv run python scripts/convert_pydantic_ai.py
"""

import json
import shutil
from pathlib import Path
from urllib.parse import urlparse

import yaml

SRC_ROOT = Path("reference/pydantic-ai/tests")
OUT_DIR = Path("aimux-providers/tests/cassettes")
SOURCE_TAG = "pydantic-ai (MIT)"

# Host substring -> our provider directory name. A request is routed to the
# first matching provider; anything unmatched is skipped with a warning so we
# can see what coverage we're missing.
HOST_TO_PROVIDER: list[tuple[str, str]] = [
    ("api.openai.com", "openai"),
    ("api.anthropic.com", "anthropic"),
    ("generativelanguage.googleapis.com", "gemini"),
    ("api.x.ai", "xai"),
    ("api.groq.com", "groq"),
    ("api.mistral.ai", "mistral"),
    ("api.deepseek.com", "deepseek"),
    ("openrouter.ai", "openrouter"),
    ("api.cerebras.ai", "cerebras"),
    ("api.cohere.com", "cohere"),
    ("router.huggingface.co", "huggingface"),
    ("127.0.0.1:11434", "ollama"),
    ("ollama.com", "ollama"),
    ("api.z.ai", "zai"),
    ("bedrock-runtime", "bedrock"),
    ("api.githubcopilot.com", "copilot"),
    ("api.perplexity.ai", "perplexity"),
]


def provider_for_uri(uri: str) -> str | None:
    host = urlparse(uri).netloc.lower()
    for needle, provider in HOST_TO_PROVIDER:
        if needle in host:
            return provider
    return None


def convert_interaction(
    provider: str, scenario: str, turn: int, interaction: dict
) -> dict | None:
    req = interaction.get("request", {}) or {}
    resp = interaction.get("response", {}) or {}

    uri = req.get("uri", "")
    path = urlparse(uri).path or "/"
    method = (req.get("method") or "POST").upper()

    # request body: parsed_body is already a JSON object; keep as-is so the
    # replay scorer can match on scalar fields (model, stream, ...).
    req_body = req.get("parsed_body")
    if req_body is None:
        req_body = {}

    # request headers: {name: [val, ...]} -> {name: val}
    req_headers = {}
    for name, vals in (req.get("headers") or {}).items():
        if isinstance(vals, list) and vals:
            req_headers[name.lower()] = vals[0]
        elif isinstance(vals, str):
            req_headers[name.lower()] = vals

    # response status
    status = (resp.get("status") or {}).get("code", 200)

    # response body: prefer the raw `body` string (SSE text) when non-empty;
    # otherwise serialize parsed_body to JSON.
    raw_body = resp.get("body")
    if isinstance(raw_body, str) and raw_body:
        resp_body = raw_body
    else:
        parsed = resp.get("parsed_body")
        resp_body = json.dumps(parsed, default=str) if parsed is not None else ""

    # response headers
    resp_headers = {}
    for name, vals in (resp.get("headers") or {}).items():
        if isinstance(vals, list) and vals:
            resp_headers[name.lower()] = vals[0]
        elif isinstance(vals, str):
            resp_headers[name.lower()] = vals

    # Drop hop-by-hop / length-sensitive headers that the mock server must
    # recompute from the actual replayed body. pydantic-ai records the original
    # content-length from the real response, which differs from the cassette
    # body after redaction and triggers hyper's "payload claims content-length
    # ... custom content-length header claims ..." panic.
    for h in ("content-length", "transfer-encoding", "content-encoding"):
        resp_headers.pop(h, None)

    turn_suffix = f"_{turn}" if turn > 0 else ""
    return {
        "source": SOURCE_TAG,
        "provider": provider,
        "scenario": f"{scenario}{turn_suffix}",
        "request": {
            "path": path,
            "method": method,
            "headers": req_headers,
            "body": req_body,
        },
        "response": {
            "status": status,
            "headers": resp_headers,
            "body": resp_body,
        },
    }


def main() -> None:
    if not SRC_ROOT.exists():
        print(f"Error: {SRC_ROOT} not found")
        return

    # Collect every *.yaml under the test roots.
    yaml_files = sorted(SRC_ROOT.rglob("*.yaml"))
    print(f"Scanning {len(yaml_files)} pydantic-ai cassette files...")

    converted = 0
    skipped_unmatched = 0
    per_provider: dict[str, int] = {}
    unmatched_hosts: dict[str, int] = {}

    # We do NOT wipe existing cassettes (rig-sourced ones stay); we only add
    # pydantic-ai ones. To avoid stale pydantic-ai outputs from a previous run,
    # delete files we previously generated (tagged with our source) per dir.
    for provider_dir in OUT_DIR.iterdir():
        if not provider_dir.is_dir():
            continue
        for f in provider_dir.glob("*.json"):
            try:
                data = json.loads(f.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                continue
            if data.get("source") == SOURCE_TAG:
                f.unlink()

    for yaml_path in yaml_files:
        try:
            with open(yaml_path, "r", encoding="utf-8") as f:
                docs = list(yaml.safe_load_all(f))
        except (OSError, yaml.YAMLError) as e:
            print(f"  skip {yaml_path}: {e}")
            continue

        scenario = yaml_path.stem
        for doc in docs:
            if not doc:
                continue
            interactions = doc.get("interactions") or []
            for i, interaction in enumerate(interactions):
                uri = (interaction.get("request") or {}).get("uri", "")
                provider = provider_for_uri(uri)
                if provider is None:
                    host = urlparse(uri).netloc
                    unmatched_hosts[host] = unmatched_hosts.get(host, 0) + 1
                    skipped_unmatched += 1
                    continue

                out = convert_interaction(provider, scenario, i, interaction)
                if out is None:
                    continue

                out_path = OUT_DIR / provider / f"{out['scenario']}.json"
                # Disambiguate collisions across source dirs (e.g. two dirs both
                # have completion_smoke) by prefixing the parent dir name.
                if out_path.exists():
                    parent = yaml_path.parent.name
                    out["scenario"] = f"{parent}_{out['scenario']}"
                    out_path = OUT_DIR / provider / f"{out['scenario']}.json"

                out_path.parent.mkdir(parents=True, exist_ok=True)
                out_path.write_text(
                    json.dumps(out, ensure_ascii=False, indent=2, default=str),
                    encoding="utf-8",
                )
                converted += 1
                per_provider[provider] = per_provider.get(provider, 0) + 1

    print(f"\nConverted {converted} cassettes to {OUT_DIR}")
    print(f"Skipped {skipped_unmatched} interactions with no matching provider")
    print("\nPer provider:")
    for p in sorted(per_provider):
        print(f"  {p}: {per_provider[p]}")
    if unmatched_hosts:
        print("\nUnmatched hosts (skipped):")
        for h in sorted(unmatched_hosts):
            print(f"  {h}: {unmatched_hosts[h]}")


if __name__ == "__main__":
    main()
