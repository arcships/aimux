//! Memory ablation benchmark suite for `aimux-core` + `aimux-provider-utils`.
//!
//! Measures exact heap behavior using a counting `#[global_allocator]`:
//! - `peak_bytes`: high-water mark of live heap above baseline during the workload
//! - `retained_bytes`: live heap retained at the end of the workload (before dropping stores)
//! - `total_alloc_bytes`: cumulative bytes allocated across the workload
//! - `alloc_count`: cumulative number of heap allocations

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::generate::{GenerateTextOptions, stream_text};
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelMessage;
use aimux_core::operation::OperationSettings;
use aimux_core::options::CallOptions;
use aimux_core::recording::{
    HttpExchange, HttpRecord, JsonlRecorder, OutcomeRecord, Recorder, RecordingContext,
    ResponseRecord, RingRecorder, TimingRecord,
};
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::shared::{RequestInfo, StreamResponseInfo};
use aimux_core::stream_part::StreamPart;
use aimux_core::trace::{RingTraceStore, TraceLayer, TraceSink};
use aimux_core::types::{FinishReason, FinishReasonUnified, LanguageModelUsage, Usage};
use aimux_provider_utils::{
    Fetch, FetchError, FetchRequest, FetchResponse, HttpRequest,
    create_event_source_response_handler, create_json_response_handler,
    create_status_code_error_response_handler, post_json_to_api,
};

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static TOTAL_ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
static TOTAL_ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let size = layout.size();
            let prev = LIVE_BYTES.fetch_add(size, Ordering::Relaxed);
            PEAK_BYTES.fetch_max(prev + size, Ordering::Relaxed);
            TOTAL_ALLOC_BYTES.fetch_add(size, Ordering::Relaxed);
            TOTAL_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let old_size = layout.size();
            if new_size >= old_size {
                let diff = new_size - old_size;
                let prev = LIVE_BYTES.fetch_add(diff, Ordering::Relaxed);
                PEAK_BYTES.fetch_max(prev + diff, Ordering::Relaxed);
                TOTAL_ALLOC_BYTES.fetch_add(diff, Ordering::Relaxed);
            } else {
                LIVE_BYTES.fetch_sub(old_size - new_size, Ordering::Relaxed);
            }
            TOTAL_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Debug, Clone, Copy)]
struct MemStats {
    peak_bytes: usize,
    retained_bytes: usize,
    total_alloc_bytes: usize,
    alloc_count: usize,
}

struct MeasureGuard {
    base_live: usize,
}

impl MeasureGuard {
    fn start() -> Self {
        let base_live = LIVE_BYTES.load(Ordering::SeqCst);
        PEAK_BYTES.store(base_live, Ordering::SeqCst);
        TOTAL_ALLOC_BYTES.store(0, Ordering::SeqCst);
        TOTAL_ALLOC_COUNT.store(0, Ordering::SeqCst);
        Self { base_live }
    }

    fn snapshot(&self) -> MemStats {
        let live = LIVE_BYTES.load(Ordering::SeqCst);
        let peak = PEAK_BYTES.load(Ordering::SeqCst);
        MemStats {
            peak_bytes: peak.saturating_sub(self.base_live),
            retained_bytes: live.saturating_sub(self.base_live),
            total_alloc_bytes: TOTAL_ALLOC_BYTES.load(Ordering::SeqCst),
            alloc_count: TOTAL_ALLOC_COUNT.load(Ordering::SeqCst),
        }
    }
}

fn build_large_request_body(num_messages: usize, content_bytes: usize, seed: usize) -> Value {
    let text_block = "x".repeat(content_bytes);
    let messages: Vec<Value> = (0..num_messages)
        .map(|i| {
            json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": [
                    {
                        "type": "text",
                        "text": format!("msg-{seed}-{i}: {text_block}")
                    }
                ]
            })
        })
        .collect();
    json!({
        "model": "gpt-4o-2024-11-20",
        "messages": messages,
        "temperature": 0.7,
        "stream": true,
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "search_codebase",
                    "description": "Search the repository for symbols and references",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" },
                            "limit": { "type": "integer" }
                        }
                    }
                }
            }
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// W1: Core `stream_text` Streaming & Backpressure
// ─────────────────────────────────────────────────────────────────────────────

