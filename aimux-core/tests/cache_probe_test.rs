//! RFC-0015 cache-probe tests: prototype scenario migration (§9, 8 scenarios)
//! + TraceLayer end-to-end probes.
//!
//! Layer 1: `judge` unit scenarios — exact control over every input.
//! Layer 2: `TraceLayer` + `RingTraceStore` + `RuleAuditor` end-to-end with a
//! mock model serving synthetic bodies (4 bytes per token, hex-encoded, so
//! the byte proxy `LCPb/4` matches the token counts). Records are read back
//! through the JSONL export and deserialized, so the checks hold whatever the
//! wire spelling of the record fields is.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::executor::block_on;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::operation::OperationSettings;
use aimux_core::options::CallOptions;
use aimux_core::recording::{RecordingContext, RingRecorder};
use aimux_core::result::{GenerateResult, GeneratedFile, StreamResult};
use aimux_core::shared::{GeneratedFileData, RequestInfo};
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{
    JudgmentInput, LcpInput, ProviderAuditSpec, RingTraceStore, SessionStats, TraceFilter,
    TraceLayer, TraceRecord, VerdictConfidence, VerdictKind, judge, matrix as verdict_matrix,
};
use aimux_core::types::{
    FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, Usage,
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

// ─────────────────────────────────────────────────────────────────────────────
// Layer 2: TraceLayer end-to-end (mock model, synthetic bodies)
// ─────────────────────────────────────────────────────────────────────────────

type Part = Result<StreamPart, AiMuxError>;

/// 4 bytes per token, hex-encoded — the byte proxy LCPb/4 matches token
/// counts exactly (system segment is shared across sessions).
fn token_body(system_tokens: u64, conv_tokens: u64, conv_start: u64) -> Value {
    let mut s = String::new();
    for i in 0..system_tokens {
        s.push_str(&format!("{:04x}", i & 0xffff));
    }
    for i in 0..conv_tokens {
        s.push_str(&format!("{:04x}", (conv_start + i) & 0xffff));
    }
    json!({
        "model": "mock-model",
        "messages": [{"role": "user", "content": s}],
    })
}

fn usage(claimed: u32, prompt_tokens: u32) -> Usage {
    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(prompt_tokens - claimed),
            cache_read: Some(claimed),
            cache_write: Some(0),
        },
        output_tokens: OutputTokenUsage {
            total: Some(10),
            text: Some(10),
            reasoning: None,
        },
        raw: None,
    }
}

fn stop() -> FinishReason {
    FinishReason {
        unified: FinishReasonUnified::Stop,
        raw: None,
    }
}

fn start() -> StreamPart {
    StreamPart::StreamStart { warnings: vec![] }
}

fn text_delta(delta: &str) -> StreamPart {
    StreamPart::TextDelta {
        id: "t1".into(),
        delta: delta.into(),
        provider_metadata: None,
    }
}

fn finish(usage: Usage) -> StreamPart {
    StreamPart::Finish {
        finish_reason: stop(),
        usage,
        provider_metadata: None,
    }
}

/// A model serving a synthetic request body, the given usage and (optionally)
/// response headers, on both paths.
struct MockModel {
    provider: &'static str,
    model: &'static str,
    body: Value,
    usage: Usage,
    headers: Option<HashMap<String, String>>,
}

impl MockModel {
    fn new(body: Value, claimed: u32, prompt_tokens: u32) -> Self {
        Self::with_identity("mock", "mock-1", body, claimed, prompt_tokens)
    }

    /// Configurable identity (B1: same model_id, different provider).
    fn with_identity(
        provider: &'static str,
        model: &'static str,
        body: Value,
        claimed: u32,
        prompt_tokens: u32,
    ) -> Self {
        Self {
            provider,
            model,
            body,
            usage: usage(claimed, prompt_tokens),
            headers: None,
        }
    }

    fn request(&self) -> Option<RequestInfo> {
        Some(RequestInfo {
            body: Some(self.body.clone()),
        })
    }
}

#[async_trait]
impl LanguageModel for MockModel {
    fn provider(&self) -> &str {
        self.provider
    }

