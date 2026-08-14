//! `TraceLayer` — the probe decorator (RFC-0015 §2/§5.3).
//!
//! Wrap `Arc<dyn LanguageModel>`; generation and streaming are automatically
//! probed: fingerprint (denoised request body) → LCP lookup → judge → sink.
//! No trait changes, no provider changes. The auditor is pluggable and off by
//! default (verdict = `None`, plain passthrough).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Process-unique trace id sequence (trace ids are `trace-{pid}-{mono}-{seq}`).
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Process-wide monotonic clock origin — every `TraceLayer` shares it, so
/// TTL comparisons stay in one clock domain even when multiple layers write
/// to the same `RingTraceStore`.
fn monotonic_origin() -> &'static Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now)
}

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};
use crate::stream_part::StreamPart;
use crate::types::Usage;

use super::fingerprint::{BlockChainFingerprint, Chain, Fingerprint, denoise};
use super::hash;
use super::record::{TraceRecord, UsageSnapshot};
use super::store::{LcpResult, RingTraceStore, TraceSink};
use super::verdict::{
    JudgmentInput, LcpInput, SessionStats, Verdict, judge as rule_judge, matrix as verdict_matrix,
};

/// The audit engine (pluggable; `RuleAuditor` is the built-in rules engine).
pub trait CacheAuditor: Send + Sync {
    fn judge(&self, input: &JudgmentInput) -> Verdict;
}

/// Built-in rules engine (RFC-0015 §4.2). Stateless — the strict/shared mode
/// comes from `JudgmentInput.strict`, set by the layer.
pub struct RuleAuditor;

impl CacheAuditor for RuleAuditor {
    fn judge(&self, input: &JudgmentInput) -> Verdict {
        rule_judge(input)
    }
}

/// Per-session chain bookkeeping (R-2.2 low-hit warning).
#[derive(Default)]
struct SessionTracker {
    rounds: u32,
    prev_body_len: u64,
}

/// The probe decorator. Wrap a model; every call is fingerprinted and
/// recorded to the sink.
pub struct TraceLayer {
    inner: Arc<dyn LanguageModel>,
    sink: Arc<dyn TraceSink>,
    auditor: Option<Arc<dyn CacheAuditor>>,
    /// strict mode (RFC-0015 §4.3); default shared (safe for multi-process
    /// / shared API keys).
    strict: bool,
    /// Default session id when `CallOptions.session_id` is absent.
    default_session: Option<String>,
    /// Scope salt (HMAC-style key for fingerprint scoping).
    scope_salt: u64,
    session_tracker: Arc<Mutex<HashMap<String, SessionTracker>>>,
}

/// Request identity computed before dispatch.
struct RequestCtx {
    session_id: Option<String>,
    scope_key: u64,
    sent_at_unix_ms: i64,
    /// Monotonic clock ms — same clock domain as the store's TTL lookups.
    monotonic_ms: u64,
}

/// Clone of the layer's recording state — lives inside the stream closure
/// (the layer itself is borrowed only for the call duration).
struct RecordCtx {
    provider: String,
    model: String,
    sink: Arc<dyn TraceSink>,
    auditor: Option<Arc<dyn CacheAuditor>>,
    strict: bool,
    scope_salt: u64,
    session_tracker: Arc<Mutex<HashMap<String, SessionTracker>>>,
    ctx: RequestCtx,
    /// RFC-0023 关联 ID:复用 `options.call_id`(缺失时由构造处生成)。
    call_id: Option<String>,
    request_body: Option<serde_json::Value>,
    response_headers: Option<HashMap<String, String>>,
    ttft_ms: Arc<AtomicU64>,
}

