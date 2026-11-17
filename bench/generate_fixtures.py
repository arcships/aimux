#!/usr/bin/env python3
"""Generate and verify synthetic UTF-8 benchmark payloads for RFC-0043 §7.3.

Usage:
    python3 bench/generate_fixtures.py --check
    python3 bench/generate_fixtures.py --write-manifest
    python3 bench/generate_fixtures.py --out-dir target/bench-fixtures
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = ROOT / "bench" / "fixtures" / "manifest.json"
DEFAULT_OUT_DIR = ROOT / "target" / "bench-fixtures"

FIXTURE_SPECS: list[dict[str, Any]] = [
    {"id": "request-1024", "kind": "request", "exact_bytes": 1024},
    {"id": "request-10240", "kind": "request", "exact_bytes": 10240},
    {"id": "request-102400", "kind": "request", "exact_bytes": 102400},
    {"id": "request-200000", "kind": "request", "exact_bytes": 200000},
    {"id": "response-1024", "kind": "response", "exact_bytes": 1024},
    {"id": "response-50000", "kind": "response", "exact_bytes": 50000},
]

_FILLER_UNIT = (
    "Synthetic OpenAI chat benchmark payload segment for reproducible serialization "
    "and transport boundary measurement without real user prompts or secrets. "
)


def _filler_of_length(length: int) -> str:
    if length <= 0:
        return ""
    repeats = (length // len(_FILLER_UNIT)) + 1
    return (_FILLER_UNIT * repeats)[:length]


def _serialize_compact(obj: dict[str, Any]) -> bytes:
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def build_payload(spec: dict[str, Any]) -> bytes:
    """Build an exact-byte UTF-8 JSON benchmark payload for `spec`."""
    kind = spec["kind"]
    target_bytes = spec["exact_bytes"]

    def make_doc(padding: str) -> dict[str, Any]:
        if kind == "request":
            return {
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "You are a synthetic benchmark fixture."},
                    {"role": "user", "content": padding},
                ],
                "temperature": 0,
                "max_tokens": 128,
            }
        return {
            "id": "chatcmpl-bench-synthetic",
            "object": "chat.completion",
            "created": 1760000000,
            "model": "gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": padding},
                    "finish_reason": "stop",
                }
            ],
            "usage": {"prompt_tokens": 256, "completion_tokens": 64, "total_tokens": 320},
        }

    base_len = len(_serialize_compact(make_doc("")))
    remaining = target_bytes - base_len
    if remaining < 0:
        raise ValueError(f"target {target_bytes} smaller than base template {base_len}")

    raw = _serialize_compact(make_doc(_filler_of_length(remaining)))
    # ASCII quotes/backslashes could shift encoded length if ever present, so adjust if needed.
    while len(raw) != target_bytes:
        delta = target_bytes - len(raw)
        remaining += delta
        if remaining < 0:
            raise ValueError(f"cannot reach exact byte length {target_bytes}")
        raw = _serialize_compact(make_doc(_filler_of_length(remaining)))

    # Validate round-trip parseability.
    json.loads(raw.decode("utf-8"))
    return raw


def compute_manifest() -> dict[str, Any]:
    entries = []
    for spec in FIXTURE_SPECS:
        payload = build_payload(spec)
        entries.append(
            {
                "id": spec["id"],
                "kind": spec["kind"],
                "exact_bytes": spec["exact_bytes"],
                "sha256": hashlib.sha256(payload).hexdigest(),
                "filename": f"{spec['id']}.json",
            }
        )
    return {"schema_version": 1, "fixtures": entries}


def check_manifest(manifest_path: pathlib.Path) -> list[str]:
    errors: list[str] = []
    if not manifest_path.is_file():
        return [f"missing manifest file: {manifest_path}"]
    try:
        recorded = json.loads(manifest_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"invalid manifest JSON: {exc}"]

    expected = compute_manifest()
    if recorded != expected:
        rec_by_id = {f.get("id"): f for f in recorded.get("fixtures", []) if isinstance(f, dict)}
        for item in expected["fixtures"]:
            fid = item["id"]
            if rec_by_id.get(fid) != item:
                errors.append(
                    f"fixture {fid} manifest mismatch: expected={item}, got={rec_by_id.get(fid)}"
                )
        if not errors:
            errors.append("fixture manifest top-level structure differs")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Generate and verify RFC-0043 §7.3 bench fixtures.")
    parser.add_argument("--manifest", type=pathlib.Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--check", action="store_true", help="Verify payloads match manifest.")
    parser.add_argument("--write-manifest", action="store_true", help="Write computed manifest JSON.")
    parser.add_argument("--out-dir", type=pathlib.Path, help="Directory to write generated fixtures.")
    args = parser.parse_args(argv)

    if args.write_manifest:
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(
            json.dumps(compute_manifest(), indent=2) + "\n", encoding="utf-8"
        )
        print(f"wrote {args.manifest}")
        return 0

    if args.out_dir is not None:
        args.out_dir.mkdir(parents=True, exist_ok=True)
        manifest = compute_manifest()
        for spec, entry in zip(FIXTURE_SPECS, manifest["fixtures"]):
            payload = build_payload(spec)
            assert len(payload) == entry["exact_bytes"]
            (args.out_dir / entry["filename"]).write_bytes(payload)
        print(f"generated {len(FIXTURE_SPECS)} fixtures into {args.out_dir}")

    if args.check or (not args.write_manifest and args.out_dir is None):
        errors = check_manifest(args.manifest)
        if errors:
            for err in errors:
                print(err, file=sys.stderr)
            return 1
        print(f"bench fixtures ok ({len(FIXTURE_SPECS)} payloads verified)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
