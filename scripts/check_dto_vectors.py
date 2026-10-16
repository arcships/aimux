#!/usr/bin/env python3
"""Run DTO codecs; successful records include `file`, `ok`, and re-encoded `value`."""
import argparse
from decimal import Decimal
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
RUNNERS = {}  # language: command (argv), run from ROOT


def same(left, right):
    if type(left) is not type(right):
        return type(left) in (int, float, Decimal) and type(right) in (int, float, Decimal) and left == right
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(same(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(same(a, b) for a, b in zip(left, right))
    return left == right


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("languages", nargs="*")
    args = parser.parse_args()
    directory = ROOT / "contract-tests/dto"
    index = json.loads((directory / "index.json").read_text())
    expected = {}
    for entry in index:
        name = entry["file"]
        path = (directory / name).resolve()
        if not path.is_relative_to(directory.resolve()) or name in expected:
            raise ValueError(f"unsafe or duplicate vector: {name}")
        expected[name] = json.loads(path.read_text(), parse_float=Decimal)
    if args.list:
        print(f"{len(index)} vectors, {len({v['type'] for v in index})} types")
        return 0
    for language in args.languages or RUNNERS:
        if language not in RUNNERS:
            raise ValueError(f"missing runner: {language}")
        result = subprocess.run(RUNNERS[language], cwd=ROOT, text=True, capture_output=True)
        if result.returncode:
            raise ValueError(f"{language}: runner failed\n{result.stderr}")
        seen = set()
        for line in result.stdout.splitlines():
            record = json.loads(line, parse_float=Decimal)
            name = record.get("file")
            if name not in expected or name in seen:
                raise ValueError(f"{language}: unexpected or duplicate vector: {name}")
            seen.add(name)
            if record.get("ok") is not True or "error" in record:
                raise ValueError(f"{language}: {name}: {record.get('error', 'codec failed')}")
            if "value" not in record or not same(record["value"], expected[name]):
                raise ValueError(f"{language}: {name}: re-encoded value differs")
        if seen != expected.keys():
            raise ValueError(f"{language}: missing vectors: {sorted(expected.keys() - seen)}")
        print(f"{language}: {len(seen)} vectors passed")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
