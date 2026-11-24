//! RFC-0049/0050: direct Rust timing and allocation counts on fixed in-memory HTTP.
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use aimux_core::generate::{GenerateTextOptions, generate_text, stream_text};
use aimux_core::parse_tool_call::parse_tool_call;
use aimux_core::result::ContentPart;
use aimux_core::stream_part::TextStreamPart;
use aimux_core::tool::RawToolCall;
use aimux_provider_utils::{
    Fetch, FetchError, FetchRequest, FetchResponse, HeaderMapOpt, HttpBody, HttpRequest,
    combine_headers, create_json_response_handler, create_status_code_error_response_handler,
    normalize_headers, post_to_api_with_header_layers,
};
use aimux_providers::openai::{OpenAIProviderSettings, create_openai};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::{StatusCode, header::HeaderMap};
use serde_json::{Value, json};

struct CountAlloc;
static COUNTING: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

fn count(size: usize) {
    if COUNTING.load(Relaxed) {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(size as u64, Relaxed);
    }
}

// SAFETY: All allocation and deallocation operations delegate to System with
// the caller's original arguments. Counters do not allocate.
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

struct MemoryFetch {
    body: Bytes,
    headers: HeaderMap,
}

#[async_trait]
impl Fetch for MemoryFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse::from_bytes(
            StatusCode::OK,
            self.headers.clone(),
            request.url,
            self.body.clone(),
        ))
    }
}

const DELTAS: usize = 1000;
const STEADY_DELTAS: usize = DELTAS - 2;
const DELTA_TEXT: &str = "word\n";
const OP_TIMEOUT: Duration = Duration::from_secs(15);

fn chat(text: &str) -> String {
    json!({
        "id": "bench", "object": "chat.completion", "model": "gpt-4o", "created": 1,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    })
    .to_string()
}

fn stream_fixture() -> String {
    let event = |delta: Value, finish: Option<&str>| {
        format!(
            "data: {}\n\n",
            json!({
                "id": "bench", "object": "chat.completion.chunk", "model": "gpt-4o", "created": 1,
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
            })
        )
    };
    let mut sse = event(json!({"role": "assistant"}), None);
    for _ in 0..DELTAS {
        sse.push_str(&event(json!({"content": DELTA_TEXT}), None));
    }
    sse.push_str(&event(json!({}), Some("stop")));
    sse.push_str("data: [DONE]\n\n");
    sse
}

