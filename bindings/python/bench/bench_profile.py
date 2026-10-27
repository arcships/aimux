"""Paired micro-benchmark of one built Python extension (RFC-0043 B-profile).

Usage: python3 bench/bench_profile.py /path/to/libaimux.so [COUNT]

Loads the given cdylib as the `aimux` package the way tests/run_real.py does,
then times six scenarios against the test suite's localhost mocks (no network):
text.generate and text.stream over small and large bodies, model.info and
provider.create+drop with no HTTP at all. Prints one JSON line with the median
and p90 in microseconds per scenario and the library's byte size. Run two
builds alternately, several rounds each, and compare medians; a single run is
not a comparison.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

binding = Path(__file__).resolve().parents[1]
so = sys.argv[1]
count = int(sys.argv[2]) if len(sys.argv) > 2 else 300

INNER = r'''
import json, statistics, sys, time
sys.path.insert(0, %(tests)r)
from ops_fixtures import slow_http, OPENAI_RESPONSES, OPENAI_RESPONSES_STREAM
from gil_ops import model_at
import aimux
from aimux import ops

N = %(count)d
OPTS = {"maxRetries": 0, "timeout": {"totalMs": 5000}}

def stats(samples):
    s = sorted(samples)
    return {"median_us": round(statistics.median(s) * 1e6, 1), "p90_us": round(s[int(len(s) * 0.9)] * 1e6, 1)}

def timed(fn, warmup=20):
    for _ in range(warmup):
        fn()
    t = []
    for _ in range(N):
        a = time.perf_counter(); fn(); t.append(time.perf_counter() - a)
    return stats(t)

# A 64 KB text body and a 400-delta stream make the parse cost visible.
heavy = json.loads(OPENAI_RESPONSES)
heavy["output"][0]["content"][0]["text"] = "Rust is a systems programming language. " * 1600
HEAVY_JSON = json.dumps(heavy)
frames = OPENAI_RESPONSES_STREAM.split("\n\n")
delta = "event: response.output_text.delta\ndata: " + json.dumps({
    "type": "response.output_text.delta", "output_index": 0, "item_id": "msg-test",
    "content_index": 0, "delta": "word "})
HEAVY_SSE = "\n\n".join(frames[:2] + [delta] * 400 + frames[4:])

out = {}
def generate(body, label):
    with slow_http(body, "application/json", delay=0) as url, model_at(url, model_id="gpt-4o") as model:
        out[label] = timed(lambda: model.call(ops.OP_TEXT_GENERATE, {"prompt": "hi", "options": OPTS}))
        if label == "text.generate":
            out["model.info"] = timed(lambda: model.call(ops.OP_MODEL_INFO))
def stream(body, label):
    with slow_http(body, "text/event-stream", delay=0) as url, model_at(url, model_id="gpt-4o") as model:
        def one():
            with model.stream(ops.OP_TEXT_STREAM, {"prompt": "hi", "options": OPTS}) as events:
                list(events)
        out[label] = timed(one)
generate(OPENAI_RESPONSES, "text.generate")
stream(OPENAI_RESPONSES_STREAM, "text.stream")
generate(HEAVY_JSON, "text.generate(64KB)")
stream(HEAVY_SSE, "text.stream(400 deltas)")
client = aimux.Client()
def create_drop():
    handle = client.call(ops.OP_PROVIDER_CREATE, {"package": "openai", "settings": {"apiKey": "k"}})["handle"]
    client.call(ops.OP_HANDLE_DROP, handle=handle)
out["provider.create+drop"] = timed(create_drop)
print(json.dumps(out))
'''

with tempfile.TemporaryDirectory(prefix='aimux-bench-') as folder:
    package = Path(folder) / 'aimux'
    shutil.copytree(binding / 'python/aimux', package, ignore=shutil.ignore_patterns('__pycache__', '*.so'))
    shutil.copyfile(so, package / ('aimux.pyd' if sys.platform == 'win32' else 'aimux.abi3.so'))
    env = dict(os.environ, PYTHONPATH=folder, AIMUX_REAL_OPS='1')
    env.pop('AIMUX_OPS_DOUBLE', None)
    inner = INNER % {'tests': str(binding / 'tests'), 'count': count}
    run = subprocess.run([sys.executable, '-c', inner], env=env, capture_output=True, text=True, timeout=900)
    if run.returncode:
        sys.exit(run.stderr[-4000:])
    result = json.loads(run.stdout.strip().splitlines()[-1])
    print(json.dumps({'library': so, 'bytes': os.path.getsize(so), **result}))
