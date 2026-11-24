#!/usr/bin/env python3
"""Run identical RFC-0049/0050 benchmark recipes against two built revisions."""

import argparse
import hashlib
import json
from pathlib import Path
import os
import platform
import random
import shlex
import signal
import statistics
import subprocess


def digest(path):
    result = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def watchdog(command, timeout):
    # Kill the benchmark and its loopback mock together on timeout.
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True, start_new_session=True) as process:
        try:
            output, _ = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            raise
        if process.returncode:
            raise subprocess.CalledProcessError(process.returncode, command)
    return json.loads(output)


def summarize(result):
    expected = result["samples"]
    if "observations" in result:
        rows = result["observations"]
        metrics = {"elapsed_ns": [row["elapsed_ns"] for row in rows]}
        for key in ("ttft_ns", "delta_window_ns"):
            if rows and key in rows[0]:
                metrics[key] = [row[key] for row in rows]
    else:
        metrics = {
            key: result[key] for key in ("elapsed_ns", "allocation_calls", "allocated_bytes")
        }
    if expected < 1 or any(len(values) != expected for values in metrics.values()):
        raise ValueError("benchmark emitted an incorrect number of samples")
    return {key: statistics.median(values) for key, values in metrics.items()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--command", required=True, help="command template containing {artifact}")
    for side in ("base", "head"):
        parser.add_argument(f"--{side}", required=True, help="path substituted for {artifact}")
        parser.add_argument(f"--{side}-sha", required=True)
        parser.add_argument(f"--{side}-digest-file", required=True, help="built binary to hash")
    parser.add_argument("--recipe-file", action="append", required=True, help="harness or lockfile to hash")
    parser.add_argument("--pairs", default=10, type=int)
    parser.add_argument("--seed", default=49, type=int)
    parser.add_argument("--run-timeout-s", default=600, type=int)
    parser.add_argument("--output", required=True, type=Path, help="new JSONL evidence file")
    args = parser.parse_args()
    tokens = shlex.split(args.command)
    if not any("{artifact}" in token for token in tokens):
        parser.error("--command must contain {artifact}")
    if args.pairs < 1 or args.run_timeout_s < 1:
        parser.error("pairs and run timeout must be positive")

    sources = {side: getattr(args, f"{side}_sha") for side in ("base", "head")}
    commands = {
        side: [token.replace("{artifact}", getattr(args, side)) for token in tokens]
        for side in sources
    }
    order = [["base", "head"] if i % 2 == 0 else ["head", "base"] for i in range(args.pairs)]
    random.Random(args.seed).shuffle(order)
    contract = None

    with args.output.open("x") as output:
        def write(record):
            print(json.dumps(record), file=output, flush=True)

        write({
            "kind": "recipe",
            "host": platform.platform(),
            "machine": platform.machine(),
            "runner_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
            "seed": args.seed,
            "run_timeout_s": args.run_timeout_s,
            "commands": commands,
            "sources": sources,
            "artifacts": {side: digest(getattr(args, f"{side}_digest_file")) for side in sources},
            "recipe_files": {name: digest(name) for name in args.recipe_file},
        })
        for index, pair in enumerate(order, start=1):
            for side in pair:
                try:
                    result = watchdog(commands[side], args.run_timeout_s)
                    identity = [result.get(key) for key in (
                        "scenario", "samples", "warmup", "concurrency", "operation_timeout_ms",
                    )]
                    if contract is None:
                        contract = identity
                    if contract != identity:
                        raise ValueError(f"benchmark recipe changed during comparison: {identity}")
                    summary = summarize(result)
                except (subprocess.SubprocessError, ValueError) as error:
                    write({"kind": "failure", "pair": index, "side": side, "error": str(error)})
                    raise
                write({"kind": "run", "pair": index, "side": side, "result": result, "median": summary})


if __name__ == "__main__":
    main()
