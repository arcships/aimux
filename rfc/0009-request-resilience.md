# RFC-0009: Request Optimization — Referencing catcher's Design

> **Status**: DRAFT (pending review)
> **Date**: 2026-07-29
> **Scope**: `aimux-provider-utils` references three specific design points from catcher (connection-pool config, jitter backoff, fixed timeouts) and implements request-layer optimization using reqwest natively + the existing retry.rs, without introducing a catcher-http dependency
> **Related**: [RFC-0002](0002-provider-improvements.md) provider adapter layer improvements, [RFC-0003](0003-test-cassette.md) test cassette plan

## 1. Motivation

aimux has built "unified LLM service access" very comprehensively (172 providers, 6 language bindings), but **the underlying request sending/receiving runs bare**. The evidence is as follows:

| # | Problem | Evidence |
|---|------|------|
| 1 | **45 providers each do `reqwest::Client::new()`**, with no connection-pool sharing and no TLS session reuse | `grep -rn "Client::new()" aimux-providers/src/` → 45 hits. reqwest officially recommends reusing a single Client across the whole application |
| 2 | **Retry logic is dead code** | `aimux-provider-utils/src/retry.rs` defines and re-exports `retry_with_exponential_backoff*`, but there are **zero call sites** across the repo |
| 3 | **79 direct `.send()` sites with no retry wrapping** | `grep -rln ".send().await" aimux-providers/src/` → 79 files. Failure throws immediately; 429/5xx are not retried |
| 4 | **No timeouts anywhere in the repo** | `grep -rn ".timeout(\|connect_timeout\|pool_idle\|tcp_keepalive"` → empty. Pending connections block forever |
| 5 | **Retry has no jitter** | `retry.rs` is pure exponential backoff; concurrent 429s will cause thundering herd |

