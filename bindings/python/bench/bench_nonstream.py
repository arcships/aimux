"""
aimux (PyO3 → Rust) vs OpenAI Python SDK — 性能对比

用 Node.js 的 mock server（已在 bench/mock-server.ts 验证可用）。
Python 端三路测量：
  B0  — httpx 直调 mock（无 SDK 基线）
  aimux — PyO3 → Rust → reqwest → mock
  openai — OpenAI Python SDK → httpx → mock

启动方式：
  1. 先启动 node mock: cd bindings/node && npx tsx bench/mock-server.ts &
  2. 再跑: python bench/bench_nonstream.py <mock-uri>
  3. 或不传参数，脚本自己判断
"""

import json
import sys
import time
import httpx

from aimux import openai as aimux_openai
import openai

# ── bench 函数 ────────────────────────────────────────────────────────────

def bench_b0(uri: str, client: httpx.Client) -> float:
    start = time.perf_counter()
    resp = client.post(f"{uri}/v1/chat/completions",
        headers={"Content-Type": "application/json", "Authorization": "Bearer test-key"},
        json={"model": "gpt-4o", "messages": [{"role": "user", "content": "Explain Rust ownership."}], "max_tokens": 50},
    )
    resp.text
    return (time.perf_counter() - start) * 1000

def bench_aimux(model) -> float:
    prompt = json.dumps("Explain Rust ownership.")
    start = time.perf_counter()
    model.generate_text(prompt)
    return (time.perf_counter() - start) * 1000

def bench_openai(client: openai.OpenAI) -> float:
    start = time.perf_counter()
    client.chat.completions.create(
        model="gpt-4o",
        messages=[{"role": "user", "content": "Explain Rust ownership."}],
        max_tokens=50,
    )
    return (time.perf_counter() - start) * 1000

# ── 统计 ─────────────────────────────────────────────────────────────────

def stats(samples):
    s = sorted(samples)
    n = len(s)
    def pct(p): return s[max(0, int(n * p / 100) - 1)]
    return sum(s) / n, pct(50), pct(95), pct(99), s[0]

# ── 主流程 ────────────────────────────────────────────────────────────────

def main():
    # 从命令行获取 mock uri，或用默认
    uri = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:38000"
    N = 200
    WARMUP = 20

    # init
    aimux_model = aimux_openai("test-key", "gpt-4o", f"{uri}/v1")
    openai_client = openai.OpenAI(api_key="test-key", base_url=f"{uri}/v1")
    httpx_client = httpx.Client()

    # 测试连接
    try:
        resp = httpx_client.post(f"{uri}/v1/chat/completions",
            headers={"Content-Type": "application/json", "Authorization": "Bearer test-key"},
            json={"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "max_tokens": 5},
            timeout=3.0)
    except Exception as e:
        print(f"  无法连接 mock server ({uri}): {e}")
        print(f"  请先启动: cd bindings/node && npx tsx bench/mock-server.ts")
        print(f"  然后查看端口，传入: python bench/bench_nonstream.py http://127.0.0.1:<PORT>")
        return

    print(f"\n  Python: aimux vs OpenAI SDK (N={N}, warmup={WARMUP})")
    print(f"  mock: {uri}\n")

    results = {}

    # B0
    for _ in range(WARMUP): bench_b0(uri, httpx_client)
    samples = [bench_b0(uri, httpx_client) for _ in range(N)]
    results["B0 httpx"] = stats(samples)
    print(f"  B0 httpx:      mean={results['B0 httpx'][0]:.3f}ms")

    # aimux
    for _ in range(WARMUP): bench_aimux(aimux_model)
    samples = [bench_aimux(aimux_model) for _ in range(N)]
    results["aimux"] = stats(samples)
    print(f"  aimux:        mean={results['aimux'][0]:.3f}ms")

    # openai
    for _ in range(WARMUP): bench_openai(openai_client)
    samples = [bench_openai(openai_client) for _ in range(N)]
    results["OpenAI SDK"] = stats(samples)
    print(f"  OpenAI SDK:   mean={results['OpenAI SDK'][0]:.3f}ms")

    # 打印
    print(f"\n  {'SDK':<14} {'mean':>8} {'P50':>8} {'P95':>8} {'P99':>8} {'min':>8}")
    print(f"  {'─'*14} {'─'*8} {'─'*8} {'─'*8} {'─'*8} {'─'*8}")
    for name, (mean, p50, p95, p99, mn) in results.items():
        print(f"  {name:<14} {mean:>8.3f} {p50:>8.3f} {p95:>8.3f} {p99:>8.3f} {mn:>8.3f}")

    b0 = results["B0 httpx"][0]
    am = results["aimux"][0]
    om = results["OpenAI SDK"][0]
    print(f"\n  B0 (无 SDK)    = {b0:.3f}ms")
    print(f"  aimux 开销     = {am - b0:+.3f}ms  (含 PyO3 FFI)")
    print(f"  OpenAI 开销    = {om - b0:+.3f}ms")
    print(f"  aimux vs OpenAI = {om / am:.1f}x  ({'aimux 快' if am < om else 'OpenAI 快'})")
    print()

    httpx_client.close()

if __name__ == "__main__":
    main()
