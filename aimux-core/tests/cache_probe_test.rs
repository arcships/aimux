//! Cache audit judgment scenarios.

use aimux_core::trace::{
    JudgmentInput, LcpInput, ProviderAuditSpec, VerdictKind, judge, matrix as verdict_matrix,
};

// ─────────────────────────────────────────────────────────────────────────────
// Layer 1: judge unit scenarios (prototype §9)
// ─────────────────────────────────────────────────────────────────────────────

fn spec_for(provider: &str, model: &str) -> ProviderAuditSpec {
    verdict_matrix::for_provider(provider, model)
}

fn base_input(spec: ProviderAuditSpec) -> JudgmentInput {
    JudgmentInput {
        spec,
        strict: true,
        first: false,
        prompt_tokens: 4096,
        prompt_bytes: 16384,
        claimed: 0,
        input_no_cache: None,
        input_cache_read: None,
        input_cache_write: None,
        write: Some(0),
        no_cache: None,
        hit: None,
        miss: None,
        usage_present: true,
        response_cache_header_hit: false,
        candidate_expired: false,
        byte_proxy: true,
        route_affinity_known: false,
        lcp: None,
        system_tokens: 0,
        session_stats: None,
    }
}

fn with_lcp(mut inp: JudgmentInput, lcp_bytes: u64, same_session: bool) -> JudgmentInput {
    inp.lcp = Some(LcpInput {
        lcp_bytes,
        // Block upper bound for a single 512-byte block (test convention).
        lcp_upper_bytes: lcp_bytes + 512,
        same_session,
        matched_exists: true,
    });
    inp
}

/// Scenario 1: append-only agent loop, claimed == prefix → Trusted.
#[test]
fn scenario1_append_only_loop_is_trusted() {
    let spec = spec_for("openai", "gpt-4o");
    // 3 blocks shared = 1536 bytes = 384 tokens upper bound.
    let mut inp = base_input(spec);
    inp.claimed = 384;
    inp = with_lcp(inp, 1536, true);
    let v = judge(&inp);
    assert_eq!(v.kind, VerdictKind::Trusted, "{}", v.describe());
}

/// Scenario 2: overclaim beyond the block upper bound U=(j+1)·B.
/// C4-2: without a tokenizer (byte_proxy), bytes/4 is not a safe token bound
/// (ASCII-heavy corpora underestimate tokens → `u` too small → real hits
/// falsely accused). When the overclaim rests SOLELY on R-1.1 (no independent
/// invariant), the verdict downgrades to Unknown instead of SuspectOverclaim.
/// Exact-token evidence (byte_proxy=false) keeps the original SuspectOverclaim.
#[test]
fn scenario2_overclaim_is_suspect() {
    let spec = spec_for("openai", "gpt-5.6"); // gran=None (no quantization noise)
    // 3 blocks shared → lower bound 1536 B (384 tokens), upper bound
    // (3+1)·512 = 2048 B (512 tokens). claimed=600 > 512+τ → R-1.1.
    let mut inp = base_input(spec);
    inp.claimed = 600;
    inp.no_cache = Some(4096 - 600); // R-1.7 equality holds (write=0)
    inp = with_lcp(inp, 1536, true);
    let v = judge(&inp);
    // C4-2: byte-proxy-only overclaim → Unknown (not SuspectOverclaim).
    assert_eq!(v.kind, VerdictKind::Unknown, "{}", v.describe());
    assert!(
        v.violated.iter().any(|r| r == "R-1.1"),
        "R-1.1 still fires; only the verdict is downgraded: {}",
        v.describe()
    );
    assert!(
        v.notes.iter().any(|n| n.contains("C4-2")),
        "downgrade must be explained: {}",
        v.describe()
    );

    // With exact token evidence the same violation is SuspectOverclaim (High).
    let mut exact = base_input(spec);
    exact.byte_proxy = false;
    exact.claimed = 600;
    exact.no_cache = Some(4096 - 600);
    exact = with_lcp(exact, 1536, true);
    let ve = judge(&exact);
    assert_eq!(ve.kind, VerdictKind::SuspectOverclaim, "{}", ve.describe());
    assert_eq!(
        ve.confidence,
        aimux_core::trace::VerdictConfidence::High,
        "{}",
        ve.describe()
    );

    // Within the block upper bound → Trusted (block-granularity ceiling).
    let mut within = base_input(spec);
    within.claimed = 500;
    within.no_cache = Some(4096 - 500);
    within = with_lcp(within, 1536, true);
    let vw = judge(&within);
    assert_eq!(vw.kind, VerdictKind::Trusted, "{}", vw.describe());
}