[catcher](https://github.com/eric8810/catcher) (same author, same Rust+reqwest+rustls stack, same MIT) has a `catcher-http` crate that has matured the request/resilience layer. After verifying module by module, the take/skip conclusions are in §3.

## 2. Approach Selection: Reference, Don't Depend

| | Route A: depend on catcher-http | Route B (this RFC): reference catcher's design, implement with reqwest natively |
|---|---|---|
| Approach | Depend on `catcher-http` + `catcher-core`, wrap error conversion + self-build builder | Copy catcher's specific design points, implement with reqwest natively + the existing retry.rs |
| New dependencies | catcher-http + catcher-core + catcher-dns + backon + reqwest-middleware + retry-policies + parking_lot + tokio-util + rmp-serde | **Zero** |
| reqwest version | Forced upgrade 0.12→0.13 (catcher uses 0.13) | No upgrade needed, stay on 0.12 |
| retry-after header | ❌ catcher's `HttpError{status,body}` has no structured passthrough; backon's fixed backoff does not read the header | ✅ Retain aimux's existing retry.rs header-reading capability (stronger) |
| Error conversion | Need to map 17 CatcherError variants | Not needed; use AiMuxError directly |
| Workload | reqwest upgrade + error conversion + self-built HttpRequestBuilder (catcher's HttpRequest is a plain data struct with no chaining) + migrate 172 providers | Copy 3 design points, ~100 lines |
| Autonomy | Follows catcher releases | Self-controlled |

**Conclusion: what is truly valuable about catcher for aimux is its design patterns (PoolConfig fields, jitter strategy, timeout config), not its code package.** Borrowing patterns is more worthwhile than carrying a dependency, especially since the retry-after point proves that "referencing" is semantically more correct than "introducing" in the LLM scenario.

## 3. Take/Skip: How Each catcher Capability Is Handled for aimux

Judgment after verifying the catcher-http source module by module:

| catcher capability | Handling for aimux | Reason |
|---|---|---|
| `PoolConfig` field design | ✅ **Introduce** (copy fields, implement with reqwest natively) | `max_idle_per_host`/`idle_timeout_secs`/`keep_alive`/`keep_alive_interval_secs`; reqwest ClientBuilder supports all of them; highest payoff |
| `Full Jitter` backoff strategy | ✅ **Introduce** (add to the existing retry.rs) | catcher `backoff.rs`'s `DecorrelatedJitter` is AWS Full Jitter, preventing thundering herd; aimux's existing retry.rs is pure exponential with no jitter |
| Fixed timeout fields | ✅ **Introduce** (copy `connect_timeout_ms`/`response_timeout_ms`) | Natively supported by reqwest ClientBuilder |
| `Adaptive Timeout` (P90 RTT adaptive) | ❌ **Do not introduce** | `timeout = clamp(P90_RTT * multiplier)`; LLM request duration depends on generation length/max_tokens rather than network RTT, and would wrongly kill long-generation requests |
| `CircuitBreaker` state machine | 📌 **Defer** | The implementation is mature (CLOSED→OPEN→HALF_OPEN, ~150 lines), but aimux is a library, not a gateway, with no fallback target; retry first—after retry lands, if "consecutive failures each waiting for a timeout" becomes a pain point, add it then |
| `reqwest-retry` middleware as a whole | ❌ **Do not introduce** | backon's fixed backoff does not read the retry-after header, weaker than aimux's existing retry.rs |
| SSE auto-reconnect | 📌 **Defer** | Streaming + retry semantics are complex (retrying after tokens have already been emitted would duplicate content); separate proposal |
| msgpack / WS / TLS pinning / DNS cache / network-switch hot rebuild | ❌ **Do not introduce** | Not needed in LLM scenarios |

## 4. Design

### 4.1 Introduction Point 1: Shared Client + PoolConfig (reqwest native)

Reference catcher's `PoolConfig` ([catcher-http/src/types/http.rs](https://github.com/eric8810/catcher/blob/master/packages/catcher-http/src/types/http.rs)) and implement it natively with reqwest's `ClientBuilder`:

```rust
//! aimux-provider-utils/src/http.rs

use std::sync::OnceLock;
use std::time::Duration;
use reqwest::Client;

/// Connection pool config (refer to catcher PoolConfig field design).
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_idle_per_host: usize,        // catcher: 10
    pub idle_timeout_secs: u64,           // catcher: 30 — prevents retry reusing a dead connection
    pub keep_alive: bool,                // catcher: true
    pub keep_alive_interval_secs: u64,   // catcher: 20 — detects dead connections faster
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 10,
            idle_timeout_secs: 30,
            keep_alive: true,
            keep_alive_interval_secs: 20,
        }
    }
}

/// Timeout config (refer to catcher HttpClientConfig's two timeout fields).
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    pub connect_timeout_ms: u64,         // catcher: 10_000
    pub response_timeout_ms: u64,        // catcher: 30_000; pass 0 for streaming requests to disable
}

/// Globally shared reqwest::Client. OnceLock guarantees it is built only once,
/// connection pool/TLS session reused repo-wide. Replaces 45 Client::new() sites.
static SHARED: OnceLock<Client> = OnceLock::new();

/// Get (or lazily initialize) the shared reqwest Client.
pub fn shared_client() -> &'static Client {
    SHARED.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::default()))
}

fn build_client(pool: PoolConfig, timeout: TimeoutConfig) -> Client {
    let mut b = Client::builder()
        .connect_timeout(Duration::from_millis(timeout.connect_timeout_ms))
        .pool_max_idle_per_host(pool.max_idle_per_host)
        .pool_idle_timeout(Duration::from_secs(pool.idle_timeout_secs));
    if pool.keep_alive {
        b = b.tcp_keepalive(Duration::from_secs(pool.keep_alive_interval_secs));
    }
    if timeout.response_timeout_ms > 0 {
        b = b.timeout(Duration::from_millis(timeout.response_timeout_ms));
    }
    b.build().expect("shared reqwest Client build failed")
}
```

**Benefit**: TLS session reuse and connection-pool sharing—aimux's current biggest performance gap, solved with zero new dependencies. Eliminates the 45 `Client::new()` sites.

### 4.2 Introduction Point 2: Jitter Backoff (added to the existing retry.rs, retaining retry-after)

catcher's `DecorrelatedJitter` ([backoff.rs](https://github.com/eric8810/catcher/blob/master/packages/catcher-http/src/resilience/backoff.rs)) is AWS Full Jitter: `delay ∈ [0, calculated_backoff]`.

aimux's existing `retry.rs` is pure exponential backoff with no jitter. Add jitter, **and retain aimux's existing retry-after header-reading capability** (this is stronger than catcher's; don't lose it):

```rust
//! aimux-provider-utils/src/retry.rs (incremental patch, no rewrite)

/// Adds Full Jitter on top of get_retry_delay_ms (refer to catcher DecorrelatedJitter).
/// delay = random(0, base); base still prefers the retry-after hint, falling back to exponential backoff.
pub fn get_retry_delay_ms_with_jitter(
    hint: Option<i64>,
    exponential_delay_ms: i64,
    rng: &mut impl rand::Rng,
) -> i64 {
    let base = get_retry_delay_ms(hint, exponential_delay_ms); // reuses existing logic
    if base <= 0 { return 0; }
    rng.gen_range(0..base) // Full Jitter
}
```

- The existing `get_retry_delay_ms` / `parse_retry_after` remain untouched; the new function reuses them
- The existing `retry_with_exponential_backoff_respecting_retry_headers` switches its internal sleep to the jitter version
- Add a `rand` dependency (the workspace already has `futures`/`tokio`; rand is small)

**Benefit**: prevents thundering herd on concurrent 429s, without losing retry-after semantics.

### 4.3 Introduction Point 3: Fixed Timeout

Directly use the `TimeoutConfig` from §4.1. Key decisions:

- **Non-streaming requests**: `response_timeout_ms = 30_000` (catcher's default), guarded by reqwest `.timeout()`
- **Streaming requests**: `response_timeout_ms = 0` disables the overall timeout—LLM streaming duration depends on generation length, and a fixed timeout would wrongly kill long generations. Only keep `connect_timeout_ms = 10_000` to guard the connection-establishment phase

The provider chooses based on whether the request is streaming when constructing it. `shared_client()` uses the default 30s overall timeout; streaming providers need a separate client without the overall timeout when calling (or override it with reqwest's per-request `.timeout(None)`).

## 5. Things Not to Do (Rationale for Items Not Introduced)

1. **Do not introduce the catcher-http dependency**. The 3 points where catcher is truly valuable to aimux (§4) can be reproduced with zero dependencies using reqwest natively + the existing retry.rs, while the cost of introducing the dependency (forced reqwest upgrade 0.12→0.13 + 17 CatcherError variant conversions + self-built HttpRequestBuilder + dependency-tree bloat + retry-after semantic regression) far exceeds the benefit.
2. **Do not introduce AdaptiveTimeout**. `timeout = P90_RTT * multiplier` does not hold for LLMs: two requests both have a 200ms RTT, but one generates 10 tokens (500ms total) and the other generates 2000 tokens (30s total); computing the timeout from RTT would wrongly kill the latter.
3. **Do not introduce CircuitBreaker**. aimux is a library, not a gateway; a single provider failure is just a failure, with no fallback target. Retry first; if consecutive failures each waiting for a timeout becomes a pain point, add it then (at that point, copying catcher's state machine is ~150 lines).
4. **Do not introduce the reqwest-retry middleware**. backon's fixed backoff does not read the retry-after header, weaker than aimux's existing retry.rs.
5. **Do not introduce SSE auto-reconnect**. Streaming + retry semantics are complex; separate proposal.
6. **Do not introduce msgpack / WS / TLS pinning / DNS cache / network-switch hot rebuild**. Not needed in LLM scenarios.

## 6. Migration Strategy

### 6.1 Test Safety Net

aimux tests use a **wiremock local mock server** (see `aimux-providers/tests/openai_image_test.rs`), pointing `base_url` at `localhost` and not depending on a specific Client instance. Refactoring how the client is constructed does not break these tests—they only depend on base_url routing. This is the biggest feasibility guarantee of this approach.

Note: retry will amplify the request count in mock tests. wiremock's `.expect(N)` assertions need to be adjusted accordingly (mocks without `expect` set allow multiple hits by default, so most tests are unaffected).

### 6.2 Phased Landing

| Batch | Scope | Notes |
|---|---|---|
| 1 | `aimux-provider-utils` adds `src/http.rs` (shared_client + PoolConfig + TimeoutConfig) + retry.rs adds jitter | Additive; does not touch providers |
| 2 | Wire the existing retry.rs into the request path (currently dead code, 0 calls) | Add a `send_with_retry` wrapper function |
| 3 | Migrate the 11 native-protocol providers (openai/anthropic/google/...) from `Client::new()` → `shared_client()` | Priority; covers the main traffic |
| 4 | 145 OpenAI-compatible thin wrappers | Share the same request path; can be batch-replaced by script |
| 5 | Speech/image/video-specific providers | Mostly non-streaming; simplest |

After each batch, run `cargo test -p aimux-providers --tests` as a guard. **No reqwest version upgrade is needed** (a major advantage of Route B over Route A).

## 7. Risks

| Risk | Level | Mitigation |
|---|---|---|
| **172 providers, large surface area** | Medium | Phased migration (§6.2), regress each batch; thin wrappers share the path and can be batch-replaced by script |
| **Streaming + retry semantics** | Medium | The first version only covers connection-establishment-phase retry; no retry after tokens have been emitted; the overall timeout is disabled for streaming |
| **wiremock `.expect(N)` conflicts with retry count** | Low | Most mocks do not set expect; a few need adjustment to N × retry count |
| **shared_client default timeout wrongly kills streaming** | Medium | Disable the overall timeout for streaming requests (§4.3), or override per-request |
| **jitter introduces a rand dependency** | Low | rand is small, part of the standard-library ecosystem |

## 8. Implementation Order

1. **Wrapper layer**: `aimux-provider-utils` creates `src/http.rs` (shared_client + PoolConfig + TimeoutConfig).
2. **retry hookup**: the existing `retry.rs` adds jitter; add `send_with_retry` to wire retry into the request path (currently 0 calls).
3. **Pilot**: migrate `aimux-providers/src/openai/` (native protocol, main traffic), run all openai tests to verify.
4. **Roll out**: migrate the remaining providers per the §6.2 batches.
5. **Wrap up**: update the README architecture diagram and the `aimux-provider-utils` module docs.

Each step can be merged independently and does not block subsequent steps. No prerequisite dependency upgrades.
