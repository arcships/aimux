//! 合成场景构造 + 顺序 runner(查 → 判 → 插,两阶段语义的简化:占位记录不参与 lookup,
//! 等价于"当前请求先查再插入,天然排除自身")。
//!
//! 字节 ↔ token 口径:合成 body 每个 token 恰好 4 字节(4 hex 字符),使无 tokenizer 的
//! 字节代理 `min(LCPb, prompt_bytes)/4` 与真实 token 数完全一致,测试断言可精确化。

use std::collections::HashMap;

use crate::fingerprint::{BlockChainFingerprint, Chain};
use crate::store::{LcpResult, StoredRecord, TraceStore};
use crate::verdict::{judge, JudgeInput, Kind, LcpInput, ProviderSpec, SessionStats, Verdict};

/// 合成请求(输入证据)。
#[derive(Debug, Clone)]
pub struct Req {
    pub scope: u64,
    pub session: Option<u64>,
    pub system_tokens: u64,
    pub conv_tokens: u64,
    /// 会话内容起点(不同会话用不同起点 → conversation 字节分叉,共享 system)
    pub conv_start: u64,
    pub t_send_ms: u64,
    pub claimed: u64,
    pub write: u64,
    /// DeepSeek:prompt_cache_hit_tokens(== claimed)/ miss_tokens
    pub hit: Option<u64>,
    pub miss: Option<u64>,
    /// OpenAI 5.6+ 差量;None 时由构造器按 prompt−claimed−write 补全
    pub no_cache: Option<u64>,
    pub usage_present: bool,
}

impl Req {
    /// OpenAI 5.6+ 系:no_cache 自动补全,满足 R-1.7 等式
    pub fn oai56(
        scope: u64,
        session: Option<u64>,
        system: u64,
        conv: u64,
        conv_start: u64,
        t: u64,
        claimed: u64,
        write: u64,
    ) -> Self {
        let prompt = system + conv;
        Self {
            scope,
            session,
            system_tokens: system,
            conv_tokens: conv,
            conv_start,
            t_send_ms: t,
            claimed,
            write,
            hit: None,
            miss: None,
            no_cache: Some(prompt.saturating_sub(claimed + write)),
            usage_present: true,
        }
    }

    /// DeepSeek:hit = claimed,miss 显式给出(等式 hit+miss==prompt 由调用方控制)
    pub fn deepseek(
        scope: u64,
        session: Option<u64>,
        system: u64,
        conv: u64,
        conv_start: u64,
        t: u64,
        claimed: u64,
        miss: u64,
    ) -> Self {
        Self {
            scope,
            session,
            system_tokens: system,
            conv_tokens: conv,
            conv_start,
            t_send_ms: t,
            claimed,
            write: 0,
            hit: Some(claimed),
            miss: Some(miss),
            no_cache: None,
            usage_present: true,
        }
    }

    pub fn prompt_tokens(&self) -> u64 {
        self.system_tokens + self.conv_tokens
    }
}

/// 每个 token 恰好 4 字节的确定性 body:system 段(公共)+ conversation 段(按会话起点分叉)。
/// 追加式会话:同 conv_start、conv_tokens 递增 → 字节级前缀追加。
pub fn token_body(system_tokens: u64, conv_tokens: u64, conv_start: u64) -> Vec<u8> {
    let mut s = String::with_capacity(((system_tokens + conv_tokens) * 4) as usize);
    for i in 0..system_tokens {
        s.push_str(&format!("{:04x}", i & 0xffff));
    }
    for i in 0..conv_tokens {
        s.push_str(&format!("{:04x}", (conv_start + i) & 0xffff));
    }
    s.into_bytes()
}

#[derive(Debug, Clone, Default)]
struct ScopeState {
    prev_session: Option<u64>,
    prev_len: u64,
    rounds: u32,
}

pub struct Runner {
    pub fp: BlockChainFingerprint,
    pub store: TraceStore,
    pub spec: ProviderSpec,
    pub strict: bool,
    state: HashMap<u64, ScopeState>,
}

impl Runner {
    pub fn new(block_size: usize, spec: ProviderSpec, strict: bool) -> Self {
        Self {
            fp: BlockChainFingerprint::new(block_size, 0x5eed_5eed),
            store: TraceStore::new(2048, 512),
            spec,
            strict,
            state: HashMap::new(),
        }
    }

    /// 单请求:查 → 判 → 插(两阶段语义)。
    pub fn process(&mut self, req: &Req) -> Verdict {
        let body = token_body(req.system_tokens, req.conv_tokens, req.conv_start);
        let chain: Chain = self.fp.compute(&body);
        let now_ms = req.t_send_ms;
        let lcp: LcpResult = self.store.lookup(req.scope, &chain, now_ms, self.spec.ttl_ms);
        let first = self.store.records_in_scope(req.scope) == 0;

        let st = self.state.entry(req.scope).or_default();
        let rounds = if st.prev_session == req.session {
            st.rounds + 1
        } else {
            1
        };
        let stable = st.prev_len > 0 && lcp.lcp_bytes * 100 >= st.prev_len * 95;
        let session_stats = SessionStats {
            same_session_rounds: rounds,
            prefix_stable: stable,
            lcp_gt_1024: lcp.lcp_bytes / 4 > 1024,
        };

        let same_session = lcp
            .matched
            .map(|m| m.session == req.session)
            .unwrap_or(false);
        let inp = JudgeInput {
            spec: &self.spec,
            strict: self.strict,
            first,
            prompt_tokens: req.prompt_tokens(),
            prompt_bytes: body.len() as u64,
            claimed: req.claimed,
            write: Some(req.write),
            no_cache: req.no_cache,
            hit: req.hit,
            miss: req.miss,
            usage_present: req.usage_present,
            lcp: lcp.matched.map(|_| LcpInput {
                lcp_bytes: lcp.lcp_bytes,
                same_session,
                matched_exists: true,
            }),
            system_tokens: req.system_tokens,
            session_stats: Some(session_stats),
        };
        let v = judge(&inp);

        // 回填 + 状态推进(注意:判完再插,排除自身)
        self.store.insert(StoredRecord {
            scope: req.scope,
            session: req.session,
            len_bytes: body.len() as u64,
            t_send_ms: now_ms,
            claimed: req.claimed,
            block_hashes: chain.block_hashes.clone(),
        });
        let st = self.state.entry(req.scope).or_default();
        st.prev_session = req.session;
        st.prev_len = body.len() as u64;
        st.rounds = rounds;
        v
    }
}

/// 断言辅助:失败信息带完整 verdict(可定位到规则)。
pub fn assert_verdict(v: &Verdict, want: Kind, label: &str) {
    assert_eq!(
        v.kind,
        want,
        "[{}] verdict mismatch → {}",
        label,
        v.describe()
    );
}

pub fn assert_violated(v: &Verdict, want: &str, label: &str) {
    assert!(
        v.violated.iter().any(|r| r == want),
        "[{}] expected violated contains {:?} → {}",
        label,
        want,
        v.describe()
    );
}

pub fn assert_not_violated(v: &Verdict, want: &str, label: &str) {
    assert!(
        !v.violated.iter().any(|r| r == want),
        "[{}] expected NOT violated {:?} → {}",
        label,
        want,
        v.describe()
    );
}