struct MockStreamingModel {
    request_body: Value,
    chunk_count: usize,
    chunk_size: usize,
}

#[async_trait]
impl LanguageModel for MockStreamingModel {
    fn provider(&self) -> &str {
        "openai.chat"
    }

    fn model_id(&self) -> &str {
        "gpt-4o"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!()
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let chunk_str = "a".repeat(self.chunk_size);
        let count = self.chunk_count;
        let stream = async_stream::stream! {
            yield Ok(StreamPart::TextStart {
                id: "txt-1".to_string(),
                provider_metadata: None,
            });
            for _ in 0..count {
                yield Ok(StreamPart::TextDelta {
                    id: "txt-1".to_string(),
                    delta: chunk_str.clone(),
                    provider_metadata: None,
                });
            }
            yield Ok(StreamPart::TextEnd {
                id: "txt-1".to_string(),
                provider_metadata: None,
            });
            yield Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: Some("stop".to_string()),
                },
                usage: Usage::default(),
                provider_metadata: None,
            });
        };
        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo {
                body: Some(self.request_body.clone()),
            }),
            response: Some(StreamResponseInfo {
                headers: Some(HashMap::new()),
            }),
        })
    }
}

async fn run_w1a_stream_text_hot_path() -> MemStats {
    let request_body = build_large_request_body(40, 3000, 1); // ~125 KB JSON AST
    let model = MockStreamingModel {
        request_body,
        chunk_count: 2000,
        chunk_size: 64, // 2000 * 64B = 128 KB streamed text
    };
    let g = MeasureGuard::start();
    for _ in 0..5 {
        let mut res = stream_text(&model, "hello", GenerateTextOptions::default())
            .await
            .unwrap();
        while let Some(part) = res.stream.next().await {
            std::hint::black_box(part.unwrap());
        }
    }
    g.snapshot()
}

async fn run_w1b_stream_text_backpressure() -> MemStats {
    let model = MockStreamingModel {
        request_body: json!({"model": "gpt-4o"}),
        chunk_count: 2000,
        chunk_size: 4096, // 2000 * 4 KB = 8 MB producer burst
    };
    let g = MeasureGuard::start();
    let mut res = stream_text(&model, "hello", GenerateTextOptions::default())
        .await
        .unwrap();
    // Poll once to start stream, then pause consumer for 30ms while producer pumps
    let first = res.stream.next().await;
    std::hint::black_box(first);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mid = g.snapshot();
    // Drain the rest
    while let Some(part) = res.stream.next().await {
        std::hint::black_box(part.unwrap());
    }
    let end = g.snapshot();
    MemStats {
        peak_bytes: end.peak_bytes,
        retained_bytes: mid.retained_bytes, // mid-stream buffered bytes under slow consumer
        total_alloc_bytes: end.total_alloc_bytes,
        alloc_count: end.alloc_count,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// W2: `aimux-provider-utils` HTTP + SSE Transport & Request Body Hot Path
// ─────────────────────────────────────────────────────────────────────────────

struct InMemoryFetch {
    sse_payload: Bytes,
    json_payload: Bytes,
}

#[async_trait]
impl Fetch for InMemoryFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let is_stream = request
            .headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        let mut headers = http::HeaderMap::new();
        if is_stream {
            headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/event-stream"),
            );
            let payload = self.sse_payload.clone();
            let body = futures::stream::iter(vec![Ok(payload)]).boxed();
            Ok(FetchResponse {
                status: http::StatusCode::OK,
                headers,
                url: request.url,
                body,
            })
        } else {
            headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
            let payload = self.json_payload.clone();
            let body = futures::stream::iter(vec![Ok(payload)]).boxed();
            Ok(FetchResponse {
                status: http::StatusCode::OK,
                headers,
                url: request.url,
                body,
            })
        }
    }
}

