"""
aimux vs OpenAI SDK — 大 payload + 持续压测 + 内存

用 Node.js mock server。
"""

import json
import sys
import time
import os
import httpx
import tracemalloc

from aimux import openai as aimux_openai
import openai

def make_context(approx_bytes):
    turn = "Explain Rust ownership in detail. " + "word " * 50
    turns = []
    total = 0
    i = 0
    while total < approx_bytes:
        turns.append(f"Message {i}: {turn}")
        total += len(turn) + 20
        i += 1
    return "\n".join(turns)

CTX_200K = make_context(200_000)

def bench_aimux(model, prompt_json):
    start = time.perf_counter()
    model.generate_text(prompt_json)
    return (time.perf_counter() - start) * 1000

def bench_openai(client, prompt):
    start = time.perf_counter()
    client.chat.completions.create(
        model="gpt-4o",
        messages=[{"role": "user", "content": prompt}],
        max_tokens=50,
    )
    return (time.perf_counter() - start) * 1000

def bench_b0(uri, httpx_client, prompt):
    start = time.perf_counter()
    resp = httpx_client.post(f"{uri}/v1/chat/completions",
        headers={"Content-Type": "application/json", "Authorization": "Bearer test-key"},
        json={"model": "gpt-4o", "messages": [{"role": "user", "content": prompt}], "max_tokens": 50},
    )
    resp.text
    return (time.perf_counter() - start) * 1000

def stats(samples):
    s = sorted(samples)
    n = len(s)
    def pct(p): return s[max(0, int(n * p / 100) - 1)]
    return sum(s) / n, pct(50), pct(95), pct(99), s[0]

def rss_mb():
    # get RSS in MB from /proc/self/status
    try:
        with open("/proc/self/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) // 1024  # KB → MB
    except:
        pass
    return 0

def main():
    uri = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:45703"

    aimux_model = aimux_openai("test-key", "gpt-4o", f"{uri}/v1")
    openai_client = openai.OpenAI(api_key="test-key", base_url=f"{uri}/v1")
    httpx_client = httpx.Client()

    # warmup
    prompt_json = json.dumps(CTX_200K)
    for _ in range(5): bench_aimux(aimux_model, prompt_json)
    for _ in range(5): bench_openai(openai_client, CTX_200K)

    N = 2000

    print(f"\n  Python: 持续压测 (N={N}, 200KB 上文)")
    print(f"  mock: {uri}")
    print(f"  RSS 初始: {rss_mb()} MB\n")

    # ── aimux ──
    start_rss = rss_mb()
    rss_samples = []
    latencies = []
    t_start = time.perf_counter()
    for i in range(N):
        lat = bench_aimux(aimux_model, prompt_json)
        latencies.append(lat)
        if i % 200 == 0:
            rss_samples.append(rss_mb())
    total_ms = (time.perf_counter() - t_start) * 1000
    end_rss = rss_mb()
    am_mean, am_p50, am_p95, am_p99, _ = stats(latencies)
    print(f"  aimux:")
    print(f"    总耗时:   {total_ms:.0f}ms ({N/total_ms*1000:.0f} rps)")
    print(f"    延迟:     mean={am_mean:.2f}ms P50={am_p50:.2f} P95={am_p95:.2f} P99={am_p99:.2f}")
    print(f"    内存:     {start_rss}MB → {end_rss}MB (+{end_rss-start_rss}MB)")
    print(f"    RSS:      {' → '.join(str(s) for s in rss_samples)} MB")
    print()

    # ── OpenAI SDK ──
    start_rss = rss_mb()
    rss_samples = []
    latencies = []
    t_start = time.perf_counter()
    for i in range(N):
        lat = bench_openai(openai_client, CTX_200K)
        latencies.append(lat)
        if i % 200 == 0:
            rss_samples.append(rss_mb())
    total_ms = (time.perf_counter() - t_start) * 1000
    end_rss = rss_mb()
    om_mean, om_p50, om_p95, om_p99, _ = stats(latencies)
    print(f"  OpenAI SDK:")
    print(f"    总耗时:   {total_ms:.0f}ms ({N/total_ms*1000:.0f} rps)")
    print(f"    延迟:     mean={om_mean:.2f}ms P50={om_p50:.2f} P95={om_p95:.2f} P99={om_p99:.2f}")
    print(f"    内存:     {start_rss}MB → {end_rss}MB (+{end_rss-start_rss}MB)")
    print(f"    RSS:      {' → '.join(str(s) for s in rss_samples)} MB")
    print()

    print(f"  aimux vs OpenAI = {om_mean/am_mean:.1f}x (aimux 快)")
    print()

    httpx_client.close()

if __name__ == "__main__":
    main()