fn options(tool_count: usize) -> GenerateTextOptions {
    let tools = (0..tool_count)
        .map(|i| {
            json!({"type": "function", "name": format!("lookup_{i}"), "inputSchema": {
                "type": "object", "required": ["query"], "additionalProperties": false,
                "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "minimum": 1}}
            }})
        })
        .collect::<Vec<_>>();
    // Keep the fixture source identical across the Vec and Arc options types.
    serde_json::from_value(if tools.is_empty() {
        json!({"maxRetries": 0})
    } else {
        json!({"maxRetries": 0, "tools": tools})
    })
    .unwrap()
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(
        (2..=3).contains(&args.len()),
        "usage: perf_data_plane <scenario> <samples> [warmup]"
    );
    let scenario = args[0].as_str();
    let samples: usize = args[1].parse().unwrap();
    let warmup: usize = args.get(2).map_or(20, |value| value.parse().unwrap());
    assert!(samples > 0);
    let (prompt, text, tool_count, streaming) = match scenario {
        "request-small" => ("hi".into(), "hello".into(), 0, false),
        "request-200k" | "serialize-200k" => ("a".repeat(200_000), "b".repeat(50_000), 0, false),
        "gen-tools-loop" | "validate-tool" => ("hi".into(), "hello".into(), 30, false),
        "stream-1k-chunks" => ("hi".into(), DELTA_TEXT.repeat(DELTAS), 0, true),
        "id-control" | "id-text" | "id-reasoning" | "active-hashset" | "active-vec"
        | "headers-legacy" | "headers-direct" => ("".into(), "".into(), 0, false),
        _ => panic!("unknown scenario {scenario}"),
    };
    let (body, content_type) = if streaming {
        (stream_fixture(), "text/event-stream")
    } else {
        (chat(&text), "application/json")
    };
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::CONTENT_TYPE, content_type.parse().unwrap());
    let fetch = Arc::new(MemoryFetch {
        body: body.into(),
        headers,
    });
    let model = create_openai(OpenAIProviderSettings {
        api_key: Some("benchmark".into()),
        base_url: Some("https://example.test/v1".into()),
        fetch: Some(fetch.clone()),
        ..Default::default()
    })
    .unwrap()
    .chat("gpt-4o");
    let options = options(tool_count);
    let tool_call = RawToolCall {
        tool_call_id: "lookup".into(),
        tool_name: "lookup_0".into(),
        input: r#"{"query":"hi","limit":1}"#.into(),
        provider_executed: None,
        dynamic: None,
        provider_metadata: None,
    };
    let header_layers: Arc<[HeaderMapOpt]> = Arc::new([
        [
            ("Authorization", Some("Bearer benchmark")),
            ("User-Agent", Some("provider")),
            ("X-Removed", Some("old")),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.map(str::to_owned)))
        .collect(),
        [("X-Removed", None), ("x-extra", Some("call"))]
            .into_iter()
            .map(|(name, value)| (name.into(), value.map(str::to_owned)))
            .collect(),
    ]);
    let active_id = "0".to_owned();
    let active_hashset = std::collections::HashSet::from([active_id.clone()]);
    let active_vec = Vec::from([active_id.clone()]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let serialize_fixture = if scenario == "serialize-200k" {
            Some(generate_text(&model, prompt.as_str(), options.clone()).await.unwrap())
        } else {
            None
        };
        let run = || async {
            if scenario == "validate-tool" {
                let call = parse_tool_call(
                    tool_call.clone(), options.tools.as_deref(), None, &[], None,
                ).await;
                assert_eq!(call.input.get("query").and_then(Value::as_str), Some("hi"));
                black_box(call);
                return;
            }
            // Isolate constant-ID ownership from SSE decoding and stream
            // scheduling. The control runs the same number of borrowed IDs.
            if matches!(scenario, "id-control" | "id-text" | "id-reasoning") {
                let id = if scenario == "id-reasoning" { "reasoning-0" } else { "0" };
                for _ in 0..STEADY_DELTAS {
                    let id = black_box(id);
                    if scenario == "id-control" {
                        black_box(id);
                    } else {
                        drop(black_box(id.to_owned()));
                    }
                }
                return;
            }
            if matches!(scenario, "active-hashset" | "active-vec") {
                if scenario == "active-hashset" {
                    for _ in 0..STEADY_DELTAS {
                        black_box(active_hashset.contains(black_box(&active_id)));
                    }
                } else {
                    for _ in 0..STEADY_DELTAS {
                        black_box(active_vec.contains(black_box(&active_id)));
                    }
                }
                return;
            }
            if matches!(scenario, "headers-legacy" | "headers-direct") {
                let (headers, layers) = if scenario == "headers-direct" {
                    (Vec::new(), Some(header_layers.clone()))
                } else {
                    (normalize_headers(combine_headers(&[&header_layers[0], &header_layers[1]])), None)
                };
                let result = post_to_api_with_header_layers(
                    HttpRequest {
                        url: "https://example.test/v1".into(),
                        headers,
                        fetch: Some(fetch.clone()),
                        ..HttpRequest::default()
                    },
                    layers,
                    HttpBody::Json(json!({})),
                    create_json_response_handler::<Value>(),
                    create_status_code_error_response_handler(),
                ).await.unwrap();
                black_box(result);
                return;
            }
            if let Some(result) = &serialize_fixture {
                let output = serde_json::to_vec(result).unwrap();
                black_box(output.len());
                return;
            }
            if streaming {
                let mut result = stream_text(&model, prompt.as_str(), options.clone()).await.unwrap();
                let mut deltas = 0;
                let mut finished = false;
                while let Some(part) = result.stream.next().await {
                    match part.unwrap() {
                        TextStreamPart::TextDelta { text, .. } => {
                            assert_eq!(text, DELTA_TEXT);
                            deltas += 1;
                        }
                        TextStreamPart::Finish { .. } => finished = true,
                        TextStreamPart::Error { .. } | TextStreamPart::Abort { .. } => {
                            panic!("invalid benchmark stream")
                        }
                        _ => {}
                    }
                }
                assert!(finished);
                assert_eq!(deltas, DELTAS);
            } else {
                let result = generate_text(&model, prompt.as_str(), options.clone()).await.unwrap();
                assert_eq!(result.steps().len(), 1);
                assert!(matches!(
                    &result.steps()[0].content[..],
                    [ContentPart::Text { text: actual, .. }] if actual == &text
                ));
                black_box(result);
            }
        };

        for _ in 0..warmup {
            tokio::time::timeout(OP_TIMEOUT, run()).await.unwrap();
        }
        let mut nanoseconds = Vec::with_capacity(samples);
        // The timer pass does not increment counters. The separate allocation
        // pass includes the current-thread runtime's pump and result teardown.
        for _ in 0..samples {
            let start = Instant::now();
            tokio::time::timeout(OP_TIMEOUT, run()).await.unwrap();
            nanoseconds.push(start.elapsed().as_nanos());
        }
        let mut allocations = Vec::with_capacity(samples);
        let mut allocation_bytes = Vec::with_capacity(samples);
        for _ in 0..samples {
            CALLS.store(0, Relaxed);
            BYTES.store(0, Relaxed);
            COUNTING.store(true, Relaxed);
            tokio::time::timeout(OP_TIMEOUT, run()).await.unwrap();
            COUNTING.store(false, Relaxed);
            allocations.push(CALLS.load(Relaxed));
            allocation_bytes.push(BYTES.load(Relaxed));
        }
        println!("{}", json!({
            "scenario": scenario, "samples": samples, "warmup": warmup,
            "elapsed_ns": nanoseconds, "allocation_calls": allocations, "allocated_bytes": allocation_bytes,
            "deltas_per_sample": if streaming { DELTAS } else { 0 },
            "operation_timeout_ms": OP_TIMEOUT.as_millis(),
        }));
    });
}