/// Scenario 3: first request with claimed > 0 → W (strict) / B (shared).
#[test]
fn scenario3_first_request_zero_hits() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.first = true;
    inp.claimed = 2048; // 128-token quantization aligned
    let v = judge(&inp);
    assert_eq!(v.kind, VerdictKind::SuspectOverclaim, "{}", v.describe());
    assert!(v.violated.iter().any(|r| r == "R-1.2"));

    // Shared mode, no local history: UNKNOWN (other process may have warmed).
    let mut shared = base_input(spec);
    shared.strict = false;
    shared.first = true;
    shared.claimed = 2048;
    let vs = judge(&shared);
    assert_eq!(vs.kind, VerdictKind::Unknown, "{}", vs.describe());
}

/// Scenario 4: 5.6+ implicit breakpoint — large LCP with claimed=0 is legal.
#[test]
fn scenario4_implicit_breakpoint_not_false_positive() {
    let spec = spec_for("openai", "gpt-5.6");
    let mut inp = base_input(spec);
    inp.claimed = 0;
    inp = with_lcp(inp, 16384, true); // LCP > 1024 tokens
    let v = judge(&inp);
    assert_eq!(v.kind, VerdictKind::Trusted, "{}", v.describe());
    assert!(!v.violated.iter().any(|r| r == "R-2.2"));
}

/// Scenario 5: DeepSeek equality — hit + miss == prompt (±1).
#[test]
fn scenario5_deepseek_equality() {
    let spec = spec_for("deepseek", "deepseek-chat");
    let mut inp = base_input(spec);
    inp.prompt_tokens = 1000;
    inp.claimed = 500;
    inp.hit = Some(500);
    inp.miss = Some(510); // 500+510 != 1000 → violation
    let v = judge(&inp);
    assert!(v.violated.iter().any(|r| r == "R-1.3"), "{}", v.describe());
    assert_eq!(v.kind, VerdictKind::SuspectOverclaim);

    // Equality holds → no R-1.3.
    let mut ok = base_input(spec);
    ok.prompt_tokens = 1000;
    ok.claimed = 500;
    ok.hit = Some(500);
    ok.miss = Some(500);
    ok = with_lcp(ok, 2048, true);
    let v2 = judge(&ok);
    assert!(
        !v2.violated.iter().any(|r| r == "R-1.3"),
        "{}",
        v2.describe()
    );
}

/// Scenario 6: TTL — the store lookup returns no live source; claimed > 0
/// without a live history source is a timing violation (R-1.8).
#[test]
fn scenario6_ttl_violation() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.claimed = 128;
    inp.lcp = None; // no live source
    inp.candidate_expired = true; // …because the candidate expired (TTL)
    let v = judge(&inp);
    assert!(v.violated.iter().any(|r| r == "R-1.8"), "{}", v.describe());

    // Granularity floor (no candidate at all) → conservative UNKNOWN.
    let mut floor = base_input(spec);
    floor.claimed = 128;
    floor.lcp = None;
    floor.candidate_expired = false;
    let vf = judge(&floor);
    assert_eq!(vf.kind, VerdictKind::Unknown, "{}", vf.describe());
}

/// Scenario 7: below the 1024 threshold with claimed > 0 → W (R-3.3).
#[test]
fn scenario7_threshold() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.prompt_tokens = 800; // < 1024
    inp.claimed = 100;
    let v = judge(&inp);
    assert!(v.violated.iter().any(|r| r == "R-3.3"), "{}", v.describe());
    assert_eq!(v.kind, VerdictKind::SuspectOverclaim);
}

