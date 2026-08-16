//! TraceRecord data model (RFC-0015 §5.1): request identity + fingerprint +
//! usage snapshot + verdict. Fully owned, `Clone`, `Send + Sync`. Plaintext
//! bodies never enter a trace — only the hashed fingerprint does.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::fingerprint::Fingerprint;
use super::verdict::Verdict;

/// Token usage snapshot (7 flat fields + raw passthrough).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageSnapshot {
    pub input_total: Option<u64>,
    pub input_no_cache: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output_total: Option<u64>,
    pub output_text: Option<u64>,
    pub output_reasoning: Option<u64>,
    /// Raw provider usage payload (opaque passthrough; numbers only — no
    /// prompt text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Request-side cache hints (best-effort, e.g. Anthropic `cache_control`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RequestCacheHints {
    /// Whether any message requested `cache_control` (write) this call.
    pub requested_write: bool,
}

/// One probed call. Plaintext never persists — fingerprints only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceRecord {
    pub provider: String,
    pub model: String,
    pub request_id: Option<String>,
    /// From `CallOptions.session_id` (RFC-0024) — explicit first, inference
    /// fallback; may also be the layer's default session.
    pub session_id: Option<String>,
    /// Call-level unique id (association key for Recording / replay,
    /// RFC-0023).
    pub call_id: String,
    /// When the call was sent (epoch ms).
    #[ts(type = "number")]
    pub sent_at_unix_ms: i64,
    /// Monotonic clock ms (same domain as the store's TTL lookups; internal
    /// bookkeeping, not part of the wire contract).
    #[serde(skip)]
    #[ts(skip)]
    pub monotonic_sent_ms: u64,
    /// Client-side LCP token upper bound (block upper bound, byte-proxy
    /// len/4). `None` when no history matched. Consumed by `TraceStats`
    /// aggregation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcp_token_upper: Option<u64>,
    /// Time to first streamed token (ms). Non-streaming: `None`.
    #[ts(type = "number | null")]
    pub ttft_ms: Option<u64>,
    pub fingerprint: Fingerprint,
    pub usage: UsageSnapshot,
    /// Response-side cache headers (e.g. `x-openrouter-cache-status`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_cache_headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_cache_hints: Option<RequestCacheHints>,
    /// Audit verdict (`None` when no auditor is attached — the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// Error string when the call failed (still recorded: failures are part
    /// of the session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Internal scope key (index only; never crosses the wire contract).
    #[serde(skip)]
    #[ts(skip)]
    pub scope_key: u64,
}

impl TraceRecord {
    /// Reported hit rate — strictly `cache_read / input_total` (RFC-0015
    /// §5.2; never added to `cache_write`).
    #[must_use]
    pub fn reported_hit_rate(&self) -> Option<f64> {
        match (self.usage.cache_read, self.usage.input_total) {
            (Some(r), Some(t)) if t > 0 => Some(r as f64 / t as f64),
            _ => None,
        }
    }

    /// Client-side upper bound hit rate — token estimate / input_total.
    #[must_use]
    pub fn client_upper_bound_hit_rate(&self) -> Option<f64> {
        let t = self.usage.input_total?;
        if t == 0 {
            return None;
        }
        Some(self.fingerprint.token_estimate.min(t) as f64 / t as f64)
    }
}
