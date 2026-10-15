"""Measure bounded streaming consumption in the process owning its handles.

Usage: python bench_stress.py REQUEST_JSON_FILE
File: {"provider": <provider.create params>, "model": <provider.model params>,
       "op": "text.stream", "params": <request DTO>}
"""
import json
import sys
import time
from aimux import Client

with open(sys.argv[1]) as file:
    request = json.load(file)
client = Client()
count = 0
with client.adopt(client.call('provider.create', request['provider'])['handle']) as provider:
    with client.adopt(provider.call('provider.model', request['model'])['handle']) as model:
        started = time.perf_counter()
        with model.stream(request['op'], request['params']) as stream:
            for _ in stream:
                count += 1
        elapsed = time.perf_counter() - started
print(json.dumps({'events': count, 'seconds': elapsed}))
