//! RFC-0015 cache-probe tests: prototype scenario migration (§9, 8 scenarios)
//! + TraceLayer end-to-end probes.
//!
//! Layer 1: `judge` unit scenarios — exact control over every input.
//! Layer 2: `TraceLayer` + `RingTraceStore` + `RuleAuditor` end-to-end with a
//! mock model serving synthetic bodies (4 bytes per token, hex-encoded, so
//! the byte proxy `LCPb/4` matches the token counts).

use std::sync::Arc;

use futures::executor::block_on;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{
    JudgmentInput, LcpInput, ProviderAuditSpec, RingTraceStore, TraceFilter, TraceLayer,
    VerdictKind, judge, matrix as verdict_matrix,
};
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, TokenUsage, Usage};

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
/// Byte-proxy evidence caps at Medium (RFC F2); exact-token evidence is High.
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
    assert_eq!(v.kind, VerdictKind::SuspectOverclaim, "{}", v.describe());
    assert!(v.violated.iter().any(|r| r == "R-1.1"));
    // Byte proxy caps the confidence at Medium (RFC F2).
    assert_eq!(
        v.confidence,
        aimux_core::trace::VerdictConfidence::Medium,
        "{}",
        v.describe()
    );

    // With exact token evidence the same violation is High.
    let mut exact = base_input(spec);
    exact.byte_proxy = false;
    exact.claimed = 600;
    exact.no_cache = Some(4096 - 600);
    exact = with_lcp(exact, 1536, true);
    let ve = judge(&exact);
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
#[test]
fn scenario8_cross_session_system_segment_only() {
    let spec = spec_for("openai", "gpt-4o");
    let mut inp = base_input(spec);
    inp.system_tokens = 128; // system segment = 128 tokens
    inp.claimed = 512; // > 128 tokens → over the shared segment (128-aligned)
    inp = with_lcp(inp, 2048, false); // large cross-session LCP
    let v = judge(&inp);
    // Claimed exceeds the shared system segment → W via R-1.1.
    assert_eq!(v.kind, VerdictKind::SuspectOverclaim, "{}", v.describe());
    assert!(v.notes.iter().any(|n| n.contains("shared system segment")));

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

// ─────────────────────────────────────────────────────────────────────────────
// Layer 2: TraceLayer end-to-end (mock model, synthetic bodies)
// ─────────────────────────────────────────────────────────────────────────────

/// 4 bytes per token, hex-encoded — the byte proxy LCPb/4 matches token
/// counts exactly (system segment is shared across sessions).
fn token_body(system_tokens: u64, conv_tokens: u64, conv_start: u64) -> serde_json::Value {
    let mut s = String::new();
    for i in 0..system_tokens {
        s.push_str(&format!("{:04x}", i & 0xffff));
    }
    for i in 0..conv_tokens {
        s.push_str(&format!("{:04x}", (conv_start + i) & 0xffff));
    }
    serde_json::json!({
        "model": "mock-model",
        "messages": [{"role": "user", "content": s}],
    })
}

struct MockModel {
    body: serde_json::Value,
    usage: Usage,
}

impl MockModel {
    fn new(body: serde_json::Value, claimed: u64, prompt_tokens: u64) -> Self {
        Self {
            body,
            usage: Usage {
                input_tokens: TokenUsage {
                    total: Some(prompt_tokens as u32),
                    no_cache: Some((prompt_tokens - claimed) as u32),
                    cache_read: Some(claimed as u32),
                    cache_write: Some(0),
                    ..Default::default()
                },
                output_tokens: TokenUsage {
                    total: Some(10),
                    text: Some(10),
                    ..Default::default()
                },
                raw: None,
            },
        }
    }
}

#[async_trait::async_trait]
impl LanguageModel for MockModel {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "mock-1"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
            usage: self.usage.clone(),
            warnings: vec![],
            provider_metadata: None,
            response: ResponseMetadata {
                id: Some("req-1".into()),
                timestamp: None,
                model_id: Some("mock-1".into()),
            },
            request_body: Some(self.body.clone()),
            response_headers: None,
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let usage = self.usage.clone();
        let body = self.body.clone();
        Ok(StreamResult {
            stream: Box::pin(async_stream::stream! {
                yield Ok(StreamPart::StreamStart { warnings: vec![] });
                yield Ok(StreamPart::TextDelta {
                    id: "t1".into(),
                    delta: "hi".into(),
                    provider_metadata: None,
                });
                yield Ok(StreamPart::Finish {
                    finish_reason: FinishReason {
                        unified: FinishReasonUnified::Stop,
                        raw: None,
                    },
                    usage,
                    provider_metadata: None,
                });
            }),
            request_body: Some(body),
            response_headers: None,
        })
    }
}

