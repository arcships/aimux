//! 判定引擎:按 round-3-verdict-rules.md §0 期望区间公式 + §1 硬不变量表实现。
//!
//! 期望命中区间:`U = min(prompt, quantize_down(token_upper − tail_loss, gran))`,
//! 无 tokenizer 时字节代理 `token_upper = min(LCPb, prompt_bytes) / 4`(方向:tok ≤ bytes);
//! 尾块损耗默认 0(保守上界)。判定:`claimed∈[0,U]→OK`;`∈(U, U+τ]→B`;`>U+τ→W`,
//! `τ = max(5%·U, 1 gran 块)`;绝对上限 `claimed>prompt→W`。
//!
//! 实现规则: R-1.1 / R-1.1abs / R-1.2 / R-1.3 / R-1.6b(量化)/ R-1.7 / R-1.8 /
//! R-2.1(段位)/ R-2.2(M 预警)/ R-2.3(5.6+ implicit 白名单)/ R-3.3(低门槛)/ R-5.1(usage 缺失)。
//! 纯函数、表驱动,输入 TraceRecord 证据 + 历史 LCP,输出 Verdict(映射 W→SuspectOverclaim、
//! OK→Trusted、U→Unknown、M→SuspectUnderclaim,与数据模型契约一致)。

/// provider 族(仅 DeepSeek 等族特有规则需要分派;gran/threshold/ttl 由 ProviderSpec 承载)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    OpenAi,
    DeepSeek,
    Anthropic,
    Gemini,
    Vllm,
    Generic,
}

/// 每 provider 族的审计参数(round-3-verdict-rules.md §7 建议值)。
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub family: Family,
    /// token 量化粒度;None = 不量化(OpenAI 原生 5.6+ / Anthropic 定性)
    pub gran: Option<u64>,
    /// 最低缓存门槛(token):低于它 claimed 必为 0
    pub threshold: u64,
    /// TTL_idle 毫秒(审计保守上限)
    pub ttl_ms: u64,
    /// gpt-5.6+ 语义(无量化、R-2.3 implicit 断点白名单、R-1.7 写读等式)
    pub model56plus: bool,
}

