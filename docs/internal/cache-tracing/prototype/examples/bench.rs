//! 性能粗测:200KB body 链计算、TraceStore lookup、全管线(链+lookup+判定)。
//! 运行:cargo run --release --example bench(debug 同样可跑作对照)。

use std::hint::black_box;
use std::time::Instant;

use cache_audit::fingerprint::BlockChainFingerprint;
use cache_audit::store::{StoredRecord, TraceStore};
use cache_audit::synth::{token_body, Req, Runner};
use cache_audit::verdict::{Kind, ProviderSpec};

fn main() {
    let block_size = 4096usize;
    let body: Vec<u8> = (0..200_000usize)
        .map(|i| ((i.wrapping_mul(31) ^ (i >> 7)) % 251) as u8)
        .collect();
    let fp = BlockChainFingerprint::new(block_size, 0x1234_5678);

    // ── 1. 链计算 ──
    let chain0 = fp.compute(&body);
    let blocks = chain0.block_hashes.len();
    println!(
        "body={}B block={}B blocks={}",
        body.len(),
        block_size,
        blocks
    );
    let n = 2000;
    let t0 = Instant::now();
    let mut acc: u128 = 0;
    for _ in 0..n {
        let c = fp.compute(&body);
        acc ^= c.body_hash;
    }
    let dt = t0.elapsed();
    let mb_per_s = body.len() as f64 * n as f64 / dt.as_secs_f64() / 1e6;
    println!(
        "[chain] {:.1} µs/op | {:.0} MB/s ({} blocks)",
        dt.as_secs_f64() * 1e6 / n as f64,
        mb_per_s,
        blocks
    );
    black_box(acc);

    // ── 2. TraceStore lookup(2000 条互异记录,每条 50 块;查询其中一条 → 满 LCP)──
    // per_scope_cap=0 关闭 per-scope 淘汰,保证 2000 条全在环内。
    let mut st = TraceStore::new(2048, 0);
    let mut chains = Vec::with_capacity(2001);
    for i in 0..2001 {
        let b: Vec<u8> = (0..200_000usize)
            .map(|j| ((j.wrapping_mul(31) ^ (i * 7919) ^ 0x5a5a_5a5a) % 251) as u8)
            .collect();
        chains.push(fp.compute(&b));
    }
    for (i, c) in chains[..2000].iter().enumerate() {
        st.insert(StoredRecord {
            scope: 1,
            session: Some(1),
            len_bytes: c.len_bytes,
            t_send_ms: i as u64,
            claimed: 0,
            block_hashes: c.block_hashes.clone(),
        });
    }
    let q = &chains[1500]; // 已插入的一条 → 满 LCP 50 块
    let n2 = 2000;
    let t1 = Instant::now();
    let mut total_blocks = 0u64;
    for _ in 0..n2 {
        let r = st.lookup(1, q, 100_000, 3_600_000);
        total_blocks += r.matched_blocks as u64;
    }
    let dt2 = t1.elapsed();
    println!(
        "[lookup] {:.1} µs/op (matched {} blocks/op, 128-bit verify + TTL idle)",
        dt2.as_secs_f64() * 1e6 / n2 as f64,
        total_blocks / n2
    );
    black_box(total_blocks);

    // ── 3. 全管线(链 + lookup + 判定),200KB 请求 ──
    let spec = ProviderSpec::openai_56();
    let mut runner = Runner::new(block_size, spec, true);
    // 预热一条历史
    let warm = Req::oai56(1, Some(1), 5_000, 45_000, 0, 0, 0, 0);
    runner.process(&warm);
    let n3 = 1000;
    let mut ok = 0usize;
    let t2 = Instant::now();
    for i in 0..n3 {
        let req = Req::oai56(1, Some(1), 5_000, 45_000, 0, (i + 1) as u64, 0, 0);
        let v = runner.process(&req);
        if v.kind == Kind::Trusted {
            ok += 1;
        }
    }
    let dt3 = t2.elapsed();
    println!(
        "[pipeline] {:.1} µs/op (chain+lookup+judge @200KB, {}/{} Trusted)",
        dt3.as_secs_f64() * 1e6 / n3 as f64,
        ok,
        n3
    );

    // ── 内存量级(round-3-design.md §9 口径)──
    let per_rec = blocks * 16 + 96; // 链 + 元数据
    println!(
        "[mem] ~{:.0} B/record({} 块),~{:.0} KB/1000 请求",
        per_rec,
        blocks,
        per_rec as f64 * 1000.0 / 1024.0
    );

    // 小提示:合成 token body 作为对照的链计算
    let tb = token_body(25_000, 25_000, 0);
    let t3 = Instant::now();
    let mut h = 0u128;
    for _ in 0..1000 {
        h ^= fp.compute(&tb).body_hash;
    }
    let dt4 = t3.elapsed();
    println!(
        "[chain-tokbody] {:.1} µs/op @{}B",
        dt4.as_secs_f64() * 1e6 / 1000.0,
        tb.len()
    );
    black_box(h);
}
