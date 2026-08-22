//! FFI tests for `aimux_recording_try_flush` (write-failure observability,
//! see #136).
//!
//! Everything lives in one `#[test]` on purpose: the global recorder is a
//! process-level singleton, and the outcomes must be observed in a
//! controlled sequence. The sticky `AIMUX_E_RECORDING_WRITE` and the
//! `WRITER_GONE` paths are not portable to inject from the FFI layer (it
//! needs a post-open ENOSPC); they are covered by the core `FailingWriter`
//! test from #133.

mod common;

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};

use aimux_ffi::{
    AIMUX_E_RECORDING_INIT, aimux_init_recording, aimux_recording_stop, aimux_recording_try_flush,
};
use common::{expect_ffi_error, expect_recording_error, ok};

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
    aimux_recording_stop();

    // No recorder initialized: nothing to flush is a success.
    ok(aimux_recording_try_flush(), "try_flush (no recorder)");

    // Valid directory: flush confirms the (empty) JSONL on disk.
    let dir = unique_dir("ok");
    let c_dir = CString::new(dir.to_str().unwrap()).unwrap();
    ok(aimux_init_recording(c_dir.as_ptr()), "init_recording");
    ok(aimux_recording_try_flush(), "try_flush");
    assert!(
        dir.join("recordings.jsonl").exists(),
        "flush should have created the JSONL file"
    );
    aimux_recording_stop();
    let _ = std::fs::remove_dir_all(&dir);

    // Unwritable directory (parent path is a regular file): init itself fails
    // with INIT — the recorder is no longer silently degraded to a no-op and
    // discovered only at the first flush (the gap #136 was about).
    let blocker = unique_dir("blocker");
    std::fs::create_dir_all(&blocker).unwrap();
    let file = blocker.join("occupied");
    std::fs::write(&file, b"x").unwrap();
    let bad_dir = file.join("sub");
    let c_bad = CString::new(bad_dir.to_str().unwrap()).unwrap();
    let (code, text) =
        expect_recording_error(aimux_init_recording(c_bad.as_ptr()), "init (bad dir)");
    assert_eq!(code, AIMUX_E_RECORDING_INIT);
    assert!(text.contains("recording init failed"), "{text}");
    // The failed init left no recorder behind: flushing is still a success.
    ok(aimux_recording_try_flush(), "try_flush after failed init");

    // The operation is recording-related, but malformed C input is still a
    // C ABI failure — no recording view.
    assert_eq!(
        expect_ffi_error(aimux_init_recording(std::ptr::null()), "init (NULL dir)"),
        "dir: must not be NULL"
    );
    aimux_recording_stop();
    let _ = std::fs::remove_dir_all(&blocker);
}