/// Scenario 8: cross-session — only the shared system segment counts.
/// C4-2: the cross-session overclaim here rests solely on R-1.1 (byte proxy),
/// so it downgrades to Unknown; with exact-token evidence it is SuspectOverclaim.
#[test]
fn scenario8_cross_session_system_segment_only() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.system_tokens = 128; // system segment = 128 tokens
    inp.claimed = 512; // > 128 tokens → over the shared segment (128-aligned)
    inp = with_lcp(inp, 2048, false); // large cross-session LCP
    let v = judge(&inp);
    // C4-2: byte-proxy-only overclaim → Unknown (R-1.1 still fires).
    assert_eq!(v.kind, VerdictKind::Unknown, "{}", v.describe());
    assert!(
        v.violated.iter().any(|r| r == "R-1.1"),
        "R-1.1 still fires: {}",
        v.describe()
    );
    assert!(
        v.notes.iter().any(|n| n.contains("shared system segment")),
        "R-2.1 note still present: {}",
        v.describe()
    );

    // With exact token evidence the same cross-session overclaim is W (High).
    let mut exact = base_input(spec);
    exact.byte_proxy = false;
    exact.system_tokens = 128;
    exact.claimed = 512;
    exact = with_lcp(exact, 2048, false);
    let ve = judge(&exact);
    assert_eq!(
        ve.kind,
        VerdictKind::SuspectOverclaim,
        "exact-token cross-session overclaim stands: {}",
        ve.describe()
    );

    // Within the system segment → OK.
    let mut ok = base_input(spec);
    ok.system_tokens = 512;
    ok.claimed = 128;
    ok = with_lcp(ok, 2048, false);
    let v2 = judge(&ok);
    assert_eq!(v2.kind, VerdictKind::Trusted, "{}", v2.describe());
}

/// R-4.1: response-cache header hit → not audited.
#[test]
fn response_cache_header_hit_skips_audit() {
    let spec = spec_for("openrouter", "gpt-4o");
    let mut inp = base_input(spec);
    inp.response_cache_header_hit = true;
    inp.claimed = 999999; // would be a violation otherwise
    let v = judge(&inp);
    assert_eq!(v.kind, VerdictKind::Unknown, "{}", v.describe());
    assert!(v.violated.iter().any(|r| r == "R-4.1"));
}

/// R-1.4: Anthropic three-field sum + first-request read==0.
#[test]
fn scenario_r14_anthropic_three_field_sum() {
    let spec = spec_for("anthropic", "claude-3-5-sonnet");
    // Unified usage: total == no_cache + read + write.
    let mut ok = base_input(spec);
    ok.prompt_tokens = 1000;
    ok.input_no_cache = Some(500);
    ok.input_cache_read = Some(300);
    ok.input_cache_write = Some(200);
    let v = judge(&ok);
    assert!(!v.violated.iter().any(|r| r == "R-1.4"), "{}", v.describe());

    // Sum violation → W.
    let mut bad = base_input(spec);
    bad.prompt_tokens = 1000;
    bad.input_no_cache = Some(500);
    bad.input_cache_read = Some(400); // 500+400+200 != 1000
    bad.input_cache_write = Some(200);
    let vb = judge(&bad);
    assert!(
        vb.violated.iter().any(|r| r == "R-1.4"),
        "{}",
        vb.describe()
    );
    assert_eq!(vb.kind, VerdictKind::SuspectOverclaim);

    // First request reporting reads → R-1.4.
    let mut first = base_input(spec);
    first.first = true;
    first.input_no_cache = Some(1000);
    first.input_cache_read = Some(50);
    first.input_cache_write = Some(0);
    let vf = judge(&first);
    assert!(
        vf.violated.iter().any(|r| r == "R-1.4"),
        "{}",
        vf.describe()
    );
}

/// R-1.5: Bedrock equality — total == input + read + write.
#[test]
fn scenario_r15_bedrock_equality() {
    let spec = spec_for("bedrock", "claude-3-5-sonnet-v2");
    let mut ok = base_input(spec);
    ok.prompt_tokens = 1000;
    ok.input_no_cache = Some(800);
    ok.input_cache_read = Some(150);
    ok.input_cache_write = Some(50);
    let v = judge(&ok);
    assert!(!v.violated.iter().any(|r| r == "R-1.5"), "{}", v.describe());

    let mut bad = base_input(spec);
    bad.prompt_tokens = 1000;
    bad.input_no_cache = Some(800);
    bad.input_cache_read = Some(300);
    bad.input_cache_write = Some(50);
    let vb = judge(&bad);
    assert!(
        vb.violated.iter().any(|r| r == "R-1.5"),
        "{}",
        vb.describe()
    );
    assert_eq!(vb.kind, VerdictKind::SuspectOverclaim);
}

/// R-5.1: usage missing → Unknown.
#[test]
fn missing_usage_is_unknown() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.usage_present = false;
    inp.claimed = 100;
    let v = judge(&inp);
    assert_eq!(v.kind, VerdictKind::Unknown, "{}", v.describe());
}