impl TraceLayer {
    /// Wrap a model with a sink (records every call).
    pub fn new(inner: Arc<dyn LanguageModel>, sink: Arc<dyn TraceSink>) -> Self {
        Self {
            inner,
            sink,
            auditor: None,
            strict: false,
            default_session: None,
            scope_salt: hash::mix(0x5eed_5eed ^ std::process::id() as u64),
            session_tracker: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Default session id (used when `CallOptions.session_id` is absent) +
    /// session key (32-byte HMAC-style salt for fingerprint scoping).
    pub fn with_default_session(mut self, session_id: String, key: [u8; 32]) -> Self {
        self.default_session = Some(session_id);
        let mut acc = 0x9e37_79b9_7f4a_7c15u64;
        for chunk in key.chunks(8) {
            let mut w = 0u64;
            for (i, b) in chunk.iter().enumerate() {
                w ^= (*b as u64) << (8 * i);
            }
            acc = hash::mix(acc ^ w);
        }
        self.scope_salt = acc;
        self
    }

    /// Attach an auditor (default: none — verdict stays `None`).
    pub fn with_auditor(mut self, auditor: Arc<dyn CacheAuditor>) -> Self {
        self.auditor = Some(auditor);
        self
    }

    /// Convenience: attach the built-in rules auditor in strict/shared mode.
    pub fn with_rules_auditor(mut self, strict: bool) -> Self {
        self.strict = strict;
        self.auditor = Some(Arc::new(RuleAuditor));
        self
    }

    fn make_record_ctx(
        &self,
        options: &CallOptions,
        request_body: Option<serde_json::Value>,
        response_headers: Option<HashMap<String, String>>,
        ttft_ms: Arc<AtomicU64>,
    ) -> RecordCtx {
        let session_id = options
            .session_id
            .clone()
            .or_else(|| self.default_session.clone());
        // B1: scope_key isolates by (provider, base_url, model_id) — not just
        // model_id — so two TraceLayers over the same model_id but different
        // providers (or base_urls) sharing one RingTraceStore never cross-match
        // their LCP history. `base_url` comes from the inner model's config
        // snapshot; providers that only report a minimal snapshot omit it, in
        // which case provider + model_id still isolate the scope. The default
        // `scope_salt` stays process-level (set in `new` / `with_default_session`).
        let provider = self.inner.provider();
        let model_id = self.inner.model_id();
        let base_url = self.inner.config_snapshot().base_url;
        let mut scope_bytes = Vec::with_capacity(
            provider.len() + model_id.len() + base_url.as_deref().map_or(0, str::len),
        );
        scope_bytes.extend_from_slice(provider.as_bytes());
        scope_bytes.extend_from_slice(model_id.as_bytes());
        if let Some(b) = &base_url {
            scope_bytes.extend_from_slice(b.as_bytes());
        }
        let scope_key = hash::hash64(self.scope_salt, &scope_bytes);
        let sent_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let monotonic_ms = monotonic_origin().elapsed().as_millis() as u64;
        RecordCtx {
            provider: provider.to_string(),
            model: model_id.to_string(),
            sink: self.sink.clone(),
            auditor: self.auditor.clone(),
            strict: self.strict,
            scope_salt: self.scope_salt,
            session_tracker: self.session_tracker.clone(),
            call_id: options.call_id.clone(),
            ctx: RequestCtx {
                session_id,
                scope_key,
                sent_at_unix_ms,
                monotonic_ms,
            },
            request_body,
            response_headers,
            ttft_ms,
        }
    }
}

impl RecordCtx {
    /// Denoised bytes for the request body (fingerprint + LCP chain share
    /// the same bytes).
    fn denoised_bytes(&self) -> Option<Vec<u8>> {
        let body = self.request_body.as_ref()?;
        serde_json::to_vec(&denoise(body)).ok()
    }

    fn usage_snapshot(usage: &Usage) -> UsageSnapshot {
        UsageSnapshot {
            input_total: usage.input_tokens.total.map(u64::from),
            input_no_cache: usage.input_tokens.no_cache.map(u64::from),
            cache_read: usage.input_tokens.cache_read.map(u64::from),
            cache_write: usage.input_tokens.cache_write.map(u64::from),
            output_total: usage.output_tokens.total.map(u64::from),
            output_text: usage.output_tokens.text.map(u64::from),
            output_reasoning: usage.output_tokens.reasoning.map(u64::from),
            raw: usage.raw.clone(),
        }
    }

    /// Per-session chain stats for R-2.2 (rounds + prefix stability).
    fn session_stats_for(&self, sid: &str, lcp_bytes: u64, body_len: u64) -> SessionStats {
        let mut map = self.session_tracker.lock().unwrap();
        let t = map.entry(sid.to_string()).or_default();
        let rounds = t.rounds + 1;
        let stable = t.prev_body_len > 0 && lcp_bytes * 100 >= t.prev_body_len * 95;
        t.rounds = rounds;
        t.prev_body_len = body_len;
        SessionStats {
            same_session_rounds: rounds,
            prefix_stable: stable,
            lcp_gt_1024: lcp_bytes / 4 > 1024,
        }
    }

    fn ring_store(&self) -> Option<&RingTraceStore> {
        self.sink.as_any().downcast_ref::<RingTraceStore>()
    }

    /// LCP lookup against the sink's history (no-op for non-ring sinks).
    fn lookup_lcp(&self, chain: &Chain, ttl_ms: u64) -> (Option<LcpInput>, u64, u64, bool) {
        let Some(store) = self.ring_store() else {
            return (None, 0, 0, false);
        };
        let now_mono = monotonic_origin().elapsed().as_millis() as u64;
        let lcp: LcpResult = store.lookup(self.ctx.scope_key, chain, now_mono, ttl_ms);
        let same_session = lcp
            .matched
            .as_ref()
            .map(|m| m.session.as_deref() == self.ctx.session_id.as_deref())
            .unwrap_or(false);
        let upper = if lcp.matched.is_some() {
            // RFC F3: audit ceiling is the block UPPER bound (j+1)·B.
            (lcp.matched_blocks as u64 + 1) * chain.block_size as u64
        } else {
            0
        };
        let input = lcp.matched.map(|_| LcpInput {
            lcp_bytes: lcp.lcp_bytes,
            lcp_upper_bytes: upper,
            same_session,
            matched_exists: true,
        });
        (input, lcp.lcp_bytes, upper, lcp.candidate_expired)
    }

    /// Build and record the trace for a completed call.
    fn record(&self, usage: &Usage, error: Option<String>, request_id: Option<String>) {
        let spec = verdict_matrix::for_provider(&self.provider, &self.model);
        let fp =
            BlockChainFingerprint::new(super::fingerprint::DEFAULT_BLOCK_SIZE, self.scope_salt);
        let (fingerprint, chain) = match self.denoised_bytes() {
            Some(bytes) => {
                let c = fp.compute(&bytes);
                (Fingerprint::from(&c), c)
            }
            None => (
                Fingerprint {
                    body_hash: String::new(),
                    len_bytes: 0,
                    block_size: 0,
                    block_hashes: Vec::new(),
                    token_estimate: 0,
                },
                Chain {
                    body_hash: 0,
                    len_bytes: 0,
                    block_size: super::fingerprint::DEFAULT_BLOCK_SIZE,
                    block_hashes: Vec::new(),
                },
            ),
        };

        let (lcp_input, lcp_bytes, lcp_upper_bytes, candidate_expired) =
            if error.is_none() && !chain.block_hashes.is_empty() {
                let (li, lb, lu, ce) = self.lookup_lcp(&chain, spec.ttl_ms);
                (li, lb, lu, ce)
            } else {
                (None, 0, 0, false)
            };

        let sid = self
            .ctx
            .session_id
            .clone()
            .unwrap_or_else(|| format!("<scope:{}>", self.ctx.scope_key));
        let session_stats = self.session_stats_for(&sid, lcp_bytes, fingerprint.len_bytes);

        let verdict = if let Some(auditor) = &self.auditor {
            let usage_present = usage.input_tokens.total.is_some();
            let prompt_tokens = usage.input_tokens.total.unwrap_or(0) as u64;
            let claimed = usage.input_tokens.cache_read.unwrap_or(0) as u64;
            let inp = JudgmentInput {
                spec,
                strict: self.strict,
                input_no_cache: usage.input_tokens.no_cache.map(u64::from),
                input_cache_read: usage.input_tokens.cache_read.map(u64::from),
                input_cache_write: usage.input_tokens.cache_write.map(u64::from),
                first: self
                    .ring_store()
                    .map(|s| s.records_in_scope(self.ctx.scope_key) == 0)
                    .unwrap_or(true),
                prompt_tokens,
                prompt_bytes: fingerprint.len_bytes,
                claimed,
                write: usage.input_tokens.cache_write.map(u64::from),
                no_cache: usage.input_tokens.no_cache.map(u64::from),
                hit: usage
                    .raw
                    .as_ref()
                    .and_then(|r| r.get("prompt_cache_hit_tokens"))
                    .and_then(|v| v.as_u64()),
                miss: usage
                    .raw
                    .as_ref()
                    .and_then(|r| r.get("prompt_cache_miss_tokens"))
                    .and_then(|v| v.as_u64()),
                usage_present,
                response_cache_header_hit: self
                    .response_headers
                    .as_ref()
                    .map(|h| {
                        h.iter().any(|(k, v)| {
                            k.to_ascii_lowercase().contains("cache-status")
                                && v.to_ascii_uppercase().contains("HIT")
                        })
                    })
                    .unwrap_or(false),
                candidate_expired,
                byte_proxy: true, // no tokenizer attached — byte proxy len/4
                // The layer cannot know the deployment topology; route
                // affinity is only guaranteed when the consumer opts in
                // (single node / sticky / global cache).
                route_affinity_known: false,
                lcp: lcp_input,
                system_tokens: 0, // semantic segments are not visible here
                session_stats: Some(session_stats),
            };
            Some(auditor.judge(&inp))
        } else {
            None
        };

        let ttft_ms = self.ttft_ms.load(Ordering::Relaxed);
        let ttft_ms = if ttft_ms == u64::MAX {
            None
        } else {
            Some(ttft_ms)
        };

        // Client-side LCP token upper bound (for aggregation; None when no
        // history matched). Derived from the block upper bound.
        let lcp_token_upper = if lcp_upper_bytes > 0 {
            Some(
                (lcp_upper_bytes.min(fingerprint.len_bytes) / 4)
                    .min(usage.input_tokens.total.unwrap_or(0) as u64),
            )
        } else {
            None
        };

        let rec = TraceRecord {
            provider: self.provider.clone(),
            model: self.model.clone(),
            request_id,
            session_id: self.ctx.session_id.clone(),
            call_id: self.call_id.clone().unwrap_or_else(|| {
                format!(
                    "trace-{}-{}-{}",
                    std::process::id(),
                    self.ctx.monotonic_ms,
                    TRACE_SEQ.fetch_add(1, Ordering::Relaxed)
                )
            }),
            sent_at_unix_ms: self.ctx.sent_at_unix_ms,
            monotonic_sent_ms: self.ctx.monotonic_ms,
            lcp_token_upper,
            ttft_ms,
            fingerprint,
            usage: Self::usage_snapshot(usage),
            response_cache_headers: self.response_headers.clone(),
            request_cache_hints: None,
            verdict,
            error,
            scope_key: self.ctx.scope_key,
        };
        self.sink.record(rec);
    }
}

/// Drop-guard stream wrapper (F1): ensures the trace is recorded even when the
/// caller drops the stream before completion (`take(N)` / abort / timeout).
/// Mirrors `RecordingOutcomeStream`'s Drop pattern (recording.rs): the previous
/// generator-based record only fired when the stream ran to completion, so
/// early drops silently lost the call. This wrapper records on natural EOF, on
/// a terminal part (`Finish` / `Error` / transport `Err`), or — via `Drop` —
/// when the caller abandons the stream mid-flight.
struct TraceRecordingStream<S> {
    inner: S,
    rec_ctx: Option<Arc<RecordCtx>>,
    /// Accumulated usage (`Finish`) / error (`Error` / `Err`) observed so far.
    usage: Usage,
    error: Option<String>,
    /// Whether the terminal record has been emitted (idempotent guard).
    recorded: bool,
}

impl<S> TraceRecordingStream<S> {
    /// Emit the trace record exactly once (idempotent across EOF / terminal /
    /// Drop). `rec_ctx` is consumed so a later `Drop` is a no-op.
    fn record_now(&mut self, usage: Usage, error: Option<String>) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        if let Some(ctx) = self.rec_ctx.take() {
            ctx.record(&usage, error, None);
        }
    }
}

impl<S> Stream for TraceRecordingStream<S>
where
    S: Stream<Item = Result<StreamPart, AiMuxError>> + Unpin,
{
    type Item = Result<StreamPart, AiMuxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(part))) => {
                // Accumulate terminal evidence (mirrors the original
                // end-of-stream record).
                match &part {
                    StreamPart::Finish { usage: u, .. } => this.usage = u.clone(),
                    StreamPart::Error { error: e } => {
                        this.error.get_or_insert_with(|| e.to_string());
                    }
                    _ => {}
                }
                Poll::Ready(Some(Ok(part)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.error.get_or_insert_with(|| e.to_string());
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                // Natural end: record accumulated state (matches the original).
                let usage = std::mem::take(&mut this.usage);
                let error = this.error.take();
                this.record_now(usage, error);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for TraceRecordingStream<S> {
    fn drop(&mut self) {
        if !self.recorded {
            // Caller dropped before completion — record what was observed with
            // a non-success marker so the call is not silently lost (F1).
            let usage = std::mem::take(&mut self.usage);
            let error = self
                .error
                .take()
                .or_else(|| Some("stream dropped before completion".into()));
            self.record_now(usage, error);
        }
    }
}

#[async_trait]
impl LanguageModel for TraceLayer {
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    fn provider(&self) -> &str {
        self.inner.provider()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    /// RFC-0023 §3.3: transparent decorators must forward the inner snapshot
    /// (otherwise recording sees the decorator's minimal record).
    fn config_snapshot(&self) -> crate::recording::ProviderRecord {
        self.inner.config_snapshot()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let ctx = self.make_record_ctx(options, None, None, Arc::new(AtomicU64::new(u64::MAX)));
        match self.inner.do_generate(options).await {
            Ok(result) => {
                let request_id = result.response.id.clone();
                let mut rec_ctx = ctx;
                rec_ctx.request_body = result.request_body.clone();
                rec_ctx.response_headers = result.response_headers.clone();
                rec_ctx.record(&result.usage, None, request_id);
                Ok(result)
            }
            Err(e) => {
                // Failures are part of the session too.
                ctx.record(&Usage::default(), Some(e.to_string()), None);
                Err(e)
            }
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let ttft = Arc::new(AtomicU64::new(u64::MAX));
        let ctx = self.make_record_ctx(options, None, None, ttft.clone());
        let result = match self.inner.do_stream(options).await {
            Ok(r) => r,
            Err(e) => {
                ctx.record(&Usage::default(), Some(e.to_string()), None);
                return Err(e);
            }
        };

        let rec_ctx = Arc::new(RecordCtx {
            request_body: result.request_body.clone(),
            response_headers: result.response_headers.clone(),
            ..ctx
        });
        let ttft_obs = ttft.clone();
        let started = Instant::now();

        // Wrap the stream: observe TTFT on the first MODEL-OUTPUT part
        // (TextDelta / Reasoning / ToolCall — not StreamStart or meta).
        let observed = result.stream.map(move |item| {
            if ttft_obs.load(Ordering::Relaxed) == u64::MAX {
                let is_model_output = matches!(
                    &item,
                    Ok(StreamPart::TextDelta { .. })
                        | Ok(StreamPart::ReasoningDelta { .. })
                        | Ok(StreamPart::ToolCall { .. })
                );
                if is_model_output {
                    ttft_obs.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                }
            }
            item
        });

        // F1: wrap with a Drop-guard stream so the trace is recorded even when
        // the caller drops the stream before completion (take(N) / abort /
        // timeout). The generator-only record previously lost such calls.
        let guarded = TraceRecordingStream {
            inner: observed,
            rec_ctx: Some(rec_ctx),
            usage: Usage::default(),
            error: None,
            recorded: false,
        };

        Ok(StreamResult {
            stream: Box::pin(guarded),
            request_body: result.request_body,
            response_headers: result.response_headers,
        })
    }
}
