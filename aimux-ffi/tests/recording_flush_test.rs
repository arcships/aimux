//! FFI tests for `aimux_recording_try_flush` (write-failure observability,
//! see #136).
//!
//! Everything lives in one `#[test]` on purpose: the global recorder is a
//! process-level singleton, and the outcomes must be observed in a
//! controlled sequence. The sticky `AIMUX_E_RECORDING_WRITE` path is not
//! portable to inject from the FFI layer (it needs a post-open ENOSPC);
//! it is covered by the core `FailingWriter` test from #133.

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};

use aimux_ffi::{
    AIMUX_E_RECORDING_WRITER_GONE, AIMUX_OK, aimux_init_recording, aimux_recording_stop,
    aimux_recording_try_flush,
};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aimux-ffi-flush-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn recording_try_flush_reports_outcomes() {
    // Reset so the test is independent of ordering.
    assert_eq!(aimux_recording_stop(), 0);

    // No recorder initialized: nothing to flush is a success.
    assert_eq!(aimux_recording_try_flush(), AIMUX_OK);

    // Valid directory: flush confirms the (empty) JSONL on disk.
    let dir = unique_dir("ok");
    let c_dir = CString::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(aimux_init_recording(c_dir.as_ptr()), 0);
    assert_eq!(aimux_recording_try_flush(), AIMUX_OK);
    assert!(
        dir.join("recordings.jsonl").exists(),
        "flush should have created the JSONL file"
    );
    assert_eq!(aimux_recording_stop(), 0);
    let _ = std::fs::remove_dir_all(&dir);

    // Unwritable directory (parent path is a regular file): the recorder
    // degrades to a no-writer no-op and try_flush reports WRITER_GONE
    // instead of a silent 0 — the exact gap #136 closes for bindings.
    let blocker = unique_dir("blocker");
    std::fs::create_dir_all(&blocker).unwrap();
    let file = blocker.join("occupied");
    std::fs::write(&file, b"x").unwrap();
    let bad_dir = file.join("sub");
    let c_bad = CString::new(bad_dir.to_str().unwrap()).unwrap();
    assert_eq!(aimux_init_recording(c_bad.as_ptr()), 0);
    assert_eq!(aimux_recording_try_flush(), AIMUX_E_RECORDING_WRITER_GONE);
    assert_eq!(aimux_recording_stop(), 0);
    let _ = std::fs::remove_dir_all(&blocker);
}