fn model_opts(session_id: Option<&str>) -> CallOptions {
    let mut o = CallOptions::new(Default::default());
    o.session_id = session_id.map(|s| s.to_string());
    o
}

/// End-to-end: append-only loop with claimed == shared block prefix →
/// Trusted; overclaim → SuspectOverclaim; aggregation + session chain +
/// JSONL work. Bodies > 4 KiB so block-aligned prefixes match.
#[test]
fn trace_layer_end_to_end() {
    let store = Arc::new(RingTraceStore::new());
    let inner: Arc<dyn LanguageModel> = Arc::new(MockModel::new(
        token_body(1024, 512, 0), // 6144 B → block 1 matches later calls
        0,
        1536,
    ));
    let layer = Arc::new(TraceLayer::new(inner, store.clone()).with_rules_auditor(true));

    // Call 1: first request — claimed 0 → Trusted.
    let r1 = block_on(layer.do_generate(&model_opts(Some("sess-1")))).unwrap();
    assert!(r1.usage.input_tokens.total.is_some());

    // Call 2: same session, prefix extended; claimed matches the shared
    // first block (1024 tokens) → Trusted.
    let inner2: Arc<dyn LanguageModel> = Arc::new(MockModel::new(
        token_body(1024, 1536, 0), // 10240 B → block 1 shared with call 1
        1024,                      // claimed == shared block tokens
        2560,
    ));
    let layer2 = Arc::new(TraceLayer::new(inner2, store.clone()).with_rules_auditor(true));
    block_on(layer2.do_generate(&model_opts(Some("sess-1")))).unwrap();

    // Call 3: overclaim — claimed exceeds the client LCP bound (1024 + τ).
    let inner3: Arc<dyn LanguageModel> = Arc::new(MockModel::new(
        token_body(1024, 1536, 0),
        1200, // > 1024 + 128 → SuspectOverclaim
        2560,
    ));
    let layer3 = Arc::new(TraceLayer::new(inner3, store.clone()).with_rules_auditor(true));
    block_on(layer3.do_generate(&model_opts(Some("sess-1")))).unwrap();

    let recs = store.aggregate(&TraceFilter::default());
    assert_eq!(recs.len(), 1);
    let s = &recs[0];
    assert_eq!(s.requests, 3);
    // 2 Trusted + 1 SuspectOverclaim.
    assert_eq!(s.verdict_counts.get("Trusted"), Some(&2));
    assert_eq!(s.verdict_counts.get("SuspectOverclaim"), Some(&1));
    assert!(s.reported_hit_rate.is_some());

    // Session chain: 3 records, stable prefix (LCP grows monotonically).
    let chain = store.session_chain("sess-1").expect("chain exists");
    assert_eq!(chain.record_ids.len(), 3);
    assert!(chain.prefix_stability >= 0.5);

    // JSONL export round-trips.
    let mut buf = Vec::new();
    store.export_jsonl(&mut buf).unwrap();
    let lines: Vec<&[u8]> = buf
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 3, "one TraceRecord per line");
}

/// Streaming path records TTFT + usage from the Finish part.
#[test]
fn trace_layer_stream_records_ttft_and_usage() {
    let store = Arc::new(RingTraceStore::new());
    let inner: Arc<dyn LanguageModel> = Arc::new(MockModel::new(token_body(1024, 512, 0), 0, 1536));
    let layer = Arc::new(TraceLayer::new(inner, store.clone()).with_rules_auditor(true));

    let mut result = block_on(layer.do_stream(&model_opts(Some("sess-1")))).unwrap();
    // Consume the stream fully (records on completion).
    while let Some(_part) = block_on(async { futures::StreamExt::next(&mut result.stream).await }) {
    }

    let recs = store.aggregate(&TraceFilter::default());
    assert_eq!(recs[0].requests, 1);
    let rec = store.session_chain("sess-1").unwrap();
    assert_eq!(rec.record_ids.len(), 1);

    // Verify TTFT was captured on the stored record.
    let json = {
        let mut buf = Vec::new();
        store.export_jsonl(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    };
    assert!(
        json.contains("\"ttft_ms\":"),
        "streamed record must carry ttft_ms: {json}"
    );
}

/// StreamPart::Error (a provider-reported mid-stream error) is recorded.
#[test]
fn trace_layer_records_stream_part_error() {
    struct MidStreamError;
    #[async_trait::async_trait]
    impl LanguageModel for MidStreamError {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-stream-err"
        }
        async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unreachable!()
        }
        async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
            Ok(StreamResult {
                stream: Box::pin(async_stream::stream! {
                    yield Ok(StreamPart::StreamStart { warnings: vec![] });
                    // Provider-reported mid-stream error (Ok(Error{..})).
                    yield Ok(StreamPart::Error { error: AiMuxError::Stream("mid-stream failure".into()) });
                }),
                request_body: None,
                response_headers: None,
            })
        }
    }

    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(Arc::new(MidStreamError), store.clone()));
    let mut result = block_on(layer.do_stream(&model_opts(Some("sess-err")))).unwrap();
    while let Some(_p) = block_on(async { futures::StreamExt::next(&mut result.stream).await }) {}

    let recs = store.aggregate(&TraceFilter::default());
    assert_eq!(recs[0].requests, 1);
    assert_eq!(recs[0].errors, 1, "provider-reported stream error recorded");
    let json = {
        let mut buf = Vec::new();
        store.export_jsonl(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    };
    assert!(json.contains("mid-stream failure"), "{json}");
}

