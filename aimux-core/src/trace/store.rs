//! Trace storage (RFC-0015 §5.3/§6): `TraceSink` trait + bounded ring
//! `RingTraceStore` with per-session index and query API.
//!
//! Zero implicit global state — everything is explicit injection. Memory is
//! hard-bounded (FIFO ring + per-scope cap), TTL expiry is lazy, and
//! (slot, gen) generational invalidation keeps eviction cheap. Plaintext
//! bodies never enter the store — fingerprints only.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::fingerprint::Chain;
use super::hash;
use super::record::TraceRecord;
use super::verdict::{Verdict, VerdictKind};

/// Default global ring capacity (≈6-7 MB with default 4 KiB blocks).
pub const DEFAULT_RING_CAPACITY: usize = 2048;
/// Default per-scope cap.
pub const DEFAULT_SCOPE_CAP: usize = 512;

/// Collection entry point (RFC-0015 §5.3). Thread-safe; the layer calls
/// `record` once per call. Optional `flush` for persistent sinks.
pub trait TraceSink: Send + Sync + 'static {
    fn record(&self, rec: TraceRecord);
    fn flush(&self) {}
    /// Downcast support (the layer uses this to reach the LCP index of a
    /// `RingTraceStore` sink).
    fn as_any(&self) -> &dyn std::any::Any;
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal ring (prototype `TraceStore` port)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct StoredRecord {
    scope: u64,
    session: Option<String>,
    len_bytes: u64,
    t_send_ms: u64,
    block_hashes: Vec<u128>,
}

#[derive(Debug, Clone, Default)]
struct RingSlot {
    generation: u32,
    record: Option<StoredRecord>,
}

