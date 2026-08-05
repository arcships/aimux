//! Verdict engine (RFC-0015 §4): expected-hit interval + 8 hard invariants +
//! strict/shared dual mode. Pure, table-driven — the prototype's `judge`
//! ported into the core, with provider parameters in a matrix (§7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Verdict kinds (RFC-0015 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VerdictKind {
    Trusted,
    SuspectOverclaim,
    SuspectUnderclaim,
    Unknown,
}

/// Confidence level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum VerdictConfidence {
    High,
    Medium,
    Low,
}

/// A verdict for one call.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub confidence: VerdictConfidence,
    /// Violated rule ids (R-1.1 / R-1.2 / …).
    pub violated: Vec<String>,
    /// Expected upper bound U (tokens).
    pub expected_max: u64,
    pub claimed: u64,
    /// Client-side LCP (bytes).
    pub lcp_bytes: u64,
    pub notes: Vec<String>,
}

impl Verdict {
    pub fn describe(&self) -> String {
        format!(
            "kind={:?} conf={:?} violated={:?} U={} claimed={} lcp={}B notes={:?}",
            self.kind,
            self.confidence,
            self.violated,
            self.expected_max,
            self.claimed,
            self.lcp_bytes,
            self.notes
        )
    }
}

/// Provider audit parameters (RFC-0015 §7 matrix — the part that lives in
/// core; display logic belongs to the CLI).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderAuditSpec {
    /// Provider family for family-specific invariants.
    pub family: ProviderFamily,
    /// Token quantization granularity; `None` = no quantization.
    pub gran: Option<u64>,
    /// Minimum cache threshold (tokens): below it `claimed` must be 0.
    pub threshold: u64,
    /// TTL_idle (ms), conservative upper bound.
    pub ttl_ms: u64,
    /// gpt-5.6+ semantics (no quantization, implicit-breakpoint whitelist,
    /// write/read equality).
    pub model56plus: bool,
    /// Claimed-hit ceiling: above this multiple of the client upper bound
    /// the verdict is overclaim (W) even in shared mode. `None` = use U+τ.
    pub shared_ceiling_mult: Option<f64>,
}

/// Provider family (only families with distinct invariants need dispatch;
/// gran/threshold/ttl live in `ProviderAuditSpec`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ProviderFamily {
    OpenAi,
    DeepSeek,
    Anthropic,
    Bedrock,
    Gemini,
    Vllm,
    Generic,
}

/// The provider audit matrix (RFC-0015 §7, A/B/C tiers → parameters).
pub mod matrix {
    use super::{ProviderAuditSpec, ProviderFamily};

    const HOUR: u64 = 3_600_000;

    /// OpenAI Chat/Responses, gpt-5.6+ (byte-exact, no quantization).
    pub fn openai_56() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::OpenAi,
            gran: None,
            threshold: 1024,
            ttl_ms: HOUR,
            model56plus: true,
            shared_ceiling_mult: None,
        }
    }

    /// OpenAI legacy (< 5.6) / compatible: 128-token quantization.
    pub fn openai_legacy() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::OpenAi,
            gran: Some(128),
            threshold: 1024,
            ttl_ms: HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Azure: 128-token granularity across the board (differs from OpenAI
    /// 5.6+ byte-exact — matrix-verified).
    pub fn azure() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::OpenAi,
            gran: Some(128),
            threshold: 1024,
            ttl_ms: HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// DeepSeek: 64-token granularity + equality invariant (hit+miss==prompt).
    pub fn deepseek() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::DeepSeek,
            gran: Some(64),
            threshold: 64,
            ttl_ms: 48 * HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Mistral: 64-token granularity.
    pub fn mistral() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Generic,
            gran: Some(64),
            threshold: 64,
            ttl_ms: HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Anthropic: qualitative (breakpoint + 20-block lookback), no strict
    /// quantization; conservative 1h TTL upper bound.
    pub fn anthropic() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Anthropic,
            gran: None,
            threshold: 512,
            ttl_ms: HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Bedrock Converse: usage-layer equality (total == input+read+write);
    /// quota burndown needs raw usage and is best-effort.
    pub fn bedrock() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Bedrock,
            gran: None,
            threshold: 0,
            ttl_ms: 24 * HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Gemini/Vertex: 24h TTL, no quantization (24h conservative).
    pub fn gemini() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Gemini,
            gran: None,
            threshold: 0,
            ttl_ms: 24 * HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// vLLM: 16-token granularity, no wall-clock TTL (LRU).
    pub fn vllm() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Vllm,
            gran: Some(16),
            threshold: 16,
            ttl_ms: u64::MAX,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Default for providers without a matrix entry (e.g. OpenRouter passthrough
    /// behaves like the upstream; start conservative).
    pub fn generic() -> ProviderAuditSpec {
        ProviderAuditSpec {
            family: ProviderFamily::Generic,
            gran: Some(128),
            threshold: 1024,
            ttl_ms: HOUR,
            model56plus: false,
            shared_ceiling_mult: None,
        }
    }

    /// Resolve by provider name (best-effort; `None` → generic).
    pub fn for_provider(provider: &str, model: &str) -> ProviderAuditSpec {
        let p = provider.to_ascii_lowercase();
        let m = model.to_ascii_lowercase();
        if p.contains("deepseek") {
            return deepseek();
        }
        if p.contains("anthropic") {
            return anthropic();
        }
        if p.contains("azure") {
            return azure();
        }
        if p.contains("gemini") || p.contains("vertex") {
            return gemini();
        }
        if p.contains("bedrock") {
            return bedrock();
        }
        if p.contains("vllm") {
            return vllm();
        }
        if p.contains("mistral") {
            return mistral();
        }
        if p.contains("openai") || p.contains("openrouter") {
            return if m.contains("gpt-5") || m.contains("o3") || m.contains("o4") {
                openai_56()
            } else {
                openai_legacy()
            };
        }
        generic()
    }
}

/// LCP result feeding the judgment (already TTL-filtered).
#[derive(Debug, Clone, Copy)]
pub struct LcpInput {
    /// Block-granularity LCP lower bound (matched_blocks × block_size).
    pub lcp_bytes: u64,
    /// Block upper bound: `(matched_blocks + 1) × block_size` — the audit
    /// upper bound per RFC F3 (`(j+1)·B`), capped at prompt_bytes by the
    /// caller when computing U.
    pub lcp_upper_bytes: u64,
    /// The matched history record belongs to the same session.
    pub same_session: bool,
    /// Whether any live history source matched.
    pub matched_exists: bool,
}

/// Session-level stats for the low-hit warning (R-2.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionStats {
    pub same_session_rounds: u32,
    pub prefix_stable: bool,
    pub lcp_gt_1024: bool,
}