    fn model_id(&self) -> &str {
        self.model
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Ok(GenerateResult {
            content: vec![],
            finish_reason: stop(),
            usage: self.usage.clone(),
            warnings: vec![],
            provider_metadata: None,
            // Built from JSON, so the test does not name the response type.
            response: Some(
                serde_json::from_value(json!({ "id": "req-1", "headers": self.headers }))
                    .expect("response info"),
            ),
            request: self.request(),
        })
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let parts: Vec<Part> = vec![
            Ok(start()),
            Ok(text_delta("hi")),
            Ok(finish(self.usage.clone())),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request: self.request(),
            // Built from JSON, as above.
            response: Some(
                serde_json::from_value(json!({ "headers": self.headers }))
                    .expect("stream response info"),
            ),
        })
    }
}

/// A stream-only model that plays a fixed script of parts.
struct Scripted {
    model: &'static str,
    parts: fn() -> Vec<Part>,
}

#[async_trait]
impl LanguageModel for Scripted {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        self.model
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unreachable!("stream-only mock")
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter((self.parts)())),
            request: Some(RequestInfo {
                body: Some(token_body(1024, 512, 0)),
            }),
            response: None,
        })
    }
}

/// Provider-reported mid-stream error (`Ok(StreamPart::Error { .. })`).
fn mid_stream_error() -> Vec<Part> {
    vec![
        Ok(start()),
        Ok(StreamPart::Error {
            error: AiMuxError::InvalidResponseData("mid-stream failure".into()),
        }),
    ]
}

/// Transport-level `Err` item.
fn transport_error() -> Vec<Part> {
    vec![
        Ok(start()),
        Err(AiMuxError::InvalidResponseData("transport failure".into())),
    ]
}

/// Five deltas, then `Finish`.
fn five_chunks() -> Vec<Part> {
    let mut parts: Vec<Part> = vec![Ok(start())];
    parts.extend((0..5).map(|i| Ok(text_delta(&format!("chunk{i}")))));
    parts.push(Ok(finish(Usage::default())));
    parts
}

/// A generated file is the only model output.
fn file_then_finish() -> Vec<Part> {
    vec![
        Ok(start()),
        Ok(StreamPart::File(png_url())),
        Ok(finish(Usage::default())),
    ]
}

/// No model output at all.
fn no_output() -> Vec<Part> {
    vec![Ok(start()), Ok(finish(Usage::default()))]
}

/// The one place naming the stream's file type.
fn png_url() -> GeneratedFile {
    GeneratedFile {
        data: GeneratedFileData::Url {
            url: "https://example.test/image.png".into(),
            original_url: None,
        },
        media_type: "image/png".into(),
        provider_metadata: None,
    }
}

/// A model whose every call fails.
struct Failing;

#[async_trait]
impl LanguageModel for Failing {
    fn provider(&self) -> &str {
        "mock"
    }

    fn model_id(&self) -> &str {
        "mock-fail"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Err(AiMuxError::Other("boom".into()))
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        Err(AiMuxError::Other("boom".into()))
    }
}

fn traced(model: impl LanguageModel + 'static, store: &Arc<RingTraceStore>) -> TraceLayer {
    TraceLayer::new(Arc::new(model), store.clone())
}

fn audited(model: impl LanguageModel + 'static, store: &Arc<RingTraceStore>) -> TraceLayer {
    traced(model, store).with_rules_auditor(true)
}

fn opts() -> CallOptions {
    CallOptions::new(Vec::new())
}

/// A recording context with `call_id`.
fn recorded(call_id: &str, recorder: &Arc<RingRecorder>) -> Option<RecordingContext> {
    Some(RecordingContext::new(call_id, recorder.clone()))
}

/// One non-streaming call through `layer`, in `session`. Only this function
/// and `stream_in` know where the session id goes.
fn generate_in(
    layer: &TraceLayer,
    session: Option<&str>,
    options: CallOptions,
) -> Result<GenerateResult, AiMuxError> {
    block_on(in_session(session).scope(layer.do_generate(&options)))
}

/// One streaming call through `layer`, in `session` (stream not consumed).
fn stream_in(
    layer: &TraceLayer,
    session: Option<&str>,
    options: CallOptions,
) -> Result<StreamResult, AiMuxError> {
    block_on(in_session(session).scope(layer.do_stream(&options)))
}

/// The operation settings carrying `session`.
fn in_session(session: Option<&str>) -> OperationSettings {
    OperationSettings {
        session_id: session.map(str::to_owned),
        ..OperationSettings::default()
    }
}

/// Polls a stream to its end, then drops it.
fn drain(result: StreamResult) {
    let mut stream = result.stream;
    while block_on(stream.next()).is_some() {}
}

