//! TraceStore 核心(单线程原型,round-3-design.md §4/§5/§7 的落地):
//!
//! - 固定容量**环形缓冲**(全局 cap)+ per-scope cap(超限惰性淘汰最老同 scope 记录);
//! - **(slot, gen) 懒失效**:被淘汰记录的链不从反向索引即时删,靠世代校验跳过;
//!   墓碑率 >25% 时整索引重建;
//! - **反向索引**(scope, 块哈希低 64) → Vec<BlockRef(slot, gen, block_idx)>,命中后用记录内
//!   全 128-bit 链验证(64-bit 索引碰撞 → 验证失败 → 拒绝,安全方向);
//! - lookup 逐块前移:块失配/过期即停(单调性,round-3-design.md §1);
//!   候选集合取最晚 t_send 做 **TTL idle 判定**(now − last_touch ≤ TTL_idle);
//! - 明文 body 不落盘,只存链 + 计数。

use std::collections::HashMap;

use crate::fingerprint::Chain;

#[derive(Debug, Clone)]
pub struct StoredRecord {
    pub scope: u64,
    pub session: Option<u64>,
    pub len_bytes: u64,
    /// 单调钟毫秒(测试直接注入;生产 = Instant.elapsed().as_millis())。
    pub t_send_ms: u64,
    pub claimed: u64,
    pub block_hashes: Vec<u128>,
}

#[derive(Debug, Clone)]
struct RingSlot {
    gen: u32,
    record: Option<StoredRecord>,
}

#[derive(Debug, Clone, Copy)]
struct BlockRef {
    slot: u32,
    gen: u32,
    block: u32,
}

/// lookup 命中信息(判定引擎的视界输入)。
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchInfo {
    pub t_send_ms: u64,
    pub session: Option<u64>,
    pub len_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LcpResult {
    /// 块粒度 LCP 字节数(下界:真共享 ≥ 该值)。
    pub lcp_bytes: u64,
    pub matched_blocks: u32,
    pub matched: Option<MatchInfo>,
}

pub struct TraceStore {
    cap: usize,
    per_scope_cap: usize,
    ring: Vec<RingSlot>,
    next: usize,
    /// (scope, 块哈希低64) → BlockRef
    index: HashMap<(u64, u64), Vec<BlockRef>>,
    scope_count: HashMap<u64, u64>,
    tombstone_hits: u64,
    pub inserted: u64,
}

impl TraceStore {
    pub fn new(cap: usize, per_scope_cap: usize) -> Self {
        assert!(cap > 0);
        TraceStore {
            cap,
            per_scope_cap,
            ring: (0..cap)
                .map(|_| RingSlot {
                    gen: 0,
                    record: None,
                })
                .collect(),
            next: 0,
            index: HashMap::new(),
            scope_count: HashMap::new(),
            tombstone_hits: 0,
            inserted: 0,
        }
    }

    pub fn records_in_scope(&self, scope: u64) -> u64 {
        self.scope_count.get(&scope).copied().unwrap_or(0)
    }

    /// 插入记录(含反向索引增量维护)。per-scope 超限时惰性清掉最老同 scope 记录。
    pub fn insert(&mut self, rec: StoredRecord) {
        if self.per_scope_cap > 0 {
            let n = self.scope_count.get(&rec.scope).copied().unwrap_or(0);
            if n >= self.per_scope_cap as u64 {
                self.evict_oldest_in_scope(rec.scope);
            }
        }
        let slot = self.next % self.cap;
        self.next += 1;
        let gen = self.ring[slot].gen.wrapping_add(1);
        self.ring[slot] = RingSlot {
            gen,
            record: Some(rec.clone()),
        };
        *self.scope_count.entry(rec.scope).or_insert(0) += 1;
        for (block, h) in rec.block_hashes.iter().enumerate() {
            self.index
                .entry((rec.scope, crate::hash::low64(*h)))
                .or_default()
                .push(BlockRef {
                    slot: slot as u32,
                    gen,
                    block: block as u32,
                });
        }
        self.inserted += 1;
    }

    fn evict_oldest_in_scope(&mut self, scope: u64) {
        let mut oldest: Option<(usize, u64)> = None; // (slot, t_send)
        for (i, s) in self.ring.iter().enumerate() {
            if let Some(r) = &s.record {
                if r.scope == scope {
                    match oldest {
                        Some((_, t)) if t <= r.t_send_ms => {}
                        _ => oldest = Some((i, r.t_send_ms)),
                    }
                }
            }
        }
        if let Some((slot, _)) = oldest {
            self.ring[slot].gen = self.ring[slot].gen.wrapping_add(1);
            self.ring[slot].record = None;
            if let Some(n) = self.scope_count.get_mut(&scope) {
                *n = n.saturating_sub(1);
            }
        }
    }

    /// 逐块查找同 scope 的最长公共前缀(块粒度)。
    /// 任一块:128-bit 验证 + (now − 最晚 t_send ≤ TTL_idle) 不满足 → 立即停止。
    pub fn lookup(&mut self, scope: u64, chain: &Chain, now_ms: u64, ttl_ms: u64) -> LcpResult {
        let mut blocks: u64 = 0;
        let mut matched: Option<MatchInfo> = None;
        'outer: for (i, h) in chain.block_hashes.iter().enumerate() {
            let key = (scope, crate::hash::low64(*h));
            let mut best: Option<MatchInfo> = None;
            if let Some(refs) = self.index.get(&key) {
                for r in refs {
                    let slot = &self.ring[r.slot as usize];
                    if slot.gen != r.gen {
                        self.tombstone_hits += 1; // 懒失效墓碑
                        continue;
                    }
                    let Some(rec) = &slot.record else { continue };
                    if rec.scope != scope {
                        continue;
                    }
                    // 位置 i 的 128-bit 链验证(64-bit 索引碰撞在此被拒绝);
                    // r.block 必须等于查询位置 i(LCP 是逐位比较)
                    if r.block as usize != i {
                        continue;
                    }
                    if rec.block_hashes.get(r.block as usize) != Some(h) {
                        continue;
                    }
                    if best
                        .map(|b| rec.t_send_ms > b.t_send_ms)
                        .unwrap_or(true)
                    {
                        best = Some(MatchInfo {
                            t_send_ms: rec.t_send_ms,
                            session: rec.session,
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
                _ => break 'outer, // 失配或已过期 → 停(单调性)
            }
        }
        self.maybe_rebuild_index();
        LcpResult {
            lcp_bytes: blocks * chain.block_size as u64,
            matched_blocks: blocks as u32,
            matched,
        }
    }

    fn maybe_rebuild_index(&mut self) {
        // 墓碑率 >25% 时整索引重建(均摊 O(总条目) ≈ 4× 插入成本)
        let live = self.index.len();
        if live > 0 && self.tombstone_hits * 4 > live as u64 {
            let mut new_index: HashMap<(u64, u64), Vec<BlockRef>> = HashMap::new();
            for (slot, s) in self.ring.iter().enumerate() {
                if let Some(rec) = &s.record {
                    for (block, h) in rec.block_hashes.iter().enumerate() {
                        new_index
                            .entry((rec.scope, crate::hash::low64(*h)))
                            .or_default()
                            .push(BlockRef {
                                slot: slot as u32,
                                gen: s.gen,
                                block: block as u32,
                            });
                    }
                }
            }
            self.index = new_index;
            self.tombstone_hits = 0;
        }
    }

    /// 仅测试用:断言内部不变量(可选)。
    #[cfg(test)]
    pub fn index_size(&self) -> usize {
        self.index.len()
    }
}
