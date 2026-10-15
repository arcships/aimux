"""Measure a configured ops call in the same process as its handles.

Usage: python bench_nonstream.py REQUEST_JSON_FILE [COUNT]
File: {"provider": <provider.create params>, "model": <provider.model params>,
       "op": "text.generate", "params": <request DTO>}
"""
import json
import statistics
import sys
import time
from aimux import Client

with open(sys.argv[1]) as file:
    request = json.load(file)
count = int(sys.argv[2]) if len(sys.argv) > 2 else 100
client = Client()
samples = []
with client.adopt(client.call('provider.create', request['provider'])['handle']) as provider:
    with client.adopt(provider.call('provider.model', request['model'])['handle']) as model:
        for _ in range(count):
            started = time.perf_counter()
            model.call(request['op'], request['params'])
            samples.append(time.perf_counter() - started)
print(json.dumps({'count': count, 'medianSeconds': statistics.median(samples)}))