/// The store's records in call order, read back through the JSONL export.
fn records(store: &RingTraceStore) -> Vec<TraceRecord> {
    let mut buf = Vec::new();
    store.export_jsonl(&mut buf).expect("export");
    String::from_utf8(buf)
        .expect("utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one TraceRecord per line"))
        .collect()
}

/// End-to-end: append-only loop with claimed == shared block prefix →
/// Trusted; overclaim → SuspectOverclaim; aggregation + session chain +
/// JSONL work. Bodies > 4 KiB so block-aligned prefixes match.
#[test]
fn trace_layer_end_to_end() {
    let store = Arc::new(RingTraceStore::new());

    // Call 1: first request — claimed 0 → Trusted.
    // 6144 B → block 1 matches later calls.
    let layer1 = audited(MockModel::new(token_body(1024, 512, 0), 0, 1536), &store);
    let r1 = generate_in(&layer1, Some("sess-1"), opts()).unwrap();
    assert!(r1.usage.input_tokens.total.is_some());

    // Call 2: same session, prefix extended; claimed matches the shared
    // first block (1024 tokens) → Trusted. 10240 B → block 1 shared.
    let layer2 = audited(
        MockModel::new(token_body(1024, 1536, 0), 1024, 2560),
        &store,
    );
    generate_in(&layer2, Some("sess-1"), opts()).unwrap();

    // Call 3: overclaim — claimed exceeds the client LCP bound (1024 + τ).
    let layer3 = audited(
        MockModel::new(token_body(1024, 1536, 0), 1200, 2560),
        &store,
    );
    generate_in(&layer3, Some("sess-1"), opts()).unwrap();

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(stats.len(), 1);
    let s = &stats[0];
    assert_eq!(s.requests, 3);
    assert_eq!(s.errors, 0);
    // 2 Trusted + 1 SuspectOverclaim.
    assert_eq!(s.verdict_counts.get("Trusted"), Some(&2));
    assert_eq!(s.verdict_counts.get("SuspectOverclaim"), Some(&1));
    assert!(s.reported_hit_rate.is_some());

    // Session chain: stable prefix (LCP grows monotonically).
    let chain = store.session_chain("sess-1").expect("chain exists");
    assert!(chain.prefix_stability >= 0.5);

    // JSONL export round-trips: one record per call, in call order, each with
    // the response id and the session of its call.
    let recs = records(&store);
    assert_eq!(recs.len(), 3, "one TraceRecord per line");
    let ids: Vec<&str> = recs.iter().map(|rec| rec.call_id.as_str()).collect();
    assert_eq!(chain.record_ids, ids);
    for rec in &recs {
        assert_eq!(rec.request_id.as_deref(), Some("req-1"));
        assert_eq!(rec.session_id.as_deref(), Some("sess-1"));
        assert_eq!(rec.error, None);
    }
}

/// Streaming path records TTFT + usage from the Finish part.
#[test]
fn trace_layer_stream_records_ttft_and_usage() {
    let store = Arc::new(RingTraceStore::new());
    let layer = audited(MockModel::new(token_body(1024, 512, 0), 0, 1536), &store);
    drain(stream_in(&layer, Some("sess-1"), opts()).unwrap());

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(stats[0].requests, 1);
    assert!(stats[0].ttft_p50_ms.is_some());
    let chain = store.session_chain("sess-1").expect("chain exists");
    assert_eq!(chain.record_ids.len(), 1);

    // The deleted version matched `"ttft_ms":` in the JSONL, which a null
    // also matches; check the value.
    let recs = records(&store);
    let rec = &recs[0];
    assert!(
        rec.ttft_ms.is_some(),
        "streamed record must carry ttft_ms: {rec:?}"
    );
    assert_eq!(rec.usage.input_total, Some(1536));
    assert_eq!(rec.usage.cache_read, Some(0));
    assert_eq!(rec.usage.output_total, Some(10));
    assert_eq!(rec.error, None);
    assert!(rec.verdict.is_some(), "rules auditor attached");
}

/// StreamPart::Error (a provider-reported mid-stream error) is recorded.
#[test]
fn trace_layer_records_stream_part_error() {
    let store = Arc::new(RingTraceStore::new());
    let model = Scripted {
        model: "mock-stream-err",
        parts: mid_stream_error,
    };
    let layer = traced(model, &store);
    drain(stream_in(&layer, Some("sess-err"), opts()).unwrap());

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(stats[0].requests, 1);
    assert_eq!(
        stats[0].errors, 1,
        "provider-reported stream error recorded"
    );
    let expected = AiMuxError::InvalidResponseData("mid-stream failure".into()).to_string();
    assert_eq!(records(&store)[0].error, Some(expected));
}

