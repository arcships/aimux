# RFC-0010: Request Performance Comparison — aimux vs Vercel AI SDK

> **Status**: IMPLEMENTED (2026-07-30 — bench suite in [bindings/node/bench/](../bindings/node/bench/) + [bindings/python/bench/](../bindings/python/bench/), results in [docs/PERF-RESULTS.md](../docs/PERF-RESULTS.md), commit 17ddffd0)
> **Date**: 2026-07-30
> **Scope**: Design a reproducible request-performance benchmark to compare, under a unified measurement baseline, the "unified access layer" overhead of aimux (Rust core + napi Node binding) versus the Vercel AI SDK (pure TS); cover three dimensions: **speed, structural overhead, and concurrency capacity**; produce landing steps and fairness constraints
> **Related**: [RFC-0009](0009-request-resilience.md) request-layer optimization (a prerequisite for comparison), [RFC-0003](0003-test-cassette.md) cassette test plan (source of mock data), [RFC-0001](0001-multilang-bindings.md) multilang bindings

## 1. Motivation

aimux positions itself in [Cargo.toml](../Cargo.toml) as `"Rust alternative to Vercel AI SDK"`. But the performance claims of the "Rust alternative" still have **no reproducible measurement support** to date—there is neither a benchmark directory nor comparison data with AISDK. This RFC defines a benchmark suite to answer:

> Under the same upstream and the same load, what are the latency / structural-overhead / concurrency-capacity differences of aimux's "unified access layer" relative to the Vercel AI SDK?

**What is and isn't being compared**:

| Dimension | Description |
|---|---|
| ✅ Compared | The SDK's own protocol conversion, request construction, serialization, and streaming parsing overhead |
| ✅ Compared | The extra overhead brought by the unified abstraction layer (whether "converging 172 providers" incurs a performance tax) |
| ❌ Not compared | Which language is faster (Rust vs JS has no cross-language direct-comparison meaning) |
| ❌ Not compared | The upstream LLM's own performance (flattened with mocks) |

## 2. Core Challenge: Numbers Cannot Be Directly Compared Across Languages

aimux's core is Rust; AISDK is pure TS. If you run aimux with Rust's `#[bench]`/criterion and AISDK with JS's mitata, the two sides' **timers, runtimes, FFI boundaries, and memory models** are all inconsistent, and the numbers are not comparable.

**The only fair comparison posture**: within **the same Node.js process**, facing **the same local mock server**, let aimux go through its real production path (Node app → napi → Rust core → reqwest → HTTP), and AISDK go through (Node app → TS core → undici → HTTP). Both share the Node event loop and the same measurement baseline.

aimux already has all the prerequisites for this path:

