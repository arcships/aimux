"""Convert rig YAML cassettes to our JSON format.

Usage: uv run python scripts/convert_cassettes.py
"""

import json
import os
from pathlib import Path

import yaml

RIG_DIR = Path("reference/rig/tests/cassettes")
OUT_DIR = Path("aimux-providers/tests/cassettes")


def convert_one(yaml_path: Path) -> list[dict]:
    """Convert a YAML cassette file. May contain multiple documents."""
    with open(yaml_path, "r", encoding="utf-8") as f:
        docs = list(yaml.safe_load_all(f))

    results = []
    for i, data in enumerate(docs):
        if data is None:
            continue
        when = data.get("when", {})
        then = data.get("then", {})

        # headers: list[{name, value}] -> dict
        req_headers = {}
        for h in when.get("header", []):
            req_headers[h["name"]] = h["value"]

        resp_headers = {}
        for h in then.get("header", []):
            resp_headers[h["name"]] = h["value"]

        # request body: try JSON parse
        req_body_raw = when.get("body", "")
        if isinstance(req_body_raw, str):
            try:
                req_body = json.loads(req_body_raw)
            except (json.JSONDecodeError, ValueError):
                req_body = req_body_raw
        else:
            req_body = json.loads(json.dumps(req_body_raw, default=str))

        # response body: keep raw text
        resp_body = then.get("body", "")
        if not isinstance(resp_body, str):
            resp_body = json.dumps(resp_body, default=str)

        scenario = yaml_path.stem
        if len(docs) > 1:
            scenario = f"{scenario}_{i}"

        results.append({
            "source": "rig (MIT)",
            "provider": yaml_path.parent.parent.name,
            "scenario": scenario,
            "request": {
                "path": when.get("path", ""),
                "method": when.get("method", "POST"),
                "headers": req_headers,
                "body": req_body,
            },
            "response": {
                "status": then.get("status", 200),
                "headers": resp_headers,
                "body": resp_body,
            },
        })
    return results


def main():
    if not RIG_DIR.exists():
        print(f"Error: {RIG_DIR} not found")
        return

    count = 0
    for yaml_path in sorted(RIG_DIR.rglob("*.yaml")):
        for result in convert_one(yaml_path):
            out_path = OUT_DIR / result["provider"] / f"{result['scenario']}.json"
            out_path.parent.mkdir(parents=True, exist_ok=True)

            with open(out_path, "w", encoding="utf-8") as f:
                json.dump(result, f, ensure_ascii=False, indent=2, default=str)

            count += 1

    print(f"Converted {count} cassettes to {OUT_DIR}")


if __name__ == "__main__":
    main()