impl ProviderSpec {
    pub fn openai_56() -> Self {
        Self {
            family: Family::OpenAi,
            gran: None,
            threshold: 1024,
            ttl_ms: 3_600_000, // 60 min
            model56plus: true,
        }
    }
    pub fn openai_legacy() -> Self {
        Self {
            family: Family::OpenAi,
            gran: Some(128),
            threshold: 1024,
            ttl_ms: 3_600_000,
            model56plus: false,
        }
    }
    pub fn deepseek() -> Self {
        Self {
            family: Family::DeepSeek,
            gran: Some(64),
            threshold: 64,
            ttl_ms: 48 * 3_600_000, // 48h 保守
            model56plus: false,
        }
    }
    pub fn anthropic() -> Self {
        Self {
            family: Family::Anthropic,
            gran: None,
            threshold: 512,
            ttl_ms: 3_600_000, // 取 1h 上界防误报
            model56plus: false,
        }
    }
    pub fn vllm() -> Self {
        Self {
            family: Family::Vllm,
            gran: Some(16),
            threshold: 16,
            ttl_ms: u64::MAX, // LRU,无墙钟 TTL
            model56plus: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Trusted,
    SuspectOverclaim,
    SuspectUnderclaim,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub kind: Kind,
    pub confidence: Confidence,
    /// 命中的规则 id(R-1.1 / R-1.2 / ... / R-2.2 / R-2.3 / R-3.3 / R-5.1)
    pub violated: Vec<String>,
    /// 期望区间上界 U(token)
    pub expected_max: u64,
    pub claimed: u64,
    pub lcp_bytes: u64,
    pub notes: Vec<String>,
}

impl Verdict {
    pub fn describe(&self) -> String {
        format!(
            "kind={:?} conf={:?} violated={:?} U={} claimed={} lcp={}B notes={:?}",
            self.kind, self.confidence, self.violated, self.expected_max, self.claimed, self.lcp_bytes, self.notes
        )
    }
}

/// TraceStore lookup 结果 → 判定输入(已含 TTL idle 过滤)。
#[derive(Debug, Clone)]
pub struct LcpInput {
    pub lcp_bytes: u64,
    /// 命中的历史记录与当前请求同 session(段位规则 R-2.1 用)
    pub same_session: bool,
    pub matched_exists: bool,
}

/// R-2.2 低命中预警所需的会话统计(由 runner 按连续请求维护)。
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionStats {
    /// 同 session 连续轮数(当前请求计入)
    pub same_session_rounds: u32,
    /// 相邻 LCP ≥ 95% 前一轮 body 长度(前缀稳定)
    pub prefix_stable: bool,
    /// 当前 LCP > 1024 token(字节代理:>4096B)
    pub lcp_gt_1024: bool,
}

pub struct JudgeInput<'a> {
    pub spec: &'a ProviderSpec,
    /// strict(自托管/单客户端)→ W;shared → B/U 降级
    pub strict: bool,
    /// 该 scope 窗口内首请求
    pub first: bool,
    pub prompt_tokens: u64,
    pub prompt_bytes: u64,
    pub claimed: u64,
    pub write: Option<u64>,
    /// OpenAI 5.6+ 差量:no_cache = prompt − claimed − write(R-1.7)
    pub no_cache: Option<u64>,
    /// DeepSeek:prompt_cache_hit_tokens / prompt_cache_miss_tokens(R-1.3)
    pub hit: Option<u64>,
    pub miss: Option<u64>,
    pub usage_present: bool,
    pub lcp: Option<LcpInput>,
    /// system/tools 段 token 数(段位期望 R-2.1;缺省视为无 system 段)
    pub system_tokens: u64,
    pub session_stats: Option<SessionStats>,
}

pub fn quantize_down(x: u64, gran: Option<u64>) -> u64 {
    match gran {
        Some(g) if g > 0 => x / g * g,
        _ => x,
    }
}

/// 上界容差 τ = max(5%·U, 1 gran 块)(§7:取大)。
fn tau(u: u64, gran: Option<u64>) -> u64 {
    let pct = ((u as f64) * 0.05).round() as u64;
    match gran {
        Some(g) => pct.max(g),
        None => pct,
    }
}

fn is_w_level(rule: &str) -> bool {
    matches!(
        rule,
        "R-1.1" | "R-1.1abs" | "R-1.2" | "R-1.3" | "R-1.7" | "R-1.8" | "R-3.3"
    )
}

fn is_b_level(rule: &str) -> bool {
    matches!(rule, "R-1.1b" | "R-1.6b")
}

/// 判定入口(纯函数,表驱动):输入证据 + 历史 LCP → Verdict。
pub fn judge(inp: &JudgeInput) -> Verdict {
    let mut v = Verdict {
        kind: Kind::Trusted,
        confidence: Confidence::High,
        violated: Vec::new(),
        expected_max: 0,
        claimed: inp.claimed,
        lcp_bytes: 0,
        notes: Vec::new(),
    };

    // ① usage 缺失 → U(R-5.1)
    if !inp.usage_present {
        v.kind = Kind::Unknown;
        v.confidence = Confidence::Medium;
        v.violated.push("R-5.1".into());
        v.notes.push("usage missing (stream truncated / stripped)".into());
        return v;
    }
    // ② 响应缓存头(X-OpenRouter-Cache-Status:HIT)→ 不审计,零归零合法(R-4.1):
    //    原型场景无该输入,代码路径留空(注释契约)。

    let prompt = inp.prompt_tokens;
    let claimed = inp.claimed;

    // 绝对上限(§0,任何视界下都不可能)
    if claimed > prompt {
        v.violated.push("R-1.1abs".into());
    }

    // R-1.3 DeepSeek 等式:prompt == hit + miss(±1 token 容差)
    if inp.spec.family == Family::DeepSeek {
        if let (Some(h), Some(m)) = (inp.hit, inp.miss) {
            if (h as i64 + m as i64 - prompt as i64).abs() > 1 {
                v.violated.push("R-1.3".into());
            }
        }
    }

    // R-3.3 低门槛(OpenAI<1024 / DeepSeek 64 / ...):低于门槛 claimed 必 0
    if prompt < inp.spec.threshold && claimed > 0 {
        v.violated.push("R-3.3".into());
    }

    // R-1.6b 量化不变量(OpenAI<5.6: claimed%128==0;DeepSeek: claimed%64==0;5.6+ 不量化)
    if let Some(g) = inp.spec.gran {
        if claimed > 0 && claimed % g != 0 && !inp.spec.model56plus {
            v.violated.push("R-1.6b".into());
        }
    }

    // R-1.7 OpenAI 5.6+ 写读等式:claimed + write + no_cache == prompt
    if inp.spec.model56plus {
        if let (Some(w), Some(nc)) = (inp.write, inp.no_cache) {
            if claimed + w + nc != prompt {
                v.violated.push("R-1.7".into());
            }
        }
    }

    // R-1.2 首请求零命中(strict→W;shared→B 中)
    if inp.first && claimed > 0 {
        v.violated.push("R-1.2".into());
    }

    // 视界依赖:U 与 R-1.1 前缀包含域
    let mut lcp_present = false;
    let u: u64;
    if let Some(l) = &inp.lcp {
        lcp_present = true;
        v.lcp_bytes = l.lcp_bytes;
        // R-2.1 段位:跨 session 只认 system/tools 段;同 session append-only 可认整段
        let sys_bytes = inp.system_tokens.saturating_mul(4);
        let cap_bytes = if l.same_session {
            l.lcp_bytes
        } else {
            l.lcp_bytes.min(sys_bytes)
        };
        if !l.same_session && claimed.saturating_mul(4) > sys_bytes {
            v.notes.push("claimed exceeds shared system segment (R-2.1)".into());
        }
        let token_upper = cap_bytes.min(inp.prompt_bytes) / 4; // 字节代理
        u = quantize_down(token_upper, inp.spec.gran).min(prompt);
    } else {
        u = 0;
    }
    if lcp_present {
        v.expected_max = u;
        let t = tau(u, inp.spec.gran);
        if claimed > u + t {
            v.violated.push("R-1.1".into()); // strict→W;shared→B(降级见分类)
        } else if claimed > u {
            v.violated.push("R-1.1b".into()); // 区间 (U, U+τ] → B(中)
        }
    } else if claimed > 0 && !inp.first {
        // 无活视界来源(无历史/超 TTL)→ R-1.8 时序违规(strict→W;shared→U 他进程可能)
        v.violated.push("R-1.8".into());
    }

    // R-2.3 5.6+ implicit 断点白名单:大 LCP + claimed=0 合法,抑制 R-2.2
    let mut suppressed_m = false;
    if inp.spec.model56plus && claimed == 0 && v.lcp_bytes / 4 > 1024 {
        v.notes
            .push("5.6+ implicit breakpoint: claimed=0 legal despite large LCP (R-2.3)".into());
        suppressed_m = true;
    }

    // R-2.2 低命中预警 M(同 session ≥3 轮 + 前缀稳定 + LCP>1024 + claimed=0)
    if !suppressed_m {
        if let Some(ss) = &inp.session_stats {
            if ss.same_session_rounds >= 3 && ss.prefix_stable && ss.lcp_gt_1024 && claimed == 0 {
                v.violated.push("R-2.2".into());
            }
        }
    }

    // 分类(strict 降级:shared 模式下 R-1.1/R-1.2/R-1.8 为 B 或 U)
    const DOWNGRADE_IDS: [&str; 3] = ["R-1.1", "R-1.2", "R-1.8"];
    let mut has_w = v.violated.iter().any(|r| is_w_level(r));
    let mut has_b = v.violated.iter().any(|r| is_b_level(r));
    if !inp.strict {
        let only_downgrade_w = has_w
            && v
                .violated
                .iter()
                .all(|r| !is_w_level(r) || DOWNGRADE_IDS.contains(&r.as_str()));
        if only_downgrade_w {
            has_w = false;
            if lcp_present {
                has_b = true; // 本地来源且超限 → B(中)
            } else {
                v.kind = Kind::Unknown;
                v.confidence = Confidence::Medium;
                v.notes
                    .push("no local history source; other process may have warmed (R-5.3)".into());
                return v;
            }
        }
    }

    if v.violated.iter().any(|r| r == "R-2.2") && !has_w {
        v.kind = Kind::SuspectUnderclaim;
        v.confidence = Confidence::Low;
    } else if has_w {
        v.kind = Kind::SuspectOverclaim;
        v.confidence = Confidence::High;
    } else if has_b {
        v.kind = Kind::SuspectOverclaim;
        v.confidence = Confidence::Medium;
    } else {
        v.kind = Kind::Trusted;
        v.confidence = Confidence::High;
    }
    v
}
