//! Memory ablation benchmark suite for `aimux-providers` + `aimux-ops` (Module M5).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use serde_json::{Value, json};

use aimux_providers::bedrock::event_stream_decoder::{decode_stream, encode_message};

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

// W5a: Bedrock event-stream decoder under malicious 2 GiB header (24x 1 MiB chunks) + large frame buffer retention
async fn run_w5a_bedrock_event_stream() -> MemStats {
    // Pre-create chunks outside measurement
    let mut header_chunk = vec![0u8; 1024 * 1024];
    header_chunk[..4].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    let first_bytes = header_chunk.into();
    let mut chunks = vec![Ok(first_bytes)];
    for _ in 0..23 {
        chunks.push(Ok(vec![b'a'; 1024 * 1024].into()));
    }

    // Also prepare a valid 2 MiB frame followed by a small frame to measure post-drain buffer retention
    let large_frame = encode_message(
        "event",
        "contentBlockDelta",
        &"x".repeat(2 * 1024 * 1024),
    )
    .into();
    let small_frame = encode_message("event", "messageStop", "{}").into();

    let g = MeasureGuard::start();
    let mut stream = decode_stream(futures::stream::iter(chunks).boxed());
    while let Some(item) = stream.next().await {
        if item.is_err() {
            break;
        }
    }
    drop(stream);

    // Now feed the 2 MiB valid frame, drain it, and measure retained buffer inside live stream before small frame
    let (tx, rx) = futures::channel::mpsc::unbounded();
    tx.unbounded_send(Ok(large_frame)).unwrap();
    let mut stream2 = decode_stream(rx.boxed());
    let msg1 = stream2.next().await.unwrap().unwrap();
    drop(msg1);
    let retained_after_large_frame = g.snapshot().retained_bytes;
    tx.unbounded_send(Ok(small_frame)).unwrap();
    drop(tx);
    let _ = stream2.next().await;
    let end = g.snapshot();
    drop(stream2);

    MemStats {
        peak_bytes: end.peak_bytes,
        retained_bytes: retained_after_large_frame,
        total_alloc_bytes: end.total_alloc_bytes,
        alloc_count: end.alloc_count,
    }
}

// W5b: `aimux_ops::manifest::manifest_digest()` (30 calls) + `manifest()` (10 calls)
fn run_w5b_manifest_calls() -> MemStats {
    // Warm up one call first so we measure steady-state + repeated calls
    let g = MeasureGuard::start();
    for _ in 0..30 {
        let d = aimux_ops::manifest::manifest_digest();
        std::hint::black_box(d);
    }
    for _ in 0..10 {
        let m = aimux_ops::manifest::manifest();
        std::hint::black_box(m);
    }
    g.snapshot()
}

// W5c: `aimux_providers::catalogue::parse_anya2a_all` (1,000 models)
fn run_w5c_catalogue_parse() -> MemStats {
    let models: Vec<Value> = (0..1000)
        .map(|i| {
            json!({
                "id": format!("model-{i}"),
                "display_name": format!("Model {i}"),
                "type": "chat",
                "tool_call": true,
                "structured_output": true,
                "temperature": true,
                "attachment": false,
                "limit": { "context": 128000, "output": 16384 },
                "modalities": { "input": ["text", "image"], "output": ["text"] },
                "cost": { "input": 2.5, "output": 10.0, "cache_read": 1.25 }
            })
        })
        .collect();
    let raw = json!({
        "updated_at": "1785976654990",
        "providers": {
            "openai": { "models": models }
        }
    });
    let raw_bytes = serde_json::to_vec(&raw).unwrap();

    let g = MeasureGuard::start();
    let parsed_val: Value = serde_json::from_slice(&raw_bytes).unwrap();
    let cat = aimux_providers::catalogue::parse_anya2a_all_owned(parsed_val).unwrap();
    let stats = g.snapshot();
    std::hint::black_box(&cat);
    drop(cat);
    stats
}

fn fmt_kb(bytes: usize) -> String {
    format!("{:.1} KB", bytes as f64 / 1024.0)
}

fn fmt_mb(bytes: usize) -> String {
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[tokio::test(flavor = "current_thread")]
async fn memory_ablation_ops_suite() {
    let w5a = run_w5a_bedrock_event_stream().await;
    let w5b = run_w5b_manifest_calls();
    let w5c = run_w5c_catalogue_parse();

    println!("\n==================== M5 OPS/PROVIDERS ABLATION METRICS ====================");
    println!(
        "| Workload | Peak Heap | Retained / Mid Heap | Total Allocated | Alloc Count |"
    );
    println!("|---|---:|---:|---:|---:|");
    println!(
        "| W5a: Bedrock decode_stream (24MB attack + 2MB frame) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w5a.peak_bytes),
        w5a.peak_bytes,
        fmt_kb(w5a.retained_bytes),
        w5a.retained_bytes,
        fmt_mb(w5a.total_alloc_bytes),
        w5a.total_alloc_bytes,
        w5a.alloc_count
    );
    println!(
        "| W5b: manifest_digest (30x) + manifest (10x) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w5b.peak_bytes),
        w5b.peak_bytes,
        fmt_kb(w5b.retained_bytes),
        w5b.retained_bytes,
        fmt_mb(w5b.total_alloc_bytes),
        w5b.total_alloc_bytes,
        w5b.alloc_count
    );
    println!(
        "| W5c: Catalogue parse (1,000 models) | {} ({} B) | {} ({} B) | {} ({} B) | {} |",
        fmt_mb(w5c.peak_bytes),
        w5c.peak_bytes,
        fmt_mb(w5c.retained_bytes),
        w5c.retained_bytes,
        fmt_mb(w5c.total_alloc_bytes),
        w5c.total_alloc_bytes,
        w5c.alloc_count
    );
    println!("===========================================================================\n");
}
