//! Smoke tests for the recording-ring FFI entry points.
//!
//! The global recorder is a process-level singleton shared across tests in this
//! binary. `default_ring_succeeds` resets it via `aimux_recording_stop`
//! before and after, so it does not depend on ordering; `cap == 0` fails
//! before touching global state, so it is order-independent as well.

mod common;

use aimux_ffi::{
    AIMUX_E_INVALID_ARGUMENT, aimux_init_recording_ring, aimux_init_recording_ring_default,
    aimux_recording_stop,
};
use common::{expect_aimux_error, ok};

#[test]
fn default_ring_succeeds() {
    // Reset the global recorder so this test is independent of ordering.
    aimux_recording_stop();
    // No-argument entry point initializes a default-capacity (2048) ring.
    aimux_init_recording_ring_default();
    ok(aimux_init_recording_ring(8), "init_recording_ring(8)");
    // Leave the global recorder unset for any downstream tests.
    aimux_recording_stop();
}

#[test]
fn explicit_zero_capacity_still_errors() {
    // The default entry point does not relax the "cap == 0 fails" contract:
    // passing an explicit 0 must keep failing as AiMuxError::InvalidArgument.
    let (code, m) = expect_aimux_error(aimux_init_recording_ring(0), "init_recording_ring(0)");
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert!(m.contains("cap"), "{m}");
}