/// All evidence needed to judge one call.
#[derive(Debug, Clone)]
pub struct JudgmentInput {
    pub spec: ProviderAuditSpec,
    /// strict (self-hosted single instance) → W; shared → B/U downgrade.
    pub strict: bool,
    /// First request in this scope window.
    pub first: bool,
    pub prompt_tokens: u64,
    pub prompt_bytes: u64,
    pub claimed: u64,
    /// Input-side breakdown (unified usage) for provider-equality rules.
    pub input_no_cache: Option<u64>,
    pub input_cache_read: Option<u64>,
    pub input_cache_write: Option<u64>,
    pub write: Option<u64>,
    /// OpenAI 5.6+ delta: no_cache = prompt − claimed − write.
    pub no_cache: Option<u64>,
    /// DeepSeek: hit / miss tokens.
    pub hit: Option<u64>,
    pub miss: Option<u64>,
    pub usage_present: bool,
    /// Response-side cache header present (e.g. OpenRouter HIT) — reported
    /// zeros are legal, skip audit (R-4.1).
    pub response_cache_header_hit: bool,
    /// A same-scope candidate existed at block 0 but failed the TTL check
    /// (true timing violation) — vs. no candidate at all (block-granularity
    /// floor, conservative UNKNOWN).
    pub candidate_expired: bool,
    /// Token upper comes from the byte proxy (len/4) — no tokenizer. Per
    /// RFC F2, byte-proxy evidence caps R-1.1 at B(Medium), never W(High).
    pub byte_proxy: bool,
    pub lcp: Option<LcpInput>,
    /// System/tools segment tokens (cross-session expectation, R-2.1).
    pub system_tokens: u64,
    pub session_stats: Option<SessionStats>,
}

pub fn quantize_down(x: u64, gran: Option<u64>) -> u64 {
    match gran {
        Some(g) if g > 0 => x / g * g,
        _ => x,
    }
}

/// Upper tolerance τ = max(5%·U, 1 gran block).
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
        "R-1.1" | "R-1.1abs" | "R-1.2" | "R-1.3" | "R-1.4" | "R-1.5" | "R-1.7" | "R-1.8" | "R-3.3"
    )
}

fn is_b_level(rule: &str) -> bool {
    matches!(rule, "R-1.1b" | "R-1.6b" | "R-1.7b")
}

