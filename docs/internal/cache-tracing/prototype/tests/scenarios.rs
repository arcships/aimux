//! 8 个合成场景 + 基础单元测试。
//! 测试用小块(512B = 128 token/块)使 LCP 精确;bench 用默认 4096。

use cache_audit::fingerprint::BlockChainFingerprint;
use cache_audit::store::TraceStore;
use cache_audit::synth::{assert_not_violated, assert_verdict, assert_violated, token_body, Req, Runner};
use cache_audit::verdict::{Kind, ProviderSpec};

const BS: usize = 512; // 测试块大小:512B = 128 token

// ───────────────────────── 基础单元 ─────────────────────────

#[test]
fn fingerprint_determinism_and_sensitivity() {
    let fp = BlockChainFingerprint::new(BS, 7);
    let a = token_body(100, 200, 0);
    let b = token_body(100, 200, 0); // 相同
    let c = token_body(100, 200, 1); // 尾部差一个 token
    let ca = fp.compute(&a);
    let cb = fp.compute(&b);
    let cc = fp.compute(&c);
    assert_eq!(ca.body_hash, cb.body_hash, "相同 body 链应相同");
    assert_ne!(ca.body_hash, cc.body_hash, "差 1 token 应不同");
    assert_eq!(ca.block_hashes.len(), (a.len() + BS - 1) / BS);
    // 同 scope 不同 salt 链不同
    let fp2 = BlockChainFingerprint::new(BS, 8);
    assert_ne!(fp2.compute(&a).body_hash, ca.body_hash, "跨 scope 盐不可比");
}

#[test]
fn store_ring_eviction_and_scope_isolation() {
    let fp = BlockChainFingerprint::new(BS, 0);
    let mut st = TraceStore::new(4, 512); // 全局环只有 4 槽
    let body = token_body(50, 50, 0);
    let chain = fp.compute(&body);
    st.insert(cache_audit::store::StoredRecord {
        scope: 1,
        session: Some(10),
        len_bytes: body.len() as u64,
        t_send_ms: 0,
        claimed: 0,
        block_hashes: chain.block_hashes.clone(),
    });
    st.insert(cache_audit::store::StoredRecord {
        scope: 1,
        session: Some(10),
        len_bytes: body.len() as u64,
        t_send_ms: 10,
        claimed: 0,
        block_hashes: chain.block_hashes.clone(),
    });
    // scope 隔离:另一个 scope 查不到
    let r = st.lookup(2, &chain, 20, u64::MAX);
    assert_eq!(r.matched_blocks, 0, "跨 scope 不可见");
    // 同 scope 命中
    let r = st.lookup(1, &chain, 20, u64::MAX);
    assert_eq!(r.matched_blocks, chain.block_count() as u32, "应全链命中");
    // 填满环 → 记录 0 被淘汰(懒失效)
    for t in 30..=80u64 {
        st.insert(cache_audit::store::StoredRecord {
            scope: 1,
            session: Some(10),
            len_bytes: body.len() as u64,
            t_send_ms: t,
            claimed: 0,
            block_hashes: chain.block_hashes.clone(),
        });
    }
    let r = st.lookup(1, &chain, 100, u64::MAX);
    assert!(r.matched_blocks > 0, "环内仍有活记录");
    // TTL:全过期的旧记录 → abstain
    let mut st2 = TraceStore::new(16, 512);
    st2.insert(cache_audit::store::StoredRecord {
        scope: 1,
        session: Some(10),
        len_bytes: body.len() as u64,
        t_send_ms: 0,
        claimed: 0,
        block_hashes: chain.block_hashes.clone(),
    });
    let r = st2.lookup(1, &chain, 3_600_001, 3_600_000);
    assert_eq!(r.matched_blocks, 0, "超 TTL idle 必须 abstain");
}

// ───────────────────────── 场景 1:append-only agent loop → OK ─────────────────────────

#[test]
fn scenario_1_append_only_loop_honest() {
    let spec = ProviderSpec::openai_56();
    let mut r = Runner::new(BS, spec, true);
    // system 1600 token,conversation 1600 起,每轮 +128 token(1 tool 消息),前缀稳定
    let system = 1600u64;
    let rounds = [(1600u64, 0u64), (1728, 3200), (1856, 3328), (1984, 3456)];
    for (i, (conv, claimed)) in rounds.iter().enumerate() {
        let req = Req::oai56(1, Some(7), system, *conv, 0, i as u64 * 1000, *claimed, 128);
        let v = r.process(&req);
        assert_verdict(&v, Kind::Trusted, &format!("S1 round {}", i + 1));
        assert!(v.expected_max >= *claimed, "U 必须 ≥ claimed:{}", v.describe());
    }
}

// ───────────────────────── 场景 2:虚报 +10% → W ─────────────────────────