async fn run_w2_provider_utils_hot_path() -> MemStats {
    let mut sse_buf = String::new();
    for i in 0..100 {
        sse_buf.push_str(&format!(
            "data: {{\"id\":\"chatcmpl-{i}\",\"choices\":[{{\"delta\":{{\"content\":\"chunk-{i}\"}}}}]}}\n\n"
        ));
    }
    sse_buf.push_str("data: [DONE]\n\n");
    let fetch: Arc<dyn Fetch> = Arc::new(InMemoryFetch {
        sse_payload: Bytes::from(sse_buf),
        json_payload: Bytes::from_static(
            br#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
        ),
    });
    let template_body = build_large_request_body(40, 3500, 2); // ~145 KB JSON AST

    let g = MeasureGuard::start();
    let mut active_stream_retained = 0usize;

    for i in 0..10 {
        let req = HttpRequest {
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            headers: vec![("accept".to_string(), "application/json".to_string())],
            fetch: Some(fetch.clone()),
            ..Default::default()
        };
        let out = post_json_to_api(
            req,
            template_body.clone(),
            create_json_response_handler::<Value>(),
            create_status_code_error_response_handler(),
        )
        .await
        .unwrap();
        std::hint::black_box(out.value);

        let stream_req = HttpRequest {
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            headers: vec![("accept".to_string(), "text/event-stream".to_string())],
            fetch: Some(fetch.clone()),
            ..Default::default()
        };
        let mut stream_out = post_json_to_api(
            stream_req,
            template_body.clone(),
            create_event_source_response_handler::<Value>(),
            create_status_code_error_response_handler(),
        )
        .await
        .unwrap();
        if i == 0 {
            active_stream_retained = g.snapshot().retained_bytes;
        }
        while let Some(ev) = stream_out.value.next().await {
            std::hint::black_box(ev.unwrap());
        }
    }
    let end = g.snapshot();
    MemStats {
        peak_bytes: end.peak_bytes,
        retained_bytes: active_stream_retained,
        total_alloc_bytes: end.total_alloc_bytes,
        alloc_count: end.alloc_count,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// W3: `TraceLayer` + `RingTraceStore` Long-Running Multi-Session Workload
// ─────────────────────────────────────────────────────────────────────────────

struct TraceMockModel {
    bodies: Vec<Value>,
    idx: AtomicUsize,
}

#[async_trait]
impl LanguageModel for TraceMockModel {
    fn provider(&self) -> &str {
        "openai.chat"
    }

    fn model_id(&self) -> &str {
        "gpt-4o"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!()
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let i = self.idx.fetch_add(1, Ordering::Relaxed);
        let body = self.bodies[i % self.bodies.len()].clone();
        let stream = futures::stream::iter(vec![
            Ok(StreamPart::TextDelta {
                id: "1".to_string(),
                delta: "ok".to_string(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ]);
        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo { body: Some(body) }),
            response: None,
        })
    }
}

async fn run_w3_trace_store_and_layer() -> MemStats {
    // Pre-generate 64 distinct ~64 KB bodies (16 blocks of 4 KiB each) outside measurement
    let bodies: Vec<Value> = (0..64)
        .map(|i| build_large_request_body(16, 3800, i * 1000 + 7))
        .collect();
    let mock = Arc::new(TraceMockModel {
        bodies,
        idx: AtomicUsize::new(0),
    });

    let g = MeasureGuard::start();
    let store = Arc::new(RingTraceStore::with_capacity(128, 64));
    let sink: Arc<dyn TraceSink> = store.clone();
    let layer = TraceLayer::new(mock, sink).with_rules_auditor(false);

    // Run 800 streaming calls across 800 sessions (ring capacity is 128)
    for i in 0..800 {
        let settings = OperationSettings {
            session_id: Some(format!("session-{i:05}")),
            ..Default::default()
        };
        settings
            .scope(async {
                let mut res = layer.do_stream(&CallOptions::default()).await.unwrap();
                while let Some(item) = res.stream.next().await {
                    std::hint::black_box(item.unwrap());
                }
            })
            .await;
    }

    let stats = g.snapshot();
    drop(layer);
    drop(store);
    stats
}

// ─────────────────────────────────────────────────────────────────────────────
// W4: `RingRecorder` & `JsonlRecorder` Under Incomplete / Stalled Calls
// ─────────────────────────────────────────────────────────────────────────────

fn run_w4_recording_retention() -> MemStats {
    let mut call_opts = CallOptions::default();
    call_opts.prompt = vec![LanguageModelMessage::System {
        content: "x".repeat(8192),
        provider_options: None,
    }];
    let exchange = HttpExchange {
        step: Some("0".to_string()),
        attempt: 1,
        exchange_index: 1,
        request: HttpRecord {
            method: "POST".to_string(),
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            headers: vec![],
            body: Some("y".repeat(8192)),
        },
        response: Some(ResponseRecord {
            status: 200,
            headers: vec![],
            body: None,
            stream_chunks: Some(4),
            ttfb_ms: Some(10),
        }),
        timing: TimingRecord {
            latency_ms: 20,
            ttfb_ms: Some(10),
        },
        error: None,
        finalized: true,
    };
    let ok_outcome =
        OutcomeRecord::success(FinishReasonUnified::Stop, &LanguageModelUsage::default());
    let temp_dir =
        std::env::temp_dir().join(format!("aimux-mem-ablation-jsonl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);

    let g = MeasureGuard::start();
    let ring = RingRecorder::with_capacity(64); // pending_max = 128
    // 1 stalled call at the very front of pending_order
    ring.record_input("call-stalled-0", &call_opts, "openai", "gpt-4o");
    // 2,000 normal calls that also trigger inconsistent tracking and then finalize
    for i in 1..=2000 {
        let id = format!("call-ring-{i:05}");
        ring.record_input(&id, &call_opts, "openai", "gpt-4o");
        ring.record_exchange(&id, &exchange);
        ring.record_exchange(&id, &exchange); // duplicate attempt marks inconsistent
        ring.record_transport_closed(&id);
        ring.record_outcome(&id, &ok_outcome);
    }

    let jsonl = JsonlRecorder::new(&temp_dir);
    // 600 incomplete calls (e.g. lost Outcome event or abandoned stream)
    for i in 1..=600 {
        let id = format!("call-jsonl-incomplete-{i:05}");
        jsonl.record_input(&id, &call_opts, "openai", "gpt-4o");
        jsonl.record_exchange(&id, &exchange);
        jsonl.record_transport_closed(&id);
        // Outcome never arrives!
    }
    jsonl.flush();

    let stats = g.snapshot();
    assert_eq!(
        ring.inconsistent_call_ids().len(),
        64,
        "RingRecorder must clean up inconsistent call IDs when completed records are evicted from the ring"
    );
    drop(jsonl);
    drop(ring);
    let _ = std::fs::remove_dir_all(&temp_dir);
    stats
}

// ─────────────────────────────────────────────────────────────────────────────
// W_ALL: Combined End-to-End Pipeline (`stream_text` + `TraceLayer` + `RingRecorder` + SSE)
// ─────────────────────────────────────────────────────────────────────────────

struct EndToEndModel {
    fetch: Arc<dyn Fetch>,
    request_body: Value,
}

#[async_trait]
impl LanguageModel for EndToEndModel {
    fn provider(&self) -> &str {
        "openai.chat"
    }

    fn model_id(&self) -> &str {
        "gpt-4o"
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        unimplemented!()
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let req = HttpRequest {
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            headers: vec![("accept".to_string(), "text/event-stream".to_string())],
            abort_signal: options.abort_signal.clone(),
            call_id: RecordingContext::current().map(|c| c.call_id),
            recording_context: RecordingContext::current(),
            fetch: Some(self.fetch.clone()),
            ..Default::default()
        };
        let body = self.request_body.clone();
        let out = post_json_to_api(
            req,
            body.clone(),
            create_event_source_response_handler::<Value>(),
            create_status_code_error_response_handler(),
        )
        .await?;
        let sse_stream = out.value;
        let mapped = async_stream::stream! {
            yield Ok(StreamPart::TextStart {
                id: "t1".to_string(),
                provider_metadata: None,
            });
            futures::pin_mut!(sse_stream);
            while let Some(item) = sse_stream.next().await {
                match item {
                    Ok(val) => {
                        let text = val["choices"][0]["delta"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        yield Ok(StreamPart::TextDelta {
                            id: "t1".to_string(),
                            delta: text,
                            provider_metadata: None,
                        });
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                }
            }
            yield Ok(StreamPart::TextEnd {
                id: "t1".to_string(),
                provider_metadata: None,
            });
            yield Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: Some("stop".to_string()),
                },
                usage: Usage::default(),
                provider_metadata: None,
            });
        };
        Ok(StreamResult {
            stream: Box::pin(mapped),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(StreamResponseInfo {
                headers: Some(out.response_headers),
            }),
        })
    }
}

async fn run_w_all_end_to_end() -> MemStats {
    let mut sse_buf = String::new();
    let chunk_text = "z".repeat(128);
    for i in 0..250 {
        sse_buf.push_str(&format!(
            "data: {{\"id\":\"chatcmpl-{i}\",\"choices\":[{{\"delta\":{{\"content\":\"{chunk_text}\"}}}}]}}\n\n"
        ));
    }
    sse_buf.push_str("data: [DONE]\n\n");
    let fetch: Arc<dyn Fetch> = Arc::new(InMemoryFetch {
        sse_payload: Bytes::from(sse_buf),
        json_payload: Bytes::new(),
    });
    let request_body = build_large_request_body(24, 2500, 99); // ~64 KB JSON AST

    let g = MeasureGuard::start();
    let trace_store = Arc::new(RingTraceStore::with_capacity(64, 64));
    let sink: Arc<dyn TraceSink> = trace_store.clone();
    let base_model = Arc::new(EndToEndModel {
        fetch,
        request_body,
    });
    let traced_model = TraceLayer::new(base_model, sink).with_rules_auditor(false);
    let ring_recorder = Arc::new(RingRecorder::with_capacity(64));
    ring_recorder.record_input("e2e-stalled-0", &CallOptions::default(), "openai", "gpt-4o");
    aimux_core::recording::init_recording(Some(ring_recorder.clone()));

    for i in 0..120 {
        let mut opts = GenerateTextOptions::default();
        opts.session_id = Some(format!("e2e-sess-{i:04}"));
        let mut res = stream_text(&traced_model, "hello world", opts)
            .await
            .unwrap();
        while let Some(part) = res.stream.next().await {
            std::hint::black_box(part.unwrap());
        }
    }

    aimux_core::recording::init_recording(None);
    let stats = g.snapshot();
    drop(ring_recorder);
    drop(traced_model);
    drop(trace_store);
    stats
}

fn fmt_kb(bytes: usize) -> String {
    format!("{:.1} KB", bytes as f64 / 1024.0)
}

fn fmt_mb(bytes: usize) -> String {
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

async fn verify_edge_case_invariants() {
    // 1. M1: When the 256-slot bounded pump channel is full and a timeout fires,
    // the terminal timeout error is preserved in `overflow_error` and delivered
    // after buffered items rather than being dropped by `try_send`.
    struct BurstThenStallModel;
    #[async_trait]
    impl LanguageModel for BurstThenStallModel {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "burst-stall"
        }
        async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unimplemented!()
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            let s = async_stream::stream! {
                yield Ok(StreamPart::TextStart { id: "t1".into(), provider_metadata: None });
                for _ in 0..300 {
                    yield Ok(StreamPart::TextDelta {
                        id: "t1".into(),
                        delta: "chunk".into(),
                        provider_metadata: None,
                    });
                }
                futures::future::pending::<()>().await;
            };
            Ok(StreamResult {
                stream: Box::pin(s),
                request: None,
                response: None,
            })
        }
    }

    let mut opts = GenerateTextOptions::default();
    opts.timeout = Some(aimux_core::options::TimeoutConfiguration {
        total_ms: Some(15),
        chunk_ms: None,
        ..Default::default()
    });
    let mut res = stream_text(&BurstThenStallModel, "hi", opts).await.unwrap();
    // Let the pump fill the 256-slot channel and trip the 15ms total timeout while full
    tokio::time::sleep(Duration::from_millis(35)).await;
    let mut saw_timeout_err = false;
    while let Some(item) = res.stream.next().await {
        if item.is_err() {
            saw_timeout_err = true;
            break;
        }
    }
    assert!(
        saw_timeout_err,
        "timeout firing while bounded pump channel is full must still deliver Err to consumer"
    );

    // 2. M1: Consumer backpressure on a full bounded channel must not trip chunk_ms
    // when the provider yields chunks immediately after tx.send unblocks.
    struct FastBurstModel;
    #[async_trait]
    impl LanguageModel for FastBurstModel {
        fn provider(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "fast-burst"
        }
        async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unimplemented!()
        }
        async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
            let s = async_stream::stream! {
                yield Ok(StreamPart::TextStart { id: "t1".into(), provider_metadata: None });
                for i in 0..320 {
                    if i == 255 || i == 256 {
                        // Non-output part arriving right as the 256-slot channel fills up
                        yield Ok(StreamPart::ResponseMetadata(aimux_core::types::ResponseMetadata::default()));
                    } else {
                        yield Ok(StreamPart::TextDelta {
                            id: "t1".into(),
                            delta: "x".into(),
                            provider_metadata: None,
                        });
                    }
                }
                yield Ok(StreamPart::TextEnd { id: "t1".into(), provider_metadata: None });
                yield Ok(StreamPart::Finish {
                    finish_reason: FinishReason {
                        unified: FinishReasonUnified::Stop,
                        raw: Some("stop".into()),
                    },
                    usage: Usage::default(),
                    provider_metadata: None,
                });
            };
            Ok(StreamResult {
                stream: Box::pin(s),
                request: None,
                response: None,
            })
        }
    }

    let mut opts2 = GenerateTextOptions::default();
    opts2.timeout = Some(aimux_core::options::TimeoutConfiguration {
        chunk_ms: Some(15),
        ..Default::default()
    });
    let mut res2 = stream_text(&FastBurstModel, "hi", opts2).await.unwrap();
    // Consumer stalls 35ms (> 15ms chunk_ms) while the 256-slot channel is full
    tokio::time::sleep(Duration::from_millis(35)).await;
    while let Some(item) = res2.stream.next().await {
        assert!(
            item.is_ok(),
            "consumer backpressure on full channel must not trip chunk timeout: {item:?}"
        );
    }

    // 3. M4: JsonlRecorder evicts >128 incomplete calls and ignores late trailing
    // events for already-evicted calls without re-inserting orphan placeholders.
    let tmp_dir = std::env::temp_dir().join(format!(
        "aimux_ablation_evict_test_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let jsonl_file = {
        let jsonl = JsonlRecorder::new(&tmp_dir);
        let call_opts = CallOptions::default();
        for i in 0..140 {
            let cid = format!("evict-call-{i}");
            jsonl.record_input(&cid, &call_opts, "openai", "gpt-4o");
        }
        // Call 0 was already evicted when call 128..139 arrived; send late trailing events for call 0
        jsonl.record_transport_closed("evict-call-0");
        jsonl.record_outcome(
            "evict-call-0",
            &OutcomeRecord::success(FinishReasonUnified::Stop, &LanguageModelUsage::default()),
        );
        jsonl.flush();
        tmp_dir.join("recordings.jsonl")
    };
    let content = std::fs::read_to_string(&jsonl_file).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let call0_lines = content
        .lines()
        .filter(|line| line.contains("\"evict-call-0\""))
        .count();
    assert_eq!(
        call0_lines, 1,
        "already-evicted call must not produce a second orphan record on late trailing events"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn memory_ablation_suite() {
    verify_edge_case_invariants().await;

    let w1a = run_w1a_stream_text_hot_path().await;
    let w1b = run_w1b_stream_text_backpressure().await;
    let w2 = run_w2_provider_utils_hot_path().await;
    let w3 = run_w3_trace_store_and_layer().await;
    let w4 = run_w4_recording_retention();
    let w_all = run_w_all_end_to_end().await;

    println!("\n==================== MEMORY ABLATION METRICS ====================");
    println!(
        "| Workload | Peak Heap | Retained / Mid Heap | Total Allocated | Alloc Count |"
    );
    println!("|---|---:|---:|---:|---:|");
    println!(
        "| W1a: stream_text (5x 128KB req+resp) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_kb(w1a.peak_bytes),
        w1a.peak_bytes,
        fmt_kb(w1a.retained_bytes),
        w1a.retained_bytes,
        fmt_mb(w1a.total_alloc_bytes),
        w1a.total_alloc_bytes,
        w1a.alloc_count
    );
    println!(
        "| W1b: stream_text slow-consumer (8MB) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w1b.peak_bytes),
        w1b.peak_bytes,
        fmt_mb(w1b.retained_bytes),
        w1b.retained_bytes,
        fmt_mb(w1b.total_alloc_bytes),
        w1b.total_alloc_bytes,
        w1b.alloc_count
    );
    println!(
        "| W2: provider-utils (20x 145KB JSON/SSE) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w2.peak_bytes),
        w2.peak_bytes,
        fmt_kb(w2.retained_bytes),
        w2.retained_bytes,
        fmt_mb(w2.total_alloc_bytes),
        w2.total_alloc_bytes,
        w2.alloc_count
    );
    println!(
        "| W3: TraceLayer+Store (800x 64KB, cap=128) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w3.peak_bytes),
        w3.peak_bytes,
        fmt_mb(w3.retained_bytes),
        w3.retained_bytes,
        fmt_mb(w3.total_alloc_bytes),
        w3.total_alloc_bytes,
        w3.alloc_count
    );
    println!(
        "| W4: Ring+JsonlRecorder (2600 calls, cap=64) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w4.peak_bytes),
        w4.peak_bytes,
        fmt_mb(w4.retained_bytes),
        w4.retained_bytes,
        fmt_mb(w4.total_alloc_bytes),
        w4.total_alloc_bytes,
        w4.alloc_count
    );
    println!(
        "| W_ALL: End-to-End (120x full pipeline) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w_all.peak_bytes),
        w_all.peak_bytes,
        fmt_mb(w_all.retained_bytes),
        w_all.retained_bytes,
        fmt_mb(w_all.total_alloc_bytes),
        w_all.total_alloc_bytes,
        w_all.alloc_count
    );
    println!("=================================================================\n");

    // Verify memory ceilings for every ablation module (M1-M4 + W_ALL)
    assert!(
        w1a.peak_bytes < 512 * 1024 && w1a.retained_bytes < 16 * 1024,
        "W1a memory regression: {w1a:?}"
    );
    assert!(
        w1b.peak_bytes < 2 * 1024 * 1024 && w1b.retained_bytes < 2 * 1024 * 1024,
        "W1b backpressure memory regression: {w1b:?}"
    );
    assert!(
        w2.peak_bytes < 1024 * 1024 && w2.retained_bytes < 300 * 1024,
        "W2 provider-utils memory regression: {w2:?}"
    );
    assert!(
        w3.peak_bytes < 1024 * 1024 && w3.retained_bytes < 600 * 1024,
        "W3 TraceLayer+TraceStore memory regression: {w3:?}"
    );
    assert!(
        w4.peak_bytes < 20 * 1024 * 1024 && w4.retained_bytes < 10 * 1024 * 1024,
        "W4 Recorder memory regression: {w4:?}"
    );
    assert!(
        w_all.peak_bytes < 10 * 1024 * 1024 && w_all.retained_bytes < 10 * 1024 * 1024,
        "W_ALL end-to-end memory regression: {w_all:?}"
    );
}
