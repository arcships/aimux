//! Smoke tests for the recording-ring FFI entry points.
//!
//! The global recorder is a process-level singleton shared across tests in this
//! binary. `default_ring_returns_zero` resets it via `aimux_recording_stop`
//! before and after, so it does not depend on ordering; `cap == 0` returns -1
//! before touching global state, so it is order-independent as well.

use aimux_ffi::{
    aimux_init_recording_ring, aimux_init_recording_ring_default, aimux_recording_stop,
};

#[test]
fn default_ring_returns_zero() {
    // Reset the global recorder so this test is independent of ordering.
    assert_eq!(aimux_recording_stop(), 0);
    // No-argument entry point initializes a default-capacity (2048) ring.
    assert_eq!(aimux_init_recording_ring_default(), 0);
    // Leave the global recorder unset for any downstream tests.
    assert_eq!(aimux_recording_stop(), 0);
}

#[test]
fn explicit_zero_capacity_still_errors() {
    // The default entry point does not relax the "cap == 0 => -1" contract:
    // passing an explicit 0 must keep failing.
    assert_eq!(aimux_init_recording_ring(0), -1);
}
