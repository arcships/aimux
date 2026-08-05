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
use super::verdict::VerdictKind;

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
    #[allow(dead_code)]
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
    /// Any block failing 128-bit verification, or an expired candidate
    /// (now − latest t_send > ttl_ms), stops the walk (monotonicity).
    pub fn lookup(&mut self, scope: u64, chain: &Chain, now_ms: u64, ttl_ms: u64) -> LcpResult {
        let mut blocks: u64 = 0;
        let mut matched: Option<MatchInfo> = None;
        let mut candidate_expired = false;
        for (i, h) in chain.block_hashes.iter().enumerate() {
            let key = (scope, hash::low64(*h));
            let mut best: Option<MatchInfo> = None;
            if let Some(refs) = self.index.get(&key) {
                for r in refs {
                    let slot = &self.ring[r.slot as usize];
                    if slot.generation != r.generation {
                        self.tombstone_hits += 1; // lazy-invalidation tombstone
                        continue;
                    }
                    let Some(rec) = &slot.record else { continue };
                    if rec.scope != scope {
                        continue;
                    }
                    // Position must match (LCP is positional) + full
                    // 128-bit verification rejects 64-bit index collisions.
                    if r.block as usize != i {
                        continue;
                    }
                    if rec.block_hashes.get(r.block as usize) != Some(h) {
                        continue;
                    }
                    if best
                        .as_ref()
                        .map(|b| rec.t_send_ms > b.t_send_ms)
                        .unwrap_or(true)
                    {
                        best = Some(MatchInfo {
                            t_send_ms: rec.t_send_ms,
                            session: rec.session.clone(),
                            len_bytes: rec.len_bytes,
                        });
                    }
                }
            }
            match best {
                Some(m) if now_ms.saturating_sub(m.t_send_ms) <= ttl_ms => {
                    blocks = i as u64 + 1;
                    matched = Some(m);
                }
                Some(_m) => {
                    // Candidate exists but outside its TTL window.
                    if i == 0 {
                        candidate_expired = true;
                    }
                    break;
                }
                _ => break,
            }
        }
        self.maybe_rebuild_index();
        LcpResult {
            lcp_bytes: blocks * chain.block_size as u64,
            matched_blocks: blocks as u32,
            matched,
            candidate_expired,
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
    by_session: HashMap<String, Vec<String>>, // session_id → trace_ids (ordered)
}

// Arc2: std::sync::Arc (name avoids confusion with the variant).
use std::sync::Arc as Arc2;

impl RingTraceStore {
    /// A store with default bounds (2048 records, 512 per scope).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY, DEFAULT_SCOPE_CAP)
    }

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
                // Client upper bound: average of per-record ratios is not
                // meaningful for totals; use ratio of estimated tokens.
                let est: u64 = inner
                    .records
                    .iter()
                    .filter(|r| r.provider == key.0 && r.model == key.1 && matches_filter(r, f))
                    .map(|r| {
                        r.fingerprint
                            .token_estimate
                            .min(r.usage.input_total.unwrap_or(0))
                    })
                    .sum();
                g.client_upper_bound_hit_rate = Some(est as f64 / total as f64);
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
            .filter_map(|id| inner.records.iter().find(|r| &r.trace_id == id))
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
                    at_record_id: b.trace_id.clone(),
                    prev_record_id: a.trace_id.clone(),
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

    /// Export all records as JSONL (one `TraceRecord` per line).
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
            ids.retain(|id| id != &evicted.trace_id);
            if ids.is_empty() {
                inner.by_session.remove(sid);
            }
        }
        if let Some(sid) = &rec.session_id {
            inner
                .by_session
                .entry(sid.clone())
                .or_default()
                .push(rec.trace_id.clone());
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
    use crate::trace::fingerprint::BlockChainFingerprint;

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
}
