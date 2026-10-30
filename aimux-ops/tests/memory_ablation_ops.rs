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
    // Pre-create chunks outside measurement and keep `chunks` alive so dropping
    // stream items inside MeasureGuard only decrements Bytes refcounts without
    // freeing pre-existing allocations.
    let mut header_chunk = vec![0u8; 1024 * 1024];
    header_chunk[..4].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    let first_bytes = header_chunk.into();
    let mut chunks = vec![Ok(first_bytes)];
    for _ in 0..23 {
        chunks.push(Ok(vec![b'a'; 1024 * 1024].into()));
    }
    let chunks_for_stream = chunks.clone();

    // Prepare a single 16-byte chunk claiming a 1 MiB frame length with a corrupted
    // prelude CRC (and no subsequent chunks). Without early 12-byte prelude CRC
    // validation, the decoder would wait for 1 MiB and fail with "Unexpected EOF"
    // instead of "prelude checksum".
    let mut bad_prelude = vec![0u8; 16];
    bad_prelude[..4].copy_from_slice(&(1024 * 1024u32).to_be_bytes());
    bad_prelude[8..12].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let bad_prelude_bytes = bad_prelude.into();
    let bad_prelude_chunks = vec![Ok(bad_prelude_bytes)];
    let bad_prelude_for_stream = bad_prelude_chunks.clone();

    // Also prepare a valid 2 MiB frame followed by a small frame to measure post-drain buffer retention.
    // Keep `large_frame_holder` and `small_frame_holder` alive across `MeasureGuard` so dropping
    // cloned `Bytes` handles inside the stream never subtracts pre-guard memory.
    let mut large_frame_holder = chunks.clone();
    large_frame_holder.clear();
    large_frame_holder.push(Ok(
        encode_message(
            "event",
            "contentBlockDelta",
            &"x".repeat(2 * 1024 * 1024),
        )
        .into(),
    ));
    let mut small_frame_holder = chunks.clone();
    small_frame_holder.clear();
    small_frame_holder.push(Ok(encode_message("event", "messageStop", "{}").into()));
    let large_frame_for_stream = large_frame_holder.remove(0);
    let small_frame_for_stream = small_frame_holder.remove(0);
    large_frame_holder.push(large_frame_for_stream.clone());
    small_frame_holder.push(small_frame_for_stream.clone());

    let g_attack = MeasureGuard::start();
    let mut stream = decode_stream(futures::stream::iter(chunks_for_stream).boxed());
    let first_outcome = stream.next().await;
    let first_err = first_outcome
        .expect("stream must yield an item")
        .expect_err("oversized 2 GiB frame header must fail immediately");
    assert!(
        first_err.to_string().contains("frame length"),
        "expected frame length error, got: {first_err}"
    );
    assert!(
        stream.next().await.is_none(),
        "stream must terminate after frame length error"
    );
    let attack_stats = g_attack.snapshot();
    drop(stream);

    // Verify early 12-byte prelude CRC check fails with "prelude checksum" on a single 16-byte chunk
    // (before accumulating the 1 MiB frame length it claims).
    let mut bad_crc_stream = decode_stream(futures::stream::iter(bad_prelude_for_stream).boxed());
    let bad_crc_err = bad_crc_stream
        .next()
        .await
        .expect("stream must yield an item")
        .expect_err("corrupted 12-byte prelude CRC must fail before full frame buffering");
    assert!(
        bad_crc_err.to_string().contains("prelude checksum"),
        "expected early prelude checksum error (not EOF), got: {bad_crc_err}"
    );
    drop(bad_crc_stream);

    // Now measure the 2 MiB valid frame and post-drain buffer retention in an isolated guard
    // where no pre-existing allocations are freed.
    let g_shrink = MeasureGuard::start();
    let (tx, rx) = futures::channel::mpsc::unbounded();
    tx.unbounded_send(large_frame_for_stream).unwrap();
    let mut stream2 = decode_stream(rx.boxed());
    let msg1 = stream2.next().await.unwrap().unwrap();
    assert_eq!(msg1.event_type, "contentBlockDelta");
    drop(msg1);
    // At this point `stream2` is still alive and holds its internal `buffer: Vec<u8>`.
    // Because `large_frame_holder` is kept alive outside `g_shrink`, `retained_after_large_frame`
    // directly reflects `stream2`'s retained heap (~16 KiB after `shrink_to(16 * 1024)`,
    // vs >2 MiB without `shrink_to`).
    let retained_after_large_frame = g_shrink.snapshot().retained_bytes;
    tx.unbounded_send(small_frame_for_stream).unwrap();
    drop(tx);
    let msg2 = stream2.next().await.unwrap().unwrap();
    assert_eq!(msg2.event_type, "messageStop");
    drop(msg2);
    let shrink_stats = g_shrink.snapshot();
    drop(stream2);

    std::hint::black_box((
        &chunks,
        &bad_prelude_chunks,
        &large_frame_holder,
        &small_frame_holder,
    ));

    MemStats {
        peak_bytes: attack_stats.peak_bytes.max(shrink_stats.peak_bytes),
        retained_bytes: retained_after_large_frame,
        total_alloc_bytes: attack_stats.total_alloc_bytes + shrink_stats.total_alloc_bytes,
        alloc_count: attack_stats.alloc_count + shrink_stats.alloc_count,
    }
}