#[test]
fn scenario_2_overclaim_by_10pct() {
    let spec = ProviderSpec::openai_56();
    let mut r = Runner::new(BS, spec, true);
    let system = 1600u64;
    // 前缀 = 3200 token(U=3200,τ=5%·U=160 → U+τ=3360)
    let v1 = r.process(&Req::oai56(1, Some(7), system, 1600, 0, 0, 0, 0));
    assert_verdict(&v1, Kind::Trusted, "S2 warmup");
    // 虚报 claimed = 前缀 × 1.1 = 3520 > 3360 → W(SuspectOverclaim);
    // prompt=4000 ≥ claimed,字段自洽,只有 R-1.1 触发(纯虚报场景)
    let v2 = r.process(&Req::oai56(1, Some(7), system, 2400, 0, 1000, 3520, 128));
    assert_verdict(&v2, Kind::SuspectOverclaim, "S2 overclaim");
    assert_violated(&v2, "R-1.1", "S2");
    assert_not_violated(&v2, "R-1.1abs", "S2");
    assert_eq!(v2.expected_max, 3200, "U 应为前缀 token:S2 {}", v2.describe());
    assert_eq!(v2.claimed, 3520, "claimed 应为 3520");
}

// ───────────────────────── 场景 3:首请求 claimed>0 → W ─────────────────────────

#[test]
fn scenario_3_first_request_claim() {
    let spec = ProviderSpec::openai_legacy();
    let mut r = Runner::new(BS, spec, true);
    let v = r.process(&Req::oai56(1, Some(9), 1000, 2000, 0, 0, 2048, 0));
    assert_verdict(&v, Kind::SuspectOverclaim, "S3 first-request");
    assert_violated(&v, "R-1.2", "S3");
}

// ───────────────────────── 场景 4:5.6+ implicit 断点(大 LCP,claimed=0)→ 不误报 ─────────────────────────

#[test]
fn scenario_4_gpt56_implicit_breakpoint_no_false_positive() {
    let spec = ProviderSpec::openai_56(); // model56plus = true
    let mut r = Runner::new(BS, spec, true);
    let system = 1000u64;
    let convs = [2000u64, 2100, 2200, 2300]; // 每轮 +1 tool 消息,尾部变化
    for (i, conv) in convs.iter().enumerate() {
        let v = r.process(&Req::oai56(1, Some(5), system, *conv, 0, i as u64 * 500, 0, 128));
        assert_verdict(&v, Kind::Trusted, &format!("S4 round {}", i + 1));
        assert_not_violated(&v, "R-2.2", "S4"); // 白名单抑制低命中预警
        if i >= 2 {
            assert!(
                v.notes.iter().any(|n| n.contains("R-2.3")),
                "应打 implicit breakpoint 白名单注记:{}",
                v.describe()
            );
            assert!(v.lcp_bytes / 4 > 1024, "LCP 应 >1024 token");
        }
    }
}

// ───────────────────────── 场景 5:DeepSeek 等式违反 → W ─────────────────────────

#[test]
fn scenario_5_deepseek_equality() {
    let spec = ProviderSpec::deepseek();
    // 5a:违反 hit+miss==prompt → W(R-1.3)
    let mut ra = Runner::new(BS, spec.clone(), true);
    let v0 = ra.process(&Req::deepseek(1, Some(1), 200, 800, 0, 0, 0, 1000));
    assert_verdict(&v0, Kind::Trusted, "S5a warmup");
    let v = ra.process(&Req::deepseek(1, Some(1), 200, 1200, 0, 1000, 640, 300));
    assert_verdict(&v, Kind::SuspectOverclaim, "S5a eq-violation");
    assert_violated(&v, "R-1.3", "S5a");
    // 5b:等式成立 → 无 R-1.3,claimed 在区间内 → Trusted
    let mut rb = Runner::new(BS, spec, true);
    let v0 = rb.process(&Req::deepseek(1, Some(1), 200, 800, 0, 0, 0, 1000));
    assert_verdict(&v0, Kind::Trusted, "S5b warmup");
    let v = rb.process(&Req::deepseek(1, Some(1), 200, 1200, 0, 1000, 640, 760));
    assert_verdict(&v, Kind::Trusted, "S5b eq-holds");
    assert_not_violated(&v, "R-1.3", "S5b");
}

// ───────────────────────── 场景 6:TTL idle 超限 → W;间隔内 → OK ─────────────────────────

#[test]
fn scenario_6_ttl_idle() {
    let ttl = 3_600_000u64; // 60 min
    let spec = ProviderSpec {
        ttl_ms: ttl,
        ..ProviderSpec::openai_56()
    };
    let mut r = Runner::new(BS, spec, true);
    let system = 256u64;
    // A:t=0 首请求(claimed=0)
    let v0 = r.process(&Req::oai56(1, Some(3), system, 1024, 0, 0, 0, 0));
    assert_verdict(&v0, Kind::Trusted, "S6 A");
    // B1:间隔 1s(<<TTL)→ 命中合法 → OK
    let v1 = r.process(&Req::oai56(1, Some(3), system, 1152, 0, 1000, 1280, 128));
    assert_verdict(&v1, Kind::Trusted, "S6 B1 within-TTL");
    // C:t=TTL+2s(距 B1 超过 TTL_idle)→ 前缀过期 → W(R-1.8)
    let v2 = r.process(&Req::oai56(1, Some(3), system, 1280, 0, ttl + 2_000, 1280, 128));
    assert_verdict(&v2, Kind::SuspectOverclaim, "S6 C over-TTL");
    assert_violated(&v2, "R-1.8", "S6");
    assert_eq!(v2.expected_max, 0, "过期前缀应 abstain(U=0)");
}

