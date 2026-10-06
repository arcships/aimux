mod common;
use aimux_ffi::*;
use common::*;
use std::ptr;

#[test]
fn decision_handles_json_errors_and_abort_follow_c_abi_contract() {
    let key = c("test-key");
    let model = c("jev-1.13");
    let endpoint = c("http://127.0.0.1:1/api/v1/systemone/");
    let mut handle = 0;
    ok(
        aimux_jev_decision_new(key.as_ptr(), model.as_ptr(), endpoint.as_ptr(), &mut handle),
        "create decision model",
    );
    assert_ne!(handle, 0);
    let mut out = ptr::null_mut();
    let request =
        c(r#"{"state":"text","questions":[{"id":"q","type":"boolean","instructions":"test"}]}"#);
    let abort = aimux_abort_signal_new();
    aimux_abort_signal_abort(abort);
    let (code, _) = expect_aimux_error(
        aimux_decide_with_abort(handle, request.as_ptr(), abort, &mut out),
        "aborted decision",
    );
    assert_eq!(code, AIMUX_E_ABORTED);
    assert!(out.is_null());
    let malformed = c("{");
    expect_ffi_error(
        aimux_decide(handle, malformed.as_ptr(), &mut out),
        "malformed JSON",
    );
    expect_ffi_error(
        aimux_decide(abort, request.as_ptr(), &mut out),
        "wrong handle type",
    );
    aimux_drop_handle(handle);
    expect_ffi_error(
        aimux_decide(handle, request.as_ptr(), &mut out),
        "dropped decision",
    );
    aimux_abort_signal_drop(abort);
}