/// The built-in rule auditor (RFC-0015 §4.2: 8 hard invariants + diagnosis).
///
/// Pure function over evidence; no state, no IO.
pub fn judge(inp: &JudgmentInput) -> Verdict {
    let mut v = Verdict {
        kind: VerdictKind::Trusted,
        confidence: VerdictConfidence::High,
        violated: Vec::new(),
        expected_max: 0,
        claimed: inp.claimed,
        lcp_bytes: 0,
        notes: Vec::new(),
    };

    // R-5.1: usage missing → Unknown.
    if !inp.usage_present {
        v.kind = VerdictKind::Unknown;
        v.confidence = VerdictConfidence::Medium;
        v.violated.push("R-5.1".into());
        v.notes
            .push("usage missing (stream truncated / stripped)".into());
        return v;
    }

    // R-4.1: response-side cache hit (gateway) — reported zeros are legal.
    if inp.response_cache_header_hit {
        v.kind = VerdictKind::Unknown;
        v.confidence = VerdictConfidence::Medium;
        v.violated.push("R-4.1".into());
        v.notes
            .push("response-cache header hit; provider cache not audited (R-4.1)".into());
        return v;
    }

    let prompt = inp.prompt_tokens;
    let claimed = inp.claimed;

    // Absolute ceiling — impossible under any view.
    if claimed > prompt {
        v.violated.push("R-1.1abs".into());
    }

    // R-1.3: DeepSeek equality prompt == hit + miss (±1 token).
    if inp.spec.family == ProviderFamily::DeepSeek
        && let (Some(h), Some(m)) = (inp.hit, inp.miss)
        && (h as i64 + m as i64 - prompt as i64).abs() > 1
    {
        v.violated.push("R-1.3".into());
    }

    // R-1.4: Anthropic three-field sum — input == read + creation +
    // input_tokens(no_cache). The provider reports input_tokens excluding
    // cache; our unified total = no_cache + read + write. History-write and
    // TTL-tier checks degrade to skip when the data is unavailable.
    if inp.spec.family == ProviderFamily::Anthropic {
        let (nc, rd, wr) = (
            inp.input_no_cache,
            inp.input_cache_read,
            inp.input_cache_write,
        );
        if let (Some(nc), Some(rd), Some(wr)) = (nc, rd, wr) {
            let total = inp.prompt_tokens;
            if (nc as i64 + rd as i64 + wr as i64 - total as i64).abs() > 1 {
                v.violated.push("R-1.4".into());
            }
        }
        // First request in the scope window must report zero reads.
        if inp.first && rd.unwrap_or(0) > 0 {
            v.violated.push("R-1.4".into());
        }
    }

    // R-1.5: Bedrock equality — total == input + read + write. Quota
    // burndown needs raw usage fields; degrade to skip when absent.
    if inp.spec.family == ProviderFamily::Bedrock {
        let (nc, rd, wr) = (
            inp.input_no_cache,
            inp.input_cache_read,
            inp.input_cache_write,
        );
        if let (Some(nc), Some(rd), Some(wr)) = (nc, rd, wr) {
            let total = inp.prompt_tokens;
            if (nc as i64 + rd as i64 + wr as i64 - total as i64).abs() > 1 {
                v.violated.push("R-1.5".into());
            }
        }
    }

    // R-3.3: below threshold, claimed must be 0.
    if prompt < inp.spec.threshold && claimed > 0 {
        v.violated.push("R-3.3".into());
    }

    // R-1.6b: quantization invariant (OpenAI<5.6: %128; DeepSeek/Mistral: %64).
    if let Some(g) = inp.spec.gran
        && claimed > 0
        && !claimed.is_multiple_of(g)
        && !inp.spec.model56plus
    {
        v.violated.push("R-1.6b".into());
    }

    // R-1.7: OpenAI 5.6+ write/read equality.
    if inp.spec.model56plus {
        match (inp.write, inp.no_cache) {
            (Some(w), Some(nc)) => {
                if claimed + w + nc != prompt {
                    v.violated.push("R-1.7".into());
                }
            }
            (None, _) => {
                // Write side missing → B(medium) per RFC (Azure reports no
                // write side; responses-API naming differs — B, not W).
                v.violated.push("R-1.7b".into());
                v.notes
                    .push("5.6+ write side missing; downgraded to medium (R-1.7)".into());
            }
            _ => {}
        }
    }

    // R-1.2: first request in the scope window must have zero hits.
    if inp.first && claimed > 0 {
        v.violated.push("R-1.2".into());
    }

    // View-dependent: U and the prefix-containment invariant (R-1.1).
    let mut lcp_present = false;
    let mut u: u64 = 0;
    // Granularity floor flag (no block-level candidate; set below, used in
    // classification).
    let mut granularity_unknown = false;
    if let Some(l) = &inp.lcp {
        lcp_present = true;
        v.lcp_bytes = l.lcp_bytes;
        // R-2.1: cross-session only the system/tools segment counts;
        // same-session append-only chains may claim the whole prefix.
        let sys_bytes = inp.system_tokens.saturating_mul(4);
        let cap_bytes = if l.same_session {
            l.lcp_upper_bytes
        } else {
            l.lcp_upper_bytes.min(sys_bytes)
        };
        if !l.same_session && claimed.saturating_mul(4) > sys_bytes {
            v.notes
                .push("claimed exceeds shared system segment (R-2.1)".into());
        }
        // Byte proxy: token_upper = bytes/4 (no tokenizer attached). The
        // block UPPER bound ((j+1)·B, RFC F3) is the audit ceiling.
        let upper_bytes = l.lcp_upper_bytes.min(cap_bytes).min(inp.prompt_bytes);
        let token_upper = upper_bytes / 4;
        u = quantize_down(token_upper, inp.spec.gran).min(prompt);
    }
    if lcp_present {
        v.expected_max = u;
        let t = tau(u, inp.spec.gran);
        if claimed > u + t {
            v.violated.push("R-1.1".into());
        } else if claimed > u {
            v.violated.push("R-1.1b".into());
        }
    } else if claimed > 0 && !inp.first {
        if inp.candidate_expired {
            // A candidate existed but its TTL window expired → timing violation.
            v.violated.push("R-1.8".into());
        } else {
            // No block-granularity candidate at all: short bodies / sub-block
            // growth cannot be matched at block level. Conservative UNKNOWN —
            // never a false accusation. Hard invariants (R-1.3 / R-3.3 /
            // R-1.1abs) still take precedence in classification.
            v.notes.push(
                "no block-granularity match; cannot confirm or refute (short body / sub-block growth)"
                    .into(),
            );
            granularity_unknown = true;
        }
    }

    // R-2.3: 5.6+ implicit breakpoint — large LCP with claimed=0 is legal.
    let mut suppressed_m = false;
    if inp.spec.model56plus && claimed == 0 && v.lcp_bytes / 4 > 1024 {
        v.notes
            .push("5.6+ implicit breakpoint: claimed=0 legal despite large LCP (R-2.3)".into());
        suppressed_m = true;
    }

    // R-2.2: low-hit warning M (same session ≥3 rounds + stable prefix +
    // LCP>1024 + claimed=0).
    if !suppressed_m
        && let Some(ss) = &inp.session_stats
        && ss.same_session_rounds >= 3
        && ss.prefix_stable
        && ss.lcp_gt_1024
        && claimed == 0
    {
        v.violated.push("R-2.2".into());
    }

    // Granularity floor: UNKNOWN unless a hard invariant fired.
    // Classification with strict/shared downgrade (RFC-0015 §4.3).
    const DOWNGRADE_IDS: [&str; 3] = ["R-1.1", "R-1.2", "R-1.8"];
    let mut has_w = v.violated.iter().any(|r| is_w_level(r));
    let mut has_b = v.violated.iter().any(|r| is_b_level(r));
    if !inp.strict {
        let only_downgrade_w = has_w
            && v.violated
                .iter()
                .all(|r| !is_w_level(r) || DOWNGRADE_IDS.contains(&r.as_str()));
        if only_downgrade_w {
            has_w = false;
            if lcp_present {
                has_b = true; // local source, over limit → B (medium)
            } else {
                v.kind = VerdictKind::Unknown;
                v.confidence = VerdictConfidence::Medium;
                v.notes
                    .push("no local history source; other process may have warmed (R-5.3)".into());
                return v;
            }
        }
    }

    // Byte-proxy cap (RFC F2): without a tokenizer, R-1.1 evidence can
    // never rise above Medium — byte length overestimates token sharing for
    // non-4-bytes/token corpora.
    if inp.byte_proxy && has_w && !v.violated.iter().any(|r| r == "R-1.1abs") {
        let byte_proxy_only = v
            .violated
            .iter()
            .filter(|r| is_w_level(r))
            .all(|r| r == "R-1.1");
        if byte_proxy_only {
            has_w = false;
            has_b = true;
            v.notes
                .push("byte-proxy evidence caps at medium (RFC F2)".into());
        }
    }

    if granularity_unknown && !has_w && !has_b && !v.violated.iter().any(|r| r == "R-2.2") {
        v.kind = VerdictKind::Unknown;
        v.confidence = VerdictConfidence::Medium;
    } else if v.violated.iter().any(|r| r == "R-2.2") && !has_w {
        v.kind = VerdictKind::SuspectUnderclaim;
        v.confidence = VerdictConfidence::Low;
    } else if has_w {
        v.kind = VerdictKind::SuspectOverclaim;
        v.confidence = VerdictConfidence::High;
    } else if has_b {
        v.kind = VerdictKind::SuspectOverclaim;
        v.confidence = VerdictConfidence::Medium;
    } else {
        v.kind = VerdictKind::Trusted;
        v.confidence = VerdictConfidence::High;
    }
    v
}
