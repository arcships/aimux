//! Calls the exported C symbols, including ownership and argument failures.
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::Arc;

use aimux_core::AbortSignal;
use aimux_ffi::{
    aimux_error_code, aimux_error_free, aimux_error_message, aimux_error_t, aimux_free_string,
};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use serde_json::Value;

// Errors are opaque pointers: their Rust payload is never passed by value.
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn aimux_abi_version() -> u32;
    fn aimux_manifest_json(out: *mut *mut c_char) -> *mut aimux_error_t;
    fn aimux_call(
        handle: u64,
        op: *const c_char,
        params: *const c_char,
        abort: u64,
        out: *mut *mut c_char,
    ) -> *mut aimux_error_t;
    fn aimux_stream(
        handle: u64,
        op: *const c_char,
        params: *const c_char,
        abort: u64,
        callback: Option<extern "C" fn(*const c_char, *mut c_void)>,
        context: *mut c_void,
    ) -> *mut aimux_error_t;
    fn aimux_operation_push(operation: u64, data: *const u8, length: usize) -> *mut aimux_error_t;
    fn aimux_operation_next_part(
        operation: u64,
        timeout: i64,
        part: *mut *mut c_char,
        state: *mut i32,
    ) -> *mut aimux_error_t;
    fn aimux_abort(handle: u64);
    fn aimux_drop(handle: u64);
    fn aimux_error_json(error: *const aimux_error_t) -> *mut c_char;
}

unsafe fn take_string(value: *mut c_char) -> String {
    assert!(!value.is_null());
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .unwrap()
        .to_owned();
    unsafe { aimux_free_string(value) };
    text
}

fn check_error(error: *mut aimux_error_t, code: i32, name: &str) {
    assert!(!error.is_null());
    assert_eq!(aimux_error_code(error), code);
    let json: Value =
        serde_json::from_str(&unsafe { take_string(aimux_error_json(error)) }).unwrap();
    assert_eq!(json["code"], code);
    assert_eq!(json["error"]["name"], name);
    assert!(json.get("retryable").is_none());
    assert!(json.get("type").is_none());
    let first = aimux_error_message(error);
    let second = aimux_error_message(error);
    assert_ne!(first, second);
    assert_eq!(unsafe { take_string(first) }, unsafe {
        take_string(second)
    });
    aimux_error_free(error);
}

#[test]
fn manifest_and_null_safe_release() {
    unsafe {
        assert_eq!(aimux_abi_version(), 1 << 16);
        let mut output = ptr::dangling_mut::<c_char>();
        assert!(aimux_manifest_json(&mut output).is_null());
        let manifest: Value = serde_json::from_str(&take_string(output)).unwrap();
        assert_eq!(manifest["abiVersion"], aimux_abi_version());
        assert_eq!(manifest["opsVersion"], "1.0");
        check_error(aimux_manifest_json(ptr::null_mut()), 200, "NullPointer");
        assert_eq!(aimux_error_code(ptr::null()), 0);
        assert!(aimux_error_message(ptr::null()).is_null());
        assert!(aimux_error_json(ptr::null()).is_null());
        aimux_error_free(ptr::null_mut());
        aimux_free_string(ptr::null_mut());
        aimux_drop(0);
        aimux_drop(u64::MAX);
        aimux_abort(0);
        aimux_abort(u64::MAX);
    }
}

#[test]
fn call_validates_strings_and_clears_output() {
    let op = CString::new("runtime.describe").unwrap();
    let params = CString::new("{}").unwrap();
    let invalid_json = CString::new("{").unwrap();
    let invalid_utf8 = [0xff_u8, 0];
    unsafe {
        for (op_arg, params_arg, code, name) in [
            (ptr::null(), params.as_ptr(), 200, "NullPointer"),
            (op.as_ptr(), ptr::null(), 200, "NullPointer"),
            (
                invalid_utf8.as_ptr().cast(),
                params.as_ptr(),
                201,
                "InvalidUtf8",
            ),
            (
                op.as_ptr(),
                invalid_utf8.as_ptr().cast(),
                201,
                "InvalidUtf8",
            ),
            (op.as_ptr(), invalid_json.as_ptr(), 202, "InvalidWireJson"),
        ] {
            let mut output = ptr::dangling_mut::<c_char>();
            check_error(
                aimux_call(0, op_arg, params_arg, 0, &mut output),
                code,
                name,
            );
            assert!(output.is_null());
        }
        check_error(
            aimux_call(0, op.as_ptr(), params.as_ptr(), 0, ptr::null_mut()),
            200,
            "NullPointer",
        );
        let mut output = ptr::dangling_mut::<c_char>();
        check_error(
            aimux_call(0, op.as_ptr(), params.as_ptr(), u64::MAX, &mut output),
            203,
            "InvalidHandle",
        );
        assert!(output.is_null());
    }
}

#[test]
fn unknown_op_is_a_return_error() {
    let op = CString::new("not.a.registered.op").unwrap();
    let params = CString::new("{}").unwrap();
    let mut output = ptr::dangling_mut::<c_char>();
    unsafe {
        check_error(
            aimux_call(0, op.as_ptr(), params.as_ptr(), 0, &mut output),
            300,
            "InvalidOp",
        );
    }
    assert!(output.is_null());
}

