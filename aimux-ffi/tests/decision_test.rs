mod common;
use aimux_ffi::*;
use common::*;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
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

#[test]
fn probability_source_survives_c_abi_round_trip() {
    for source in ["native", "logit_scoring", "model_estimate"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = c(&format!(
            "http://{}/v1/systemone",
            listener.local_addr().unwrap()
        ));
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut size = 0;
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    size = value.trim().parse::<usize>().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; size]).unwrap();
            let body = r#"{"model":"jev-1.13","answers":{"q":{"type":"noul","noul":0.89}}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let mut handle = 0;
        ok(
            aimux_jev_decision_new_with_probability_source(
                c("test-key").as_ptr(),
                c("jev-1.13").as_ptr(),
                endpoint.as_ptr(),
                c(source).as_ptr(),
                &mut handle,
            ),
            "create with provenance",
        );
        let request = c(
            r#"{"state":"text","questions":[{"id":"q","type":"boolean","instructions":"test"}],"max_retries":0,"timeout":{"total_ms":2000}}"#,
        );
        let mut out = ptr::null_mut();
        ok(
            aimux_decide(handle, request.as_ptr(), &mut out),
            "decide with provenance",
        );
        let result: serde_json::Value = serde_json::from_str(&take(out)).unwrap();
        assert_eq!(result["probability_source"], source);
        aimux_drop_handle(handle);
        server.join().unwrap();
    }
    let mut handle = 0;
    let (code, _) = expect_aimux_error(
        aimux_jev_decision_new_with_probability_source(
            c("test-key").as_ptr(),
            c("jev-1.13").as_ptr(),
            ptr::null(),
            c("unknown").as_ptr(),
            &mut handle,
        ),
        "unknown provenance",
    );
    assert_eq!(code, AIMUX_E_INVALID_ARGUMENT);
    assert_eq!(handle, 0);
}