| Prerequisite | Current status |
|---|---|
| Node binding available | [bindings/node/](../bindings/node/) exports `generateText(model, prompt)` / `streamText(model, prompt)` ([bindings/node/src/index.ts](../bindings/node/src/index.ts#L94)) |
| Local mock infrastructure | 100+ Rust tests use wiremock mock servers; cassettes can be replayed (see `aimux-providers/tests/`) |
| Comparison target accessible | `reference/ai/` contains the full Vercel AI SDK source |
| Design semantics share an origin | aimux's `retry.rs` comment explicitly says "Mirrors the TS SDK's `getRetryDelayInMs` / `retryWithExponentialBackoffRespectingRetryHeaders`"; the two SDKs' semantics are aligned, so the comparison is meaningful |

## 3. Comparison Architecture

```
                  ┌─────────────────────────────────────┐
                  │   Node.js benchmark process         │
                  │   Unified timing: mitata / tinybench│
                  │   Unified warmup + unified GC control│
                  └────────────┬───────────────┬────────┘
              ┌────────────────┘               └─────────────────┐
              ▼                                                  ▼
   ┌──────────────────────┐                          ┌──────────────────────┐
   │ aimux (napi → Rust)   │                          │ @ai-sdk/openai (TS)  │
   │  generateText(model,  │                          │  generateText({      │
   │    prompt)            │                          │    model, prompt })  │
   │  streamText(...)      │                          │  streamText({...})   │
   └──────────┬───────────┘                          └──────────┬───────────┘
              │  reqwest + rustls-tls                            │  undici
              │  (via napi FFI boundary)                           │  (pure JS)
              └────────────────────┬──────────────────────────────┘
                                   ▼
                  ┌─────────────────────────────────────┐
                  │   local mock server                 │
                  │   Fixed JSON / replayable SSE        │
                  │   (flattens network RTT and LLM time) │
                  └─────────────────────────────────────┘
```

**Key design**: the mock server returns **fixed responses** (fixed JSON for non-streaming; fixed-shard SSE recording replay for streaming). After turning network RTT and LLM generation duration into constants, the remaining difference is the two SDKs' own protocol-conversion / serialization / stream-parsing overhead.

To strip out aimux's napi FFI boundary cost, add a third baseline:

| Baseline | Implementation purpose |
|---|---|
| **B0. Pure Node direct call to mock HTTP** | Use `undici.request` to hit the mock server directly, as the "no SDK" baseline. aimux number − B0 = aimux access layer + FFI overhead; aimux number − AISDK number = aimux's net difference relative to AISDK; AISDK − B0 = AISDK access-layer overhead |

## 4. Three Comparison Dimensions

The three dimensions complement each other: **speed** measures user-perceived end-to-end latency; **structural overhead** measures the SDK's own CPU tax (stripping the network, locating optimization space); **concurrency capacity** measures scaled behavior. All three are included in the comparison; none can be omitted.

### 4.1 Dimension One: Speed (End-to-end Latency)

Includes network round-trips, reflecting user perception under real load.

| Scenario | What is measured | Why it is representative | Prerequisites |
|---|---|---|---|
| **A. Non-streaming single request** | One `generateText` end-to-end latency (P50/P95/P99) | Baseline, the most common call form | None |
| **B. Streaming TTFT + tokens/s** | First-token latency, steady-state throughput, P99 tail latency | aimux's SSE parsing is in Rust (`aimux-stream`), AISDK's is in JS; this is where the Rust core can best show its advantage | SSE mock replayer |

### 4.2 Dimension Two: Structural Overhead (Pure CPU)

This is the most persuasive comparison point for the "unified access layer": aimux converges 172 providers into a unified `LanguageModel` interface; how much CPU does this convergence cost. **The key is to strip the network**—only measure the SDK's own protocol conversion / serialization / parsing, with no network round-trips mixed in.

**Two measurement approaches**:

1. **Difference method (primary)**: `SDK total latency − B0 pure-network latency = structural overhead (including FFI)`. aimux and AISDK each subtract B0 and then subtract from each other to get the net difference. Reuses Dimension One's B0 baseline, with no extra work.
2. **Segmented timing (auxiliary)**: Instrument points within the SDK call path, splitting it into four segments—"request construction", "network round-trip", "response parsing", and "streaming-shard parsing"—giving each segment's proportion and locating overhead hotspots. On the aimux side, instrumentation is inside Rust and exposed via napi; on the AISDK side, instrumentation is inside TS. Optional—the difference method already answers the main question; segmented timing is for deeper investigation.

**Payload-size curve**: structural overhead should grow linearly with payload size. Test 4 tiers and compare the two's **slopes**:

| Tier | Request body | Response body | Purpose |
|---|---|---|---|
| Small | 1 conversation turn | Short response 100 tokens | Baseline overhead (proportion of FFI boundary fixed cost) |
| Medium | 10 conversation turns | Medium response 500 tokens | Normal load |
| Large | Long prompt 4K tokens | Long response 2K tokens | Whether Rust serde's advantage at large payloads can offset FFI |
| Tool | 5 tool schemas + tool_call response | Tool-call parsing | Structured (non-text) parsing path |

**Core question**: can Rust serde's advantage at large payloads (if any) offset the napi FFI boundary cost? This is the core evidence for judging whether "rewriting the access layer in Rust" is worthwhile. At small payloads, the FFI fixed overhead proportion is high (aimux may actually be slower); at large payloads, serde's advantage emerges (aimux may overtake)—where the inflection point lies is the key output of this dimension.

### 4.3 Dimension Three: Concurrency Capacity (Scaling)

Don't just compare single-point throughput; look at the **curve and stability**.

| Metric | Method |
|---|---|
| Throughput curve | Concurrency N=1/10/50/100/200, each run for a fixed duration; plot reqs/s curve and look at the inflection point (where throughput no longer grows with concurrency) |
| Memory growth | Record peak RSS at each concurrency tier; see whether it inflates linearly with concurrency (a leak/backlog signal) |
| Stress stability | Whether error rate, timeout rate, and P99 tail latency spike under high concurrency |
| Connection-reuse efficiency | aimux before vs after landing RFC-0009; the improvement connection-pool reuse brings to concurrency (see §7) |

**Prerequisite**: depends on RFC-0009 landing (see §7); otherwise the concurrency conclusion is a "known defect" rather than an "architectural ceiling".

## 5. Metric List

Each dimension uniformly collects:

- **Latency**: mean / P50 / P95 / P99
- **Throughput**: reqs/s (non-streaming), tokens/s (streaming steady state)
- **TTFT** (Time To First Token, streaming only): time from call to receiving the first `TextDelta`
- **Memory**: process RSS peak; the before/after increment (stripping the mock server's own footprint)
- **GC stability**: `node --expose-gc` + periodic `gc()` sampling, observing whether aimux's napi introduces cross-boundary GC jitter
- **Structural overhead** (Dimension Two only): proportion of each segment (request construction / network / response parsing / streaming parsing); pure-CPU time for the four payload tiers + regression slope

Output format: each run produces a JSON file (`{ dimension, sdk, n, p50, p95, p99, mean, rss_peak_kb, ... }`), eventually merged into a comparison table + line chart.

## 6. Fairness Control Variables

| Variable | Control method |
|---|---|
| Network | All go through `127.0.0.1`, no real network |
| LLM generation duration | Mock returns fixed responses / fixed-shard SSE, deterministic replay |
| Process startup | Warmup (warmup ≥ 50 times) before timing starts |
| Timer | Same process, same framework (mitata); do not use each language's own bench |
| Connection reuse | Compare with default config; if RFC-0009 is not landed, the report must note that the aimux side has no connection pool (see §7) |
| Retry | Both sides disable retry (`maxRetries: 0`); otherwise retry counts pollute the latency distribution |
| Concurrency model | Both use the Node event loop + the same concurrency primitives (`p-limit` or a hand-written semaphore) |

## 7. Key Dependency: How aimux's Current Request Layer Affects the Comparison

[RFC-0009](0009-request-resilience.md) has found that aimux currently has **45 `Client::new()` sites with no connection-pool sharing, no TLS session reuse, no timeouts anywhere in the repo, and retry as dead code**. This has a decisive impact on the comparison results; each dimension is affected to a different degree:

- **Dimension One · Speed (non-streaming A)**: minimally affected (a single request has no connection-reuse opportunity). Can be done first.
- **Dimension One · Speed (streaming B)**: moderately affected (connection-setup cost is amortized over a long stream). Can be done, but TTFT will include one TLS handshake.
- **Dimension Two · Structural overhead**: **almost unaffected**—it strips the network and only measures CPU; whether a connection pool exists does not affect serialization time. Can be done first.
- **Dimension Three · Concurrency capacity (C)**: **fatally affected**. aimux's providers each establish their own connections; under concurrency, a fresh handshake may occur each time. AISDK (undici) defaults to connection-pool reuse. Here aimux looks worse, but **this is not Rust being slow—it's not having a connection pool**—a known defect, not an architectural ceiling.

**Two options**:

1. **Land RFC-0009's `shared_client()` + `PoolConfig` before measuring Dimension Three** (recommended). Otherwise the concurrency comparison's conclusion is "aimux has a known bug" rather than "aimux's architectural ceiling".
2. If you are in a hurry to produce data, you can do Dimension One (speed) and Dimension Two (structural overhead) first; mark Dimension Three as "comparing the pre-RFC-0009-landing state" and list it separately.

## 8. Landing Structure

It is recommended to add a benchmark subdirectory under `bindings/node/`, reusing the existing node build chain:

```
bindings/node/
├── bench/
│   ├── README.md            # How to run, how to reproduce
│   ├── mock-server.ts       # Local mock server (fixed JSON + SSE replay)
│   ├── cassettes/           # Replay data reused from aimux-providers/tests/cassettes
│   ├── payloads/            # Dimension 2 payload four tiers (small/medium/large/tools) fixed data
│   ├── bench-nonstream.ts   # Dimension 1 · speed (non-streaming A)
│   ├── bench-stream.ts      # Dimension 1 · speed (streaming B)
│   ├── bench-struct.ts      # Dimension 2 · structured overhead (difference method + payload curve + segmented timing)
│   ├── bench-concurrent.ts  # Dimension 3 · concurrency capacity (throughput curve + memory + stability)
│   └── package.json         # Depends on mitata + @ai-sdk/openai + openai
└── ... (existing structure unchanged)
```

**Mock server selection**: Node's native `http` or `undici`'s `MockAgent`. Prefer `undici` MockAgent—AISDK already uses undici, and aimux's side hits real HTTP via reqwest, so both can align on the same mock backend. If reqwest cannot be intercepted by undici MockAgent (different HTTP stacks), fall back to starting a real local `http.Server` that both sides hit.

**Dimension Two payload generation**: the small/medium/large/tool tiers preferably reuse real cassettes from `aimux-providers/tests/cassettes/` (to keep the parsing path real); any shortfall is generated by the mock server from fixed templates.

## 9. Implementation Order

| Step | Content | Output |
|---|---|---|
| 1 | Set up mock server + B0 baseline (pure undici direct call) | Verify the measurement baseline is usable |
| 2 | Dimension One · Speed: non-streaming A (aimux + AISDK) | First comparison data |
| 3 | Dimension Two · Structural overhead: difference method (A − B0) + four-tier payload curve | Inflection point + slope, answering "is the Rust rewrite worth it" |
| 4 | Dimension One · Speed: streaming B (TTFT + tokens/s) | SSE parsing comparison |
| 5 | Evaluate: whether to land RFC-0009 before Dimension Three | Decision point |
| 6 | Dimension Three · Concurrency capacity: throughput curve + memory + stability | Complete comparison report |
| 7 | Aggregate into a comparison report (tables + charts) and put it in `docs/` | Performance data that can be cited externally |

Each step is independently runnable and does not block subsequent steps. Steps 1-4 do not depend on RFC-0009 and can start immediately.

## 10. Risks

| Risk | Level | Mitigation |
|---|---|---|
| **The mock server itself becomes a bottleneck** | Medium | Run the B0 baseline first, confirming mock throughput is far higher than the SDK under test; use multiple mock instances if necessary |
| **napi FFI serialization eats the Rust advantage** | Medium | The B0 baseline strips FFI cost; Dimension Two's payload curve is exactly what answers this—if aimux's advantage is erased by FFI, that conclusion is itself an important finding |
| **Streaming SSE replay timing distortion** | Medium | Use byte-level replay of real cassettes, preserving original shard boundaries and intervals |
| **AISDK defaults to retry/middleware** | Low | Explicitly `maxRetries:0`, turn off unrelated middleware, align both sides |
| **Concurrency dimension polluted by RFC-0009 defects** | High | As stated in §7, decide before Dimension Three |
| **Node version / undici version differences** | Low | Lock `engines` + lock versions, annotate the environment in the report |

## 11. Things Not to Do

1. **Do not compare pure-Rust direct call vs pure-JS direct call**—meaningless across languages. All comparisons are within the Node process.
2. **Do not hit real LLMs**—network and generation duration are uncontrollable and pollute the measurement. If real end-to-end stress testing is needed, raise a separate proposal.
3. **Do not build a production-grade stress-testing platform**—this benchmark is a reproducible micro-benchmark, not a load generator like wrk/k6.
4. **Do not compare full coverage of 172 providers**—take only one, the OpenAI protocol, as a representative (native protocol); thin wrappers share the same request path, so the conclusion can be extrapolated.
5. **Do not land RFC-0009 in this RFC**—the two are decoupled, but Dimension Three depends on RFC-0009 (§7).

## 12. Acceptance

- [ ] `bindings/node/bench/` can be run with a single `pnpm bench`
- [ ] A single run produces a JSON result file + a terminal comparison table
- [ ] Dimension One (speed): one set of data each for non-streaming A + streaming B
- [ ] Dimension Two (structural overhead): four-tier payload curve + inflection-point/slope conclusion; FFI cost can be stripped
- [ ] Dimension Three (concurrency capacity): throughput curve + memory growth + stability data
- [ ] The report explicitly annotates aimux's request-layer status (pre/post RFC-0009)
- [ ] B0 baseline data is complete
- [ ] The conclusion is reproducible: two consecutive runs on the same machine have P50 variance < 5%