extern "C" fn count_callback(_: *const c_char, context: *mut c_void) {
    unsafe { *context.cast::<usize>() += 1 };
}

#[test]
fn rejected_stream_has_no_callback() {
    let op = CString::new("not.a.registered.op").unwrap();
    let params = CString::new("{}").unwrap();
    let mut count = 0_usize;
    unsafe {
        check_error(
            aimux_stream(
                0,
                op.as_ptr(),
                params.as_ptr(),
                0,
                Some(count_callback),
                (&mut count as *mut usize).cast(),
            ),
            300,
            "InvalidOp",
        );
        check_error(
            aimux_stream(0, op.as_ptr(), params.as_ptr(), 0, None, ptr::null_mut()),
            200,
            "NullPointer",
        );
    }
    assert_eq!(count, 0);
}

#[test]
fn push_validates_before_reading_and_accepts_null_empty_input() {
    unsafe {
        check_error(aimux_operation_push(0, ptr::null(), 1), 200, "NullPointer");
        check_error(
            aimux_operation_push(0, ptr::dangling(), usize::MAX),
            305,
            "ResourceLimit",
        );
        check_error(
            aimux_operation_push(0, usize::MAX as *const u8, 1),
            305,
            "ResourceLimit",
        );
        check_error(
            aimux_operation_push(0, ptr::dangling(), 1024 * 1024 + 1),
            305,
            "ResourceLimit",
        );
        // NULL+0 reaches handle validation rather than causing NullPointer/UB.
        check_error(
            aimux_operation_push(0, ptr::null(), 0),
            203,
            "InvalidHandle",
        );
    }
}

#[test]
fn poll_clears_both_outputs_including_partial_null_arguments() {
    unsafe {
        let mut part = ptr::dangling_mut::<c_char>();
        let mut state = 99;
        check_error(
            aimux_operation_next_part(0, 0, &mut part, &mut state),
            203,
            "InvalidHandle",
        );
        assert!(part.is_null());
        assert_eq!(state, 0);
        state = 99;
        check_error(
            aimux_operation_next_part(0, 0, ptr::null_mut(), &mut state),
            200,
            "NullPointer",
        );
        assert_eq!(state, 0);
        part = ptr::dangling_mut();
        check_error(
            aimux_operation_next_part(0, 0, &mut part, ptr::null_mut()),
            200,
            "NullPointer",
        );
        assert!(part.is_null());
    }
}

#[test]
fn abort_and_drop_use_the_c_owner() {
    let registry = HandleRegistry::global();
    let signal = Arc::new(AbortSignal::new());
    let handle = registry
        .insert(Owner::CProcess, Entry::Abort(signal.clone()))
        .ok()
        .unwrap();
    unsafe {
        aimux_abort(handle.0);
        aimux_abort(handle.0);
    }
    assert!(signal.is_aborted());
    unsafe {
        aimux_drop(handle.0);
        aimux_drop(handle.0);
    }
    assert!(
        registry
            .get(Owner::CProcess, handle, &[HandleKind::Abort])
            .is_err()
    );
    let signal = Arc::new(AbortSignal::new());
    let owner = Owner::new_node().unwrap();
    let foreign = registry
        .insert(owner, Entry::Abort(signal.clone()))
        .ok()
        .unwrap();
    unsafe {
        aimux_abort(foreign.0);
        aimux_drop(foreign.0);
    }
    assert!(!signal.is_aborted());
    assert!(
        registry
            .get(owner, HandleId(foreign.0), &[HandleKind::Abort])
            .is_ok()
    );
    drop(registry.remove(owner, foreign));
}

#[tokio::test]
async fn async_caller_is_rejected_before_nested_runtime_entry() {
    let op = CString::new("runtime.describe").unwrap();
    let params = CString::new("{}").unwrap();
    let mut output = ptr::dangling_mut::<c_char>();
    unsafe {
        check_error(
            aimux_call(0, op.as_ptr(), params.as_ptr(), 0, &mut output),
            204,
            "ReentrantCall",
        );
    }
    assert!(output.is_null());
}

#[test]
fn abort_argument_uses_the_secondary_owner_resolver() {
    let registry = HandleRegistry::global();
    let owner = Owner::new_python().unwrap();
    let foreign = registry
        .insert(owner, Entry::Abort(Arc::new(AbortSignal::new())))
        .unwrap();
    let wrong = registry
        .insert(
            Owner::CProcess,
            Entry::Registry(Arc::new(
                aimux_core::provider_registry::create_provider_registry(
                    Default::default(),
                    Default::default(),
                ),
            )),
        )
        .unwrap();
    let mut envelopes = Vec::new();
    for abort in [foreign.0, u64::MAX, wrong.0] {
        let mut output = ptr::dangling_mut();
        let error =
            unsafe { aimux_call(0, c"not.an.op".as_ptr(), c"{}".as_ptr(), abort, &mut output) };
        assert!(output.is_null());
        assert_eq!(aimux_error_code(error), 203);
        let json = unsafe { take_string(aimux_error_json(error)) };
        envelopes.push(serde_json::from_str::<Value>(&json).unwrap());
        aimux_error_free(error);
    }
    assert_eq!(envelopes[0], envelopes[1]);
    assert_eq!(envelopes[2]["error"]["actual"], "Registry");
    drop(registry.remove(owner, foreign));
    unsafe { aimux_drop(wrong.0) };
}