// W5b: `aimux_ops::manifest::manifest_digest()` (30 calls) + `manifest()` (10 calls)
fn run_w5b_manifest_calls() -> MemStats {
    // Warm up OnceLock cache first so we measure steady-state repeated calls
    let warmup_digest = aimux_ops::manifest::manifest_digest();
    assert!(!warmup_digest.is_empty());

    let g_digest = MeasureGuard::start();
    for _ in 0..30 {
        let d = aimux_ops::manifest::manifest_digest();
        assert_eq!(d, warmup_digest);
        std::hint::black_box(d);
    }
    let digest_only = g_digest.snapshot();
    assert!(
        digest_only.total_alloc_bytes < 8 * 1024,
        "30 cached manifest_digest() calls must allocate < 8 KiB (got {} B)",
        digest_only.total_alloc_bytes
    );

    let g = MeasureGuard::start();
    for _ in 0..30 {
        let d = aimux_ops::manifest::manifest_digest();
        assert_eq!(d, warmup_digest);
        std::hint::black_box(d);
    }
    for _ in 0..10 {
        let m = aimux_ops::manifest::manifest();
        assert_eq!(
            m.get("digest").and_then(Value::as_str),
            Some(warmup_digest.as_str())
        );
        std::hint::black_box(m);
    }
    g.snapshot()
}

// W5c: `aimux_providers::catalogue::parse_anya2a_all_owned` (1,000 models)
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
    assert_eq!(
        cat.specs.get("openai").map(std::collections::HashMap::len),
        Some(1000)
    );
    assert!(cat.specs["openai"]["model-0"].raw.is_some());
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

    // Verify M5 memory bounds
    assert!(
        w5a.peak_bytes < 6 * 1024 * 1024,
        "W5a peak heap ({}) must stay well below the 24 MiB attack stream",
        w5a.peak_bytes
    );
    assert!(
        (8 * 1024..64 * 1024).contains(&w5a.retained_bytes),
        "W5a retained buffer after 2 MiB frame ({}) must be non-zero (~16 KiB) and shrink below 64 KiB",
        w5a.retained_bytes
    );
    assert!(
        w5b.total_alloc_bytes < 35 * 1024 * 1024,
        "W5b total allocated bytes ({}) must reflect OnceLock manifest caching",
        w5b.total_alloc_bytes
    );
    assert!(
        w5c.peak_bytes < 5 * 1024 * 1024 && w5c.total_alloc_bytes < 5 * 1024 * 1024,
        "W5c peak/total heap ({w5c:?}) must reflect in-place raw Value move without full-tree cloning"
    );
}