/// Transport-level `Err` from the stream is also recorded.
#[test]
fn trace_layer_records_stream_transport_error() {
    let store = Arc::new(RingTraceStore::new());
    let model = Scripted {
        model: "mock-transport-err",
        parts: transport_error,
    };
    let layer = traced(model, &store);
    drain(stream_in(&layer, Some("sess-err"), opts()).unwrap());

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(stats[0].errors, 1, "transport error recorded");
    let expected = AiMuxError::InvalidResponseData("transport failure".into()).to_string();
    assert_eq!(records(&store)[0].error, Some(expected));
}

/// Failures (`Err` from do_generate or do_stream) are still recorded, without
/// a verdict (no auditor attached).
#[test]
fn trace_layer_records_failures() {
    let store = Arc::new(RingTraceStore::new());
    let layer = traced(Failing, &store);
    assert!(generate_in(&layer, Some("sess-err"), opts()).is_err());
    assert!(stream_in(&layer, Some("sess-err"), opts()).is_err());

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(stats[0].requests, 2);
    assert_eq!(stats[0].errors, 2);
    for rec in records(&store) {
        assert_eq!(rec.error.as_deref(), Some("boom"));
        assert!(rec.verdict.is_none());
        assert_eq!(rec.session_id.as_deref(), Some("sess-err"));
    }
}

/// Aggregation: client_upper_bound_hit_rate sums per-record LCP token upper
/// bounds only — records without LCP evidence contribute 0, never the full
/// request length (RFC-0015 §5.2).
#[test]
fn aggregate_uses_lcp_upper_bound_not_full_length() {
    let store = Arc::new(RingTraceStore::new());

    // Call 1: first request — no history → no LCP upper bound.
    let layer1 = audited(MockModel::new(token_body(1024, 512, 0), 0, 1536), &store);
    generate_in(&layer1, Some("sess-1"), opts()).unwrap();

    // Call 2: same-session prefix continuation → block-1 LCP upper bound
    // (4096 B → 1024 tokens).
    let layer2 = audited(
        MockModel::new(token_body(1024, 1536, 0), 1024, 2560),
        &store,
    );
    generate_in(&layer2, Some("sess-1"), opts()).unwrap();

    let stats = store.aggregate(&TraceFilter::default());
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
    let recs = records(&store);
    assert_eq!(recs[0].lcp_token_upper, None);
    assert_eq!(recs[1].lcp_token_upper, Some(2048));
}

/// R-2.2 cluster-routing exemption: a stable prefix with zero claimed hits is
/// the normal signature of a load-balanced cluster without sticky routing —
/// without route-affinity evidence it must NOT be flagged as a broken prefix.
#[test]
fn r22_suppressed_without_route_affinity_evidence() {
    let spec = spec_for("openai", "gpt-4o");
    let stats = |rounds: u32| SessionStats {
        same_session_rounds: rounds,
        prefix_stable: true,
        lcp_gt_1024: true,
    };

    // 4 rounds, stable prefix, claimed=0 — cluster signature.
    let mut inp = base_input(spec);
    inp.claimed = 0;
    inp.session_stats = Some(stats(4));
    inp = with_lcp(inp, 8192, true);
    let v = judge(&inp);
    assert_eq!(
        v.kind,
        VerdictKind::Trusted,
        "no route-affinity evidence → not a broken-prefix warning: {}",
        v.describe()
    );
    assert!(!v.violated.iter().any(|r| r == "R-2.2"), "{}", v.describe());
    assert!(
        v.notes
            .iter()
            .any(|n| n.contains("cluster routing artifact")),
        "suppression must be explained: {}",
        v.describe()
    );

    // With route affinity KNOWN (single node / sticky / global cache), the
    // same signature IS the broken-prefix diagnostic.
    let mut aff = base_input(spec);
    aff.claimed = 0;
    aff.route_affinity_known = true;
    aff.session_stats = Some(stats(4));
    aff = with_lcp(aff, 8192, true);
    let va = judge(&aff);
    assert_eq!(va.kind, VerdictKind::SuspectUnderclaim, "{}", va.describe());
    assert_eq!(va.confidence, VerdictConfidence::Low);
}

// ─────────────────────────────────────────────────────────────────────────────
// B1 / C4-2 / F1 regression tests
// ─────────────────────────────────────────────────────────────────────────────