// ───────────────────────── 场景 7:低于 1024 门槛 claimed>0 → W ─────────────────────────

#[test]
fn scenario_7_low_threshold() {
    let spec = ProviderSpec::openai_legacy();
    let mut r = Runner::new(BS, spec, true);
    // 预热(大请求,claimed=0)
    let v0 = r.process(&Req::oai56(1, Some(4), 500, 1500, 0, 0, 0, 0));
    assert_verdict(&v0, Kind::Trusted, "S7 warmup");
    // 小请求:300 token < 1024,claimed=128 > 0 → W(R-3.3)
    let v = r.process(&Req::oai56(1, Some(4), 300, 0, 0, 1000, 128, 0));
    assert_verdict(&v, Kind::SuspectOverclaim, "S7 low-threshold");
    assert_violated(&v, "R-3.3", "S7");
}

// ───────────────────────── 场景 8:并行/乱序,跨 session 共享 system 段 → 段位期望 ─────────────────────────

#[test]
fn scenario_8_parallel_cross_session_segment() {
    let spec = ProviderSpec::openai_56();
    let mut r = Runner::new(BS, spec, true);
    let system = 512u64; // system 段 512 token
    // A:会话 1(首次,claimed=0)
    let v0 = r.process(&Req::oai56(1, Some(1), system, 2048, 0x1000, 0, 0, 0));
    assert_verdict(&v0, Kind::Trusted, "S8 A");
    // B:会话 2 并行请求,只共享 system 段;claimed = system 段 → OK
    let v1 = r.process(&Req::oai56(1, Some(2), system, 2048, 0x2000, 10, 512, 0));
    assert_verdict(&v1, Kind::Trusted, "S8 B segment-hit");
    assert_eq!(v1.expected_max, 512, "跨 session 期望上界 = system 段 token");
    // C:会话 3,claimed 越过 system 段(700>512+26)→ W(R-1.1)+ R-2.1 注记
    let v2 = r.process(&Req::oai56(1, Some(3), system, 2048, 0x3000, 20, 700, 0));
    assert_verdict(&v2, Kind::SuspectOverclaim, "S8 C beyond-segment");
    assert_violated(&v2, "R-1.1", "S8");
    assert!(
        v2.notes.iter().any(|n| n.contains("R-2.1")),
        "应有段位超限注记:{}",
        v2.describe()
    );
    // D:同会话 2 append-only(+256 token)→ 同 session 可认整段 → OK
    let v3 = r.process(&Req::oai56(1, Some(2), system, 2304, 0x2000, 30, 768, 128));
    assert_verdict(&v3, Kind::Trusted, "S8 D same-session-append");
    assert!(v3.expected_max >= 768, "同 session 应认整段 U={}", v3.expected_max);
}

// ───────────────────────── 附加:R-2.2 M 预警(对照场景 4 白名单)─────────────────────────

#[test]
fn bonus_m_underclaim_warning() {
    let spec = ProviderSpec::anthropic(); // 非 5.6+,不适用 R-2.3
    let mut r = Runner::new(BS, spec, true);
    let system = 1000u64;
    let convs = [2000u64, 2100, 2200, 2300];
    for (i, conv) in convs.iter().enumerate() {
        let v = r.process(&Req::oai56(1, Some(5), system, *conv, 0, i as u64 * 500, 0, 128));
        if i + 1 <= 2 {
            assert_verdict(&v, Kind::Trusted, &format!("M round {}", i + 1));
        } else {
            // ≥3 轮、前缀稳定、claimed 恒 0、LCP>1024 → SuspectUnderclaim(Low)
            assert_verdict(&v, Kind::SuspectUnderclaim, &format!("M round {}", i + 1));
            assert_violated(&v, "R-2.2", "M");
        }
    }
}

// ───────────────────────── 附加:R-5.1 usage 缺失 → Unknown ─────────────────────────

#[test]
fn bonus_usage_missing_unknown() {
    let spec = ProviderSpec::openai_56();
    let mut r = Runner::new(BS, spec, true);
    let mut req = Req::oai56(1, Some(6), 500, 1500, 0, 0, 0, 0);
    req.usage_present = false; // 流式中断 → usage 缺失
    let v = r.process(&req);
    assert_verdict(&v, Kind::Unknown, "usage-missing");
    assert_violated(&v, "R-5.1", "usage-missing");
}