#[derive(Debug, Clone, Copy)]
struct BlockRef {
    slot: u32,
    generation: u32,
    block: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MatchInfo {
    pub t_send_ms: u64,
    pub session: Option<String>,
    #[allow(dead_code)]
    pub len_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LcpResult {
    pub lcp_bytes: u64,
    pub matched_blocks: u32,
    pub matched: Option<MatchInfo>,
    /// Block-0 had a candidate that failed the TTL check (timing violation)
    /// — versus no candidate at all (granularity floor).
    pub candidate_expired: bool,
}

/// Internal index: bounded ring + reverse (scope, block-hash-low64) index
/// with (slot, gen) lazy invalidation.
pub(crate) struct TraceStore {
    cap: usize,
    per_scope_cap: usize,
    ring: Vec<RingSlot>,
    next: usize,
    index: HashMap<(u64, u64), Vec<BlockRef>>,
    scope_count: HashMap<u64, u64>,
    tombstone_hits: u64,
}

impl TraceStore {
    pub fn new(cap: usize, per_scope_cap: usize) -> Self {
        assert!(cap > 0);
        TraceStore {
            cap,
            per_scope_cap,
            ring: (0..cap).map(|_| RingSlot::default()).collect(),
            next: 0,
            index: HashMap::new(),
            scope_count: HashMap::new(),
            tombstone_hits: 0,
        }
    }

    pub fn records_in_scope(&self, scope: u64) -> u64 {
        self.scope_count.get(&scope).copied().unwrap_or(0)
    }

    /// Insert a record (maintaining the reverse index). Per-scope overflow
    /// lazily evicts the oldest record of the same scope.
    pub fn insert(&mut self, rec: StoredRecord) {
        if self.per_scope_cap > 0 {
            let n = self.scope_count.get(&rec.scope).copied().unwrap_or(0);
            if n >= self.per_scope_cap as u64 {
                self.evict_oldest_in_scope(rec.scope);
            }
        }
        let slot = self.next % self.cap;
        self.next += 1;
        // The slot may hold an older record (global ring wrap): drop its
        // scope count so `records_in_scope` stays an accurate live count.
        if let Some(old) = &self.ring[slot].record
            && let Some(n) = self.scope_count.get_mut(&old.scope)
        {
            *n = n.saturating_sub(1);
        }
        let generation = self.ring[slot].generation.wrapping_add(1);
        self.ring[slot] = RingSlot {
            generation,
            record: Some(rec.clone()),
        };
        *self.scope_count.entry(rec.scope).or_insert(0) += 1;
        for (block, h) in rec.block_hashes.iter().enumerate() {
            self.index
                .entry((rec.scope, hash::low64(*h)))
                .or_default()
                .push(BlockRef {
                    slot: slot as u32,
                    generation,
                    block: block as u32,
                });
        }
    }

    fn evict_oldest_in_scope(&mut self, scope: u64) {
        let mut oldest: Option<(usize, u64)> = None;
        for (i, s) in self.ring.iter().enumerate() {
            if let Some(r) = &s.record
                && r.scope == scope
                && oldest.map(|(_, t)| r.t_send_ms < t).unwrap_or(true)
            {
                oldest = Some((i, r.t_send_ms));
            }
        }
        if let Some((slot, _)) = oldest {
            self.ring[slot].generation = self.ring[slot].generation.wrapping_add(1);
            self.ring[slot].record = None;
            if let Some(n) = self.scope_count.get_mut(&scope) {
                *n = n.saturating_sub(1);
            }
        }
    }

    /// Longest common prefix against same-scope history (block granularity).
    ///
    /// C4-1: the matched prefix must come from a SINGLE record. The previous
    /// implementation picked the best candidate independently per block, which
    /// could stitch blocks from different records into a "historical prefix"
    /// that no single request ever sent. Now block-0 candidates are collected
    /// and each candidate's full common prefix with the request chain is
    /// computed; the longest live (TTL-valid) prefix wins. Any block failing
    /// 128-bit verification, or an expired candidate (now − t_send > ttl_ms),
    /// is excluded (monotonicity).
    pub fn lookup(&mut self, scope: u64, chain: &Chain, now_ms: u64, ttl_ms: u64) -> LcpResult {
        let mut best: Option<(u64, MatchInfo)> = None; // (shared_blocks, info)
        let mut candidate_expired = false;

        // The LCP is a prefix: only a record whose block 0 matches can
        // contribute. An empty request chain matches nothing.
        let Some(first) = chain.block_hashes.first() else {
            self.maybe_rebuild_index();
            return LcpResult {
                lcp_bytes: 0,
                matched_blocks: 0,
                matched: None,
                candidate_expired: false,
            };
        };
        let key = (scope, hash::low64(*first));
        if let Some(refs) = self.index.get(&key) {
            for r in refs {
                // Position must be block 0 (LCP is positional) + full 128-bit
                // verification rejects 64-bit index collisions.
                if r.block != 0 {
                    continue;
                }
                let slot = &self.ring[r.slot as usize];
                if slot.generation != r.generation {
                    self.tombstone_hits += 1; // lazy-invalidation tombstone
                    continue;
                }
                let Some(rec) = &slot.record else { continue };
                if rec.scope != scope {
                    continue;
                }
                if rec.block_hashes.first() != Some(first) {
                    continue;
                }
                // TTL window (monotonicity): expired candidates do not extend
                // the prefix. A block-0 candidate that is expired marks the
                // timing-violation flag (versus no candidate at all).
                if now_ms.saturating_sub(rec.t_send_ms) > ttl_ms {
                    candidate_expired = true;
                    continue;
                }
                // Whole-prefix match against THIS record only (C4-1): no
                // stitching across records.
                let shared = common_prefix_blocks(&rec.block_hashes, &chain.block_hashes) as u64;
                let info = MatchInfo {
                    t_send_ms: rec.t_send_ms,
                    session: rec.session.clone(),
                    len_bytes: rec.len_bytes,
                };
                // Longest real prefix wins; tie-break by latest t_send_ms
                // (mirrors the prior per-block "best" preference).
                let better = best.as_ref().is_none_or(|(b, m)| {
                    shared > *b || (shared == *b && rec.t_send_ms > m.t_send_ms)
                });
                if better {
                    best = Some((shared, info));
                }
            }
        }
        self.maybe_rebuild_index();
        match best {
            Some((blocks, matched)) => LcpResult {
                lcp_bytes: blocks * chain.block_size as u64,
                matched_blocks: blocks as u32,
                matched: Some(matched),
                candidate_expired: false,
            },
            None => LcpResult {
                lcp_bytes: 0,
                matched_blocks: 0,
                matched: None,
                candidate_expired,
            },
        }
    }

    fn maybe_rebuild_index(&mut self) {
        // Tombstone ratio > 25% of total refs → full index rebuild.
        let live: usize = self.index.values().map(Vec::len).sum();
        if live > 0 && self.tombstone_hits * 4 > live as u64 {
            let mut new_index: HashMap<(u64, u64), Vec<BlockRef>> = HashMap::new();
            for (slot, s) in self.ring.iter().enumerate() {
                if let Some(rec) = &s.record {
                    for (block, h) in rec.block_hashes.iter().enumerate() {
                        new_index
                            .entry((rec.scope, hash::low64(*h)))
                            .or_default()
                            .push(BlockRef {
                                slot: slot as u32,
                                generation: s.generation,
                                block: block as u32,
                            });
                    }
                }
            }
            self.index = new_index;
            self.tombstone_hits = 0;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RingTraceStore — public bounded sink with query API
// ─────────────────────────────────────────────────────────────────────────────

/// Query filter (RFC-0015 §5.3).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceFilter {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub since_unix_ms: Option<i64>,
}

/// Aggregated statistics (RFC-0015 §5.2 — the two hit rates are reported
/// side by side, never merged into a single number).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TraceStats {
    pub provider: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens_total: u64,
    pub claimed_cache_read_total: u64,
    pub claimed_cache_write_total: u64,
    pub reported_hit_rate: Option<f64>,
    pub client_upper_bound_hit_rate: Option<f64>,
    pub verdict_counts: BTreeMap<String, u64>,
    pub ttft_p50_ms: Option<u64>,
    pub ttft_p95_ms: Option<u64>,
    pub errors: u64,
}

/// Session chain view (append-only order + prefix stability, RFC-0015 §5.3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionChainView {
    pub session_id: String,
    pub record_ids: Vec<String>,
    pub prefix_stability: f64,
    pub breaks: Vec<PrefixBreak>,
}

/// One step of a session's cache-hit trajectory (RFC-0024 §4.3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionStepStat {
    /// Step within the session (0-based, session order).
    pub step: u32,
    /// The call's id (association key into Recording / replay, RFC-0023).
    pub call_id: String,
    /// Reported hit rate for this step (`cache_read / input_total`); `None`
    /// when the call carried no input/cache-read usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_rate: Option<f64>,
    /// Audit verdict (present only when an auditor is attached).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// Error string for failed calls (failures are part of the session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A prefix break between two consecutive records of a session.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PrefixBreak {
    pub at_record_id: String,
    pub prev_record_id: String,
    pub lcp_bytes: u64,
    pub expected_break: bool,
    pub kind: BreakKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum BreakKind {
    SystemChanged,
    ToolsChanged,
    ConversationReset,
    Unknown,
}

/// The built-in bounded sink: FIFO ring (global) + per-scope cap + per-session
/// index. Thread-safe; queries are consistent snapshots.
pub struct RingTraceStore {
    inner: Mutex<Inner>,
}

struct Inner {
    store: TraceStore,
    records: VecDeque<Arc2<TraceRecord>>,
    by_session: HashMap<String, Vec<String>>, // session_id → call_ids (ordered)
}

// Arc2: std::sync::Arc (name avoids confusion with the variant).
use std::sync::Arc as Arc2;

impl RingTraceStore {
    /// A store with default bounds (2048 records, 512 per scope).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY, DEFAULT_SCOPE_CAP)
    }

    #[must_use]
    pub fn with_capacity(cap: usize, per_scope_cap: usize) -> Self {
        assert!(cap > 0 && per_scope_cap > 0);
        Self {
            inner: Mutex::new(Inner {
                store: TraceStore::new(cap, per_scope_cap),
                records: VecDeque::with_capacity(cap),
                by_session: HashMap::new(),
            }),
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.store = TraceStore::new(inner.store.cap, inner.store.per_scope_cap);
        inner.records.clear();
        inner.by_session.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Aggregate statistics, grouped by (provider, model), filtered.
    pub fn aggregate(&self, f: &TraceFilter) -> Vec<TraceStats> {
        let inner = self.inner.lock().unwrap();
        let mut groups: BTreeMap<(String, String), TraceStats> = BTreeMap::new();
        let mut ttfts: BTreeMap<(String, String), Vec<u64>> = BTreeMap::new();
        for rec in &inner.records {
            if !matches_filter(rec, f) {
                continue;
            }
            let key = (rec.provider.clone(), rec.model.clone());
            let g = groups.entry(key.clone()).or_insert_with(|| TraceStats {
                provider: rec.provider.clone(),
                model: rec.model.clone(),
                ..Default::default()
            });
            g.requests += 1;
            g.input_tokens_total += rec.usage.input_total.unwrap_or(0);
            g.claimed_cache_read_total += rec.usage.cache_read.unwrap_or(0);
            g.claimed_cache_write_total += rec.usage.cache_write.unwrap_or(0);
            if let Some(kind) = rec.verdict.as_ref().map(|v| kind_name(v.kind)) {
                *g.verdict_counts.entry(kind.to_string()).or_insert(0) += 1;
            }
            if rec.error.is_some() {
                g.errors += 1;
            }
            if let Some(t) = rec.ttft_ms {
                ttfts.entry(key).or_default().push(t);
            }
        }
        // Hit rates + TTFT percentiles per group.
        for (key, g) in groups.iter_mut() {
            let total = g.input_tokens_total;
            if total > 0 {
                g.reported_hit_rate = Some(g.claimed_cache_read_total as f64 / total as f64);
                // Client upper bound: per-record LCP token upper bound only —
                // records without LCP evidence (first request / no match)
                // contribute 0 (RFC-0015 §5.2; never the full request length).
                let est: u64 = inner
                    .records
                    .iter()
                    .filter(|r| r.provider == key.0 && r.model == key.1 && matches_filter(r, f))
                    .map(|r| r.lcp_token_upper.unwrap_or(0))
                    .sum();
                if est > 0 {
                    g.client_upper_bound_hit_rate = Some(est as f64 / total as f64);
                }
            }
            if let Some(mut ts) = ttfts.remove(key) {
                ts.sort_unstable();
                g.ttft_p50_ms = percentile(&ts, 0.50);
                g.ttft_p95_ms = percentile(&ts, 0.95);
            }
        }
        groups.into_values().collect()
    }

    /// Session chain view: ordered record ids + prefix stability + breaks.
    pub fn session_chain(&self, session_id: &str) -> Option<SessionChainView> {
        let inner = self.inner.lock().unwrap();
        let ids = inner.by_session.get(session_id)?;
        if ids.is_empty() {
            return None;
        }
        let records: Vec<&TraceRecord> = ids
            .iter()
            .filter_map(|id| inner.records.iter().find(|r| &r.call_id == id))
            .map(|r| &**r)
            .collect();
        let mut lcp_sum = 0u64;
        let mut prev_len_sum = 0u64;
        let mut breaks = Vec::new();
        for w in records.windows(2) {
            let (a, b) = (w[0], w[1]);
            // Byte LCP between consecutive bodies — from stored fingerprints
            // (hex chains) via prefix-block comparison.
            let (lcp, shared) = chain_lcp(a, b);
            prev_len_sum += a.fingerprint.len_bytes;
            lcp_sum += lcp;
            if shared < a.fingerprint.block_hashes.len() as u64 {
                breaks.push(PrefixBreak {
                    at_record_id: b.call_id.clone(),
                    prev_record_id: a.call_id.clone(),
                    lcp_bytes: lcp,
                    expected_break: false,
                    kind: BreakKind::Unknown,
                });
            }
        }
        let prefix_stability = if prev_len_sum > 0 {
            lcp_sum as f64 / prev_len_sum as f64
        } else {
            1.0
        };
        Some(SessionChainView {
            session_id: session_id.to_string(),
            record_ids: ids.clone(),
            prefix_stability: prefix_stability.min(1.0),
            breaks,
        })
    }

    /// Per-step cache-hit trajectory of a session (RFC-0024 §4.3
    /// `session_cache_trajectory`): one [`SessionStepStat`] per call, in
    /// session order, with the reported hit rate and audit verdict (when an
    /// auditor is attached). Empty for unknown sessions.
    pub fn session_cache_trajectory(&self, session_id: &str) -> Vec<SessionStepStat> {
        let inner = self.inner.lock().unwrap();
        let ids = match inner.by_session.get(session_id) {
            Some(ids) => ids,
            None => return Vec::new(),
        };
        ids.iter()
            .enumerate()
            .filter_map(|(step, id)| {
                let rec = inner.records.iter().find(|r| &r.call_id == id)?;
                Some(SessionStepStat {
                    step: step as u32,
                    call_id: rec.call_id.clone(),
                    hit_rate: rec.reported_hit_rate(),
                    verdict: rec.verdict.clone(),
                    error: rec.error.clone(),
                })
            })
            .collect()
    }

    /// Export all records as JSONL (one `TraceRecord` per line).
    ///
    /// # Errors
    ///
    /// Returns an I/O error (JSON serialization failures included) when
    /// serializing a record or writing a line fails.
    pub fn export_jsonl(&self, w: &mut impl Write) -> std::io::Result<()> {
        let inner = self.inner.lock().unwrap();
        for rec in &inner.records {
            let line = serde_json::to_string(&**rec).map_err(std::io::Error::other)?;
            writeln!(w, "{line}")?;
        }
        Ok(())
    }

    /// Insert a record: ring append + reverse index + per-session index.
    fn append(&self, rec: TraceRecord) {
        let mut inner = self.inner.lock().unwrap();
        // Feed the internal LCP index.
        inner.store.insert(StoredRecord {
            scope: rec.scope_key,
            session: rec.session_id.clone(),
            len_bytes: rec.fingerprint.len_bytes,
            t_send_ms: rec.monotonic_sent_ms,
            block_hashes: rec
                .fingerprint
                .block_hashes
                .iter()
                .map(|h| u128::from_str_radix(h, 16).unwrap_or(0))
                .collect(),
        });
        // Bounded FIFO ring.
        if inner.records.len() >= inner.store.cap
            && let Some(evicted) = inner.records.pop_front()
            && let Some(sid) = &evicted.session_id
            && let Some(ids) = inner.by_session.get_mut(sid)
        {
            ids.retain(|id| id != &evicted.call_id);
            if ids.is_empty() {
                inner.by_session.remove(sid);
            }
        }
        if let Some(sid) = &rec.session_id {
            inner
                .by_session
                .entry(sid.clone())
                .or_default()
                .push(rec.call_id.clone());
        }
        inner.records.push_back(Arc2::new(rec));
    }

    /// Internal LCP lookup (used by `TraceLayer`).
    pub(crate) fn lookup(&self, scope: u64, chain: &Chain, now_ms: u64, ttl_ms: u64) -> LcpResult {
        self.inner
            .lock()
            .unwrap()
            .store
            .lookup(scope, chain, now_ms, ttl_ms)
    }

    pub(crate) fn records_in_scope(&self, scope: u64) -> u64 {
        self.inner.lock().unwrap().store.records_in_scope(scope)
    }
}

impl Default for RingTraceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceSink for RingTraceStore {
    fn record(&self, rec: TraceRecord) {
        self.append(rec);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn matches_filter(rec: &TraceRecord, f: &TraceFilter) -> bool {
    if let Some(p) = &f.provider
        && &rec.provider != p
    {
        return false;
    }
    if let Some(m) = &f.model
        && &rec.model != m
    {
        return false;
    }
    if let Some(s) = &f.session_id
        && rec.session_id.as_deref() != Some(s.as_str())
    {
        return false;
    }
    if let Some(since) = f.since_unix_ms
        && rec.sent_at_unix_ms < since
    {
        return false;
    }
    true
}

fn kind_name(kind: VerdictKind) -> &'static str {
    match kind {
        VerdictKind::Trusted => "Trusted",
        VerdictKind::SuspectOverclaim => "SuspectOverclaim",
        VerdictKind::SuspectUnderclaim => "SuspectUnderclaim",
        VerdictKind::Unknown => "Unknown",
    }
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    Some(sorted[idx.min(sorted.len() - 1)])
}

/// Number of leading blocks two chains share (positional, 128-bit exact).
/// Used by the LCP lookup to confine a prefix match to a single record (C4-1).
fn common_prefix_blocks(a: &[u128], b: &[u128]) -> usize {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i;
        }
    }
    n
}

/// Block-level LCP between two records' stored fingerprint chains.
fn chain_lcp(a: &TraceRecord, b: &TraceRecord) -> (u64, u64) {
    let bs = a.fingerprint.block_size.max(1);
    let n = a
        .fingerprint
        .block_hashes
        .len()
        .min(b.fingerprint.block_hashes.len());
    let mut shared = 0u64;
    for i in 0..n {
        if a.fingerprint.block_hashes[i] == b.fingerprint.block_hashes[i] {
            shared += 1;
        } else {
            break;
        }
    }
    (shared * bs, shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::fingerprint::{BlockChainFingerprint, Fingerprint};
    use crate::trace::record::{TraceRecord, UsageSnapshot};

    fn chain_for(data: &[u8], salt: u64) -> Chain {
        BlockChainFingerprint::new(512, salt).compute(data)
    }

    fn insert(st: &mut TraceStore, scope: u64, t: u64, chain: &Chain) {
        st.insert(StoredRecord {
            scope,
            session: Some("s1".into()),
            len_bytes: chain.len_bytes,
            t_send_ms: t,
            block_hashes: chain.block_hashes.clone(),
        });
    }

    #[test]
    fn lookup_matches_live_records_within_ttl() {
        let mut st = TraceStore::new(16, 512);
        let body = vec![b'a'; 1500];
        let chain = chain_for(&body, 1);
        insert(&mut st, 1, 0, &chain);
        let r = st.lookup(1, &chain, 100, u64::MAX);
        assert!(r.matched_blocks > 0);
        assert!(r.matched.is_some());
    }

    #[test]
    fn lookup_expires_by_ttl_idle() {
        let mut st = TraceStore::new(16, 512);
        let body = vec![b'b'; 1500];
        let chain = chain_for(&body, 2);
        insert(&mut st, 1, 0, &chain);
        let ttl = 3_600_000u64;
        let ok = st.lookup(1, &chain, ttl - 1, ttl);
        assert!(ok.matched.is_some(), "within TTL must match");
        let expired = st.lookup(1, &chain, ttl + 1, ttl);
        assert_eq!(expired.matched_blocks, 0, "over TTL must abstain");
        assert!(expired.candidate_expired, "expired candidate flagged");
    }

    #[test]
    fn lookup_granularity_miss_is_not_timing_violation() {
        let mut st = TraceStore::new(16, 512);
        let short = vec![b'c'; 100];
        let long = vec![b'c'; 200];
        let short_chain = chain_for(&short, 3);
        let long_chain = chain_for(&long, 3);
        insert(&mut st, 1, 0, &short_chain);
        let r = st.lookup(1, &long_chain, 1, u64::MAX);
        assert_eq!(r.matched_blocks, 0);
        assert!(!r.candidate_expired, "no candidate → granularity floor");
    }

    #[test]
    fn lru_ring_evicts_oldest_globally() {
        let mut st = TraceStore::new(2, 512);
        let a = chain_for(b"aaaa", 4);
        let b = chain_for(b"bbbb", 4);
        let c = chain_for(b"cccc", 4);
        insert(&mut st, 1, 0, &a);
        insert(&mut st, 1, 1, &b);
        insert(&mut st, 1, 2, &c);
        let ra = st.lookup(1, &a, 3, u64::MAX);
        assert_eq!(ra.matched_blocks, 0, "a evicted");
        let rc = st.lookup(1, &c, 3, u64::MAX);
        assert!(rc.matched.is_some(), "c live");
    }

    // ── C4-1: same-record prefix constraint ────────────────────────────────

    /// Helper that inserts a record with crafted block hashes (bypasses the
    /// chained `compute` so a cross-record stitch can be constructed).
    fn insert_hashes(st: &mut TraceStore, scope: u64, t: u64, session: &str, hashes: &[u128]) {
        st.insert(StoredRecord {
            scope,
            session: Some(session.into()),
            len_bytes: (hashes.len() * 512) as u64,
            t_send_ms: t,
            block_hashes: hashes.to_vec(),
        });
    }

    fn chain_with(hashes: &[u128]) -> Chain {
        Chain {
            body_hash: 0,
            len_bytes: (hashes.len() * 512) as u64,
            block_size: 512,
            block_hashes: hashes.to_vec(),
        }
    }

    /// Records `[A,X]` and `[Y,B]` exist; a request `[A,B]` must NOT report
    /// LCP=2 (block 0 from record 1, block 1 from record 2 — a fabricated
    /// prefix no single request ever sent). The match must stay within one
    /// record, so LCP=1 (record 1's real prefix).
    #[test]
    fn lookup_does_not_stitch_blocks_across_records() {
        let mut st = TraceStore::new(16, 512);
        // Distinct low-64 bits so no reverse-index bucket collides.
        let a: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0001;
        let b: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0002;
        let x: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0003;
        let y: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0004;
        insert_hashes(&mut st, 1, 0, "s1", &[a, x]);
        insert_hashes(&mut st, 1, 1, "s2", &[y, b]);

        let req = chain_with(&[a, b]);
        let r = st.lookup(1, &req, 100, u64::MAX);
        assert_eq!(
            r.matched_blocks, 1,
            "C4-1: prefix must come from one record (expected 1, got {})",
            r.matched_blocks
        );
        assert!(r.matched.is_some(), "block-0 match still reported");
        // The winning record is record 1 ([A,X]) — its session.
        assert_eq!(r.matched.as_ref().unwrap().session.as_deref(), Some("s1"));
    }

    /// When two records share the real prefix [A,B] but diverge later, the
    /// longest real prefix wins (not the latest block-0 candidate's tail).
    #[test]
    fn lookup_picks_longest_single_record_prefix() {
        let mut st = TraceStore::new(16, 512);
        let a: u128 = 1;
        let b: u128 = 2;
        let c: u128 = 3;
        let x: u128 = 4;
        // R1 = [A,B,C] (t=0, older, longer real prefix), R2 = [A,X] (t=9, newer).
        insert_hashes(&mut st, 1, 0, "long", &[a, b, c]);
        insert_hashes(&mut st, 1, 9, "short", &[a, x]);

        let req = chain_with(&[a, b, c]);
        let r = st.lookup(1, &req, 100, u64::MAX);
        // R1 shares all 3 blocks; R2 shares only 1. Longest real prefix = 3.
        assert_eq!(r.matched_blocks, 3, "longest single-record prefix wins");
        assert_eq!(
            r.matched.as_ref().unwrap().session.as_deref(),
            Some("long"),
            "must pick the record that owns the full prefix"
        );
    }

    /// A block-0 candidate that is expired sets the timing-violation flag even
    /// when no live candidate extends the prefix.
    #[test]
    fn lookup_expired_block0_candidate_flags_timing() {
        let mut st = TraceStore::new(16, 512);
        let a: u128 = 1;
        let x: u128 = 2;
        insert_hashes(&mut st, 1, 0, "s1", &[a, x]);

        let req = chain_with(&[a, x]);
        let ttl = 1_000u64;
        let live = st.lookup(1, &req, ttl - 1, ttl);
        assert!(live.matched.is_some(), "within TTL matches");
        assert!(!live.candidate_expired);

        let expired = st.lookup(1, &req, ttl + 1, ttl);
        assert_eq!(expired.matched_blocks, 0, "over TTL abstains");
        assert!(
            expired.candidate_expired,
            "expired block-0 candidate must flag timing violation"
        );
    }

    // ── RFC-0024 P4: session_cache_trajectory ──────────────────────────────

    fn trajectory_record(
        call_id: &str,
        session_id: &str,
        input_total: u64,
        cache_read: u64,
        error: Option<String>,
    ) -> TraceRecord {
        TraceRecord {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            request_id: None,
            session_id: Some(session_id.into()),
            call_id: call_id.into(),
            sent_at_unix_ms: 1,
            monotonic_sent_ms: 0,
            lcp_token_upper: None,
            ttft_ms: None,
            fingerprint: Fingerprint {
                body_hash: "0".repeat(32),
                len_bytes: 0,
                block_size: 0,
                block_hashes: vec![],
                token_estimate: 0,
            },
            usage: UsageSnapshot {
                input_total: Some(input_total),
                cache_read: Some(cache_read),
                ..Default::default()
            },
            response_cache_headers: None,
            request_cache_hints: None,
            verdict: None,
            error,
            scope_key: 0,
        }
    }

    #[test]
    fn session_trajectory_returns_per_step_hit_rates_in_order() {
        let store = RingTraceStore::with_capacity(16, 256);
        // 命中率:0.8 → 0 → 0.5(同一 session);另一 session 不受影响。
        store.record(trajectory_record("c1", "s1", 10, 8, None));
        store.record(trajectory_record("c2", "s1", 5, 0, None));
        store.record(trajectory_record("c3", "s1", 4, 2, Some("boom".into())));
        store.record(trajectory_record("c-x", "s2", 9, 9, None));

        let traj = store.session_cache_trajectory("s1");
        assert_eq!(traj.len(), 3);
        assert_eq!(traj[0].step, 0);
        assert_eq!(traj[0].call_id, "c1");
        assert_eq!(traj[0].hit_rate, Some(0.8));
        assert_eq!(traj[0].error, None);
        assert_eq!(traj[1].step, 1);
        assert_eq!(traj[1].hit_rate, Some(0.0));
        assert_eq!(traj[2].step, 2);
        assert_eq!(traj[2].hit_rate, Some(0.5));
        assert_eq!(traj[2].error.as_deref(), Some("boom"));

        // 未知 session 与其它 session 互不干扰。
        assert!(store.session_cache_trajectory("nope").is_empty());
        assert_eq!(store.session_cache_trajectory("s2").len(), 1);
        assert_eq!(store.session_cache_trajectory("s2")[0].call_id, "c-x");
    }
}