/// B1: `scope_key` isolates by provider and model_id. Two TraceLayers over the SAME model_id but DIFFERENT providers
/// sharing one RingTraceStore must not cross-match LCP history. (Before the
/// fix `scope_key` hashed only the model_id, so beta would have matched
/// alpha's prefix.)
#[test]
fn scope_key_isolates_by_provider_same_model_id() {
    let store = Arc::new(RingTraceStore::new());
    let body = token_body(1024, 512, 0); // > 4 KiB → block-aligned prefix
    let model = |provider: &'static str| {
        MockModel::with_identity(provider, "shared-m", body.clone(), 0, 1536)
    };

    // Provider "alpha", model "shared-m": two identical calls. Call 2 must
    // match call 1 (same provider → same scope) → LCP upper bound present.
    generate_in(&audited(model("alpha"), &store), Some("sa"), opts()).unwrap();
    generate_in(&audited(model("alpha"), &store), Some("sa"), opts()).unwrap();

    // Provider "beta", SAME model "shared-m", SAME body — must NOT match
    // alpha's records (different provider → different scope_key).
    generate_in(&audited(model("beta"), &store), Some("sb"), opts()).unwrap();

    let stats = store.aggregate(&TraceFilter::default());
    // Two groups: (alpha, shared-m) and (beta, shared-m).
    let alpha = stats
        .iter()
        .find(|s| s.provider == "alpha")
        .expect("alpha group");
    let beta = stats
        .iter()
        .find(|s| s.provider == "beta")
        .expect("beta group");
    assert_eq!(alpha.requests, 2);
    assert_eq!(beta.requests, 1);
    // alpha call 2 matched call 1 → client upper bound present.
    assert!(
        alpha.client_upper_bound_hit_rate.is_some(),
        "same-provider continuation must produce an LCP upper bound"
    );
    // beta is isolated → no LCP evidence (would be Some if scope leaked).
    assert!(
        beta.client_upper_bound_hit_rate.is_none(),
        "different provider must NOT cross-match LCP history (B1)"
    );
}

/// C4-2 negative case: when an independent invariant (here R-1.6b quantization)
/// ALSO fires, the overclaim does NOT rely solely on the byte proxy, so it is
/// NOT downgraded to Unknown — it stays SuspectOverclaim (capped at Medium by
/// the byte-proxy F2 rule, but still an overclaim).
#[test]
fn c42_byte_proxy_overclaim_with_independent_evidence_is_not_unknown() {
    let spec = spec_for("openai", "gpt-4o"); // gran=128
    // u = 512 tokens (LCP upper 2048 B / 4). claimed=700 > 640 (u+τ) → R-1.1,
    // and 700 % 128 ≠ 0 → R-1.6b (independent). byte_proxy = true.
    let mut inp = base_input(spec);
    inp.claimed = 700;
    inp = with_lcp(inp, 1536, true);
    let v = judge(&inp);
    assert_eq!(
        v.kind,
        VerdictKind::SuspectOverclaim,
        "independent evidence (R-1.6b) keeps the overclaim: {}",
        v.describe()
    );
    assert!(v.violated.iter().any(|r| r == "R-1.6b"));
    assert!(v.violated.iter().any(|r| r == "R-1.1"));
    // Byte-proxy R-1.1 is capped at Medium (F2); the independent R-1.6b is B.
    assert_eq!(v.confidence, VerdictConfidence::Medium, "{}", v.describe());
}