/// Transport-level `Err` from the stream is also recorded.
#[test]
fn trace_layer_records_stream_transport_error() {
    struct TransportErr;
    #[async_trait::async_trait]
    impl LanguageModel for TransportErr {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-transport-err"
        }
        async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unreachable!()
        }
        async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
            Ok(StreamResult {
                stream: Box::pin(async_stream::stream! {
                    yield Ok(StreamPart::StreamStart { warnings: vec![] });
                    yield Err(AiMuxError::Stream("transport failure".into()));
                }),
                request_body: None,
                response_headers: None,
            })
        }
    }

    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(Arc::new(TransportErr), store.clone()));
    let mut result = block_on(layer.do_stream(&model_opts(Some("sess-err")))).unwrap();
    while let Some(_p) = block_on(async { futures::StreamExt::next(&mut result.stream).await }) {}

    let recs = store.aggregate(&TraceFilter::default());
    assert_eq!(recs[0].errors, 1, "transport error recorded");
}

/// Failures (Err from do_generate) are still recorded, without a verdict.
#[test]
fn trace_layer_records_failures() {
    struct Failing;
    #[async_trait::async_trait]
    impl LanguageModel for Failing {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-fail"
        }
        async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            Err(AiMuxError::Other("boom".into()))
        }
        async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
            Err(AiMuxError::Other("boom".into()))
        }
    }

    let store = Arc::new(RingTraceStore::new());
    let layer = Arc::new(TraceLayer::new(Arc::new(Failing), store.clone()));
    assert!(block_on(layer.do_generate(&model_opts(Some("sess-err")))).is_err());

    let recs = store.aggregate(&TraceFilter::default());
    assert_eq!(recs[0].requests, 1);
    assert_eq!(recs[0].errors, 1);
    let json = {
        let mut buf = Vec::new();
        store.export_jsonl(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    };
    assert!(json.contains("\"error\":\"boom\""), "{json}");
}

/// Aggregation: client_upper_bound_hit_rate sums per-record LCP token upper
/// bounds only — records without LCP evidence contribute 0, never the full
/// request length (RFC-0015 §5.2).
#[test]
fn aggregate_uses_lcp_upper_bound_not_full_length() {
    use aimux_core::trace::TraceStats;

    let store = Arc::new(RingTraceStore::new());

    // Call 1: first request — no history → no LCP upper bound.
    let inner1: Arc<dyn LanguageModel> =
        Arc::new(MockModel::new(token_body(1024, 512, 0), 0, 1536));
    let layer1 = Arc::new(TraceLayer::new(inner1, store.clone()).with_rules_auditor(true));
    block_on(layer1.do_generate(&model_opts(Some("sess-1")))).unwrap();

    // Call 2: same-session prefix continuation → block-1 LCP upper bound
    // (4096 B → 1024 tokens).
    let inner2: Arc<dyn LanguageModel> =
        Arc::new(MockModel::new(token_body(1024, 1536, 0), 1024, 2560));
    let layer2 = Arc::new(TraceLayer::new(inner2, store.clone()).with_rules_auditor(true));
    block_on(layer2.do_generate(&model_opts(Some("sess-1")))).unwrap();

    let stats: Vec<TraceStats> = store.aggregate(&TraceFilter::default());
    assert_eq!(stats.len(), 1);
    let s = &stats[0];
    assert_eq!(s.requests, 2);
    let rate = s.client_upper_bound_hit_rate.expect("rate present");
    // LCP upper contributions: call1 = 0 (no match), call2 = 2×4096B/4 =
    // 2048 tokens; total input = 1536+2560.
    let expected = 2048.0 / (1536.0 + 2560.0);
    assert!(
        (rate - expected).abs() < 1e-9,
        "client upper bound must be LCP-based: {rate} vs {expected}"
    );
    assert!(
        rate < 0.9,
        "full-length estimate would be ~1.0 — LCP evidence must cap it: {rate}"
    );
}