/// F1: a stream dropped before completion (take(N) / abort / timeout) must
/// still record its trace — the Drop guard writes it with a non-success marker.
#[test]
fn trace_layer_stream_dropped_early_still_records() {
    let store = Arc::new(RingTraceStore::new());
    let model = Scripted {
        model: "mock-long",
        parts: five_chunks,
    };
    let layer = traced(model, &store);
    let mut result = stream_in(&layer, Some("sess-drop"), opts()).unwrap();
    // Consume only 2 parts, then drop the stream (no Finish seen).
    for _ in 0..2 {
        assert!(block_on(result.stream.next()).is_some());
    }
    assert!(
        store.is_empty(),
        "nothing is recorded while the stream is live"
    );
    drop(result); // early drop — must NOT lose the record (F1)

    let stats = store.aggregate(&TraceFilter::default());
    assert_eq!(
        stats[0].requests, 1,
        "early-dropped stream must still record its trace (F1)"
    );
    assert_eq!(
        records(&store)[0].error.as_deref(),
        Some("stream dropped before completion"),
        "dropped stream record must carry the non-success marker"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Call id, response fields and usage on the record
// ─────────────────────────────────────────────────────────────────────────────

/// The call id comes from the call's recording context, on the success,
/// failure and stream paths alike. A call without a context gets
/// a fresh `trace-` id.
#[test]
fn trace_layer_takes_the_call_id_from_the_recording_context() {
    let store = Arc::new(RingTraceStore::new());
    let recorder = Arc::new(RingRecorder::new());
    let ok = traced(MockModel::new(token_body(16, 16, 0), 0, 32), &store);
    let failing = traced(Failing, &store);

    let generate = |layer: &TraceLayer, context| {
        block_on(RecordingContext::scope(context, layer.do_generate(&opts())))
    };
    let stream = |layer: &TraceLayer, context| {
        block_on(RecordingContext::scope(context, layer.do_stream(&opts())))
    };
    generate(&ok, recorded("call-generate", &recorder)).unwrap();
    drain(stream(&ok, recorded("call-stream", &recorder)).unwrap());
    assert!(generate(&failing, recorded("call-failed", &recorder)).is_err());
    generate(&ok, None).unwrap();
    drain(stream(&ok, None).unwrap());

    let ids: Vec<String> = records(&store).into_iter().map(|rec| rec.call_id).collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(ids[..3], ["call-generate", "call-stream", "call-failed"]);
    let minted = &ids[3..];
    assert!(
        minted.iter().all(|id| id.starts_with("trace-")),
        "{minted:?}"
    );
    assert_ne!(minted[0], minted[1], "minted ids are unique");
}

/// The request body comes from `result.request`, the response id and headers
/// from `result.response` (`trace/layer.rs:537-549`, `:572-582`): the
/// fingerprint, the request id and the response cache headers (R-4.1) reach
/// the record on both paths.
#[test]
fn trace_layer_reads_body_id_and_headers_from_the_result() {
    let store = Arc::new(RingTraceStore::new());
    let headers = HashMap::from([("x-openrouter-cache-status".to_string(), "HIT".to_string())]);
    let model = MockModel {
        headers: Some(headers.clone()),
        ..MockModel::new(token_body(1024, 512, 0), 0, 1536)
    };
    let layer = audited(model, &store);
    generate_in(&layer, None, opts()).unwrap();
    drain(stream_in(&layer, None, opts()).unwrap());

    let recs = records(&store);
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].request_id.as_deref(), Some("req-1"));
    assert!(recs[0].fingerprint.len_bytes > 0, "request body read");
    assert_eq!(
        recs[0].fingerprint.body_hash, recs[1].fingerprint.body_hash,
        "the stream path reads the same request body"
    );
    for rec in &recs {
        assert_eq!(rec.response_cache_headers.as_ref(), Some(&headers));
        let verdict = rec.verdict.as_ref().expect("rules auditor attached");
        assert_eq!(verdict.kind, VerdictKind::Unknown, "{}", verdict.describe());
        assert!(
            verdict.violated.iter().any(|rule| rule == "R-4.1"),
            "{}",
            verdict.describe()
        );
    }
}

/// A generated file is model output for TTFT (`trace/layer.rs:595-596`);
/// stream metadata alone is not.
#[test]
fn trace_layer_counts_a_file_part_as_first_output() {
    let store = Arc::new(RingTraceStore::new());
    let files = Scripted {
        model: "mock-file",
        parts: file_then_finish,
    };
    let silent = Scripted {
        model: "mock-silent",
        parts: no_output,
    };
    drain(stream_in(&traced(files, &store), None, opts()).unwrap());
    drain(stream_in(&traced(silent, &store), None, opts()).unwrap());

    let recs = records(&store);
    assert!(
        recs[0].ttft_ms.is_some(),
        "a file part is output: {:?}",
        recs[0]
    );
    assert_eq!(recs[1].ttft_ms, None, "StreamStart / Finish are not output");
    assert!(recs.iter().all(|rec| rec.error.is_none()));
}

/// `Usage.raw` is a JSON object; the record keeps it as given.
#[test]
fn trace_layer_keeps_the_raw_usage_object() {
    let store = Arc::new(RingTraceStore::new());
    let raw = json!({ "prompt_cache_hit_tokens": 1024, "prompt_cache_miss_tokens": 512 });
    let mut model = MockModel::new(token_body(1024, 512, 0), 1024, 1536);
    model.usage.raw = raw.as_object().cloned();
    generate_in(&traced(model, &store), None, opts()).unwrap();
    assert_eq!(records(&store)[0].usage.raw, Some(raw));
}
