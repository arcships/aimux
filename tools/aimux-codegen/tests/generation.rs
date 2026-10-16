use aimux_codegen::{Artifacts, canonical_json, check, generate, write};
use jsonschema::{Draft, Registry, Resource};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
fn cli_generates_and_checks_the_runtime_manifest_in_a_selected_root() {
    let root = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_aimux-codegen");
    let generated = std::process::Command::new(binary)
        .arg("--root")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let checked = std::process::Command::new(binary)
        .arg("--root")
        .arg(root.path())
        .arg("--check")
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    for header in [
        "aimux-ffi/include/aimux.h",
        "bindings/flutter/native/aimux_ops.h",
    ] {
        fs::remove_file(root.path().join(header)).unwrap();
        let stale = std::process::Command::new(binary)
            .arg("--root")
            .arg(root.path())
            .arg("--check")
            .output()
            .unwrap();
        assert!(!stale.status.success());
        assert!(String::from_utf8_lossy(&stale.stderr).contains(&format!("missing: {header}")));
        assert!(!root.path().join(header).exists());
    }
    assert!(
        !std::process::Command::new(binary)
            .arg("--unknown")
            .status()
            .unwrap()
            .success()
    );
}

fn fixture() -> Value {
    let mut value = json!({"abiVersion": 65536, "opsVersion": "1.0", "manifestRevision": "test-1",
        "handleKinds": ["Language", "Operation"],
        "errorCodes": [
            {"name": "ApiCall", "kind": "aimux", "code": 1},
            {"name": "Io", "kind": "recording", "code": 100},
            {"name": "InvalidHandle", "kind": "binding", "code": 203},
            {"name": "InvalidOp", "kind": "protocol", "code": 300}],
        "ops": [
            {"name": "text.generateOpenAI", "mode": "call", "allowedHandleKinds": ["Language"], "requestType": "GenerateTextOptions"},
            {"name": "runtime.describe", "mode": "call", "allowedHandleKinds": [], "requestType": "EmptyParams"},
            {"name": "operation.push", "mode": "binaryPush", "allowedHandleKinds": ["Operation"], "requestType": "Bytes"},
            {"name": "operation.next", "mode": "poll", "allowedHandleKinds": ["Operation"], "requestType": "PollParams"},
            {"name": "text.stream", "mode": "stream", "allowedHandleKinds": ["Language"], "requestType": "GenerateTextOptions"},
            {"name": "handle.drop", "mode": "call", "allowedHandleKinds": ["Language", "Operation"], "requestType": "EmptyParams"}]});
    value["types"] = json!(
        aimux_ops::manifest::manifest()["types"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|ty| ["HandleId", "ErrorEnvelope", "StreamFrame"]
                .contains(&ty["id"].as_str().unwrap()))
            .cloned()
            .collect::<Vec<_>>()
    );
    value
}

fn digest(value: &Value) -> String {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
    }
    if let Some(ops) = value.get_mut("ops").and_then(Value::as_array_mut) {
        ops.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }
    format!("{:x}", Sha256::digest(canonical_json(&value).unwrap()))
}

fn artifacts() -> Artifacts {
    let manifest = fixture();
    generate(&manifest, &digest(&manifest)).unwrap()
}

fn schema(artifacts: &Artifacts, name: &str) -> Value {
    serde_json::from_slice(&artifacts[Path::new(&format!("schema/ops/{name}.json"))]).unwrap()
}

fn validator(artifacts: &Artifacts, name: &str) -> jsonschema::Validator {
    let mut registry = Registry::new();
    for (path, bytes) in artifacts {
        if path.starts_with("schema/ops") {
            let document: Value = serde_json::from_slice(bytes).unwrap();
            let id = document["$id"].as_str().unwrap().to_owned();
            registry = registry.add(id, Resource::from_contents(document)).unwrap();
        }
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .with_registry(&registry)
        .build(&schema(artifacts, name))
        .unwrap()
}

#[test]
fn canonical_digest_sorts_nested_objects_without_reordering_arrays() {
    let first: Value = serde_json::from_str(r#"{"z":{"b":2,"a":1},"a":[2,1]}"#).unwrap();
    let second = json!({"a": [2, 1], "z": {"a": 1, "b": 2}});
    assert_eq!(
        canonical_json(&first).unwrap(),
        br#"{"a":[2,1],"z":{"a":1,"b":2}}"#
    );
    assert_eq!(digest(&first), digest(&second));
    assert_ne!(
        digest(&first),
        digest(&json!({"a": [1,2], "z": {"a": 1, "b": 2}}))
    );
    assert_eq!(
        canonical_json(&json!({"z": -0.0, "a": u64::MAX})).unwrap(),
        br#"{"a":18446744073709551615,"z":-0.0}"#
    );
    assert!(
        generate(&fixture(), "wrong digest")
            .unwrap_err()
            .to_string()
            .contains("manifest digest drift")
    );
}

#[test]
fn deterministic_complete_outputs_cover_every_language_and_abi_primitive() {
    let outputs = artifacts();
    assert_eq!(outputs, artifacts());
    assert_eq!(outputs.len(), 18);
    assert_eq!(
        outputs[Path::new("aimux-ffi/include/aimux.h")],
        outputs[Path::new("bindings/flutter/native/aimux_ops.h")]
    );
    let inventory: Value =
        serde_json::from_slice(&outputs[Path::new("tools/aimux-codegen/generated-files.json")])
            .unwrap();
    assert_eq!(
        inventory["files"],
        json!(outputs.keys().collect::<Vec<_>>())
    );
    let header =
        String::from_utf8(outputs[Path::new("aimux-ffi/include/aimux.h")].clone()).unwrap();
    for primitive in [
        "aimux_abi_version",
        "aimux_manifest_json",
        "aimux_call",
        "aimux_stream",
        "aimux_operation_push",
        "aimux_operation_next_part",
        "aimux_abort",
        "aimux_drop",
        "aimux_free_string",
        "aimux_error_free",
        "aimux_error_code",
        "aimux_error_message",
        "aimux_error_json",
        "aimux_handle_finalizer",
    ] {
        assert!(
            header.contains(&format!("{primitive}(")),
            "missing {primitive}"
        );
    }
    assert!(header.contains("AIMUX_ERROR_BINDING_INVALID_HANDLE = 203"));
    assert!(header.contains("AIMUX_ERROR_PROTOCOL_INVALID_OP = 300"));
    assert!(header.contains("AIMUX_OP_TEXT_GENERATE_OPEN_AI \"text.generateOpenAI\""));
    assert!(header.contains("borrowed until the callback returns"));
    for language in ["go", "java", "kotlin", "swift", "flutter", "node", "python"] {
        let (_, bytes) = outputs
            .iter()
            .find(|(path, _)| {
                path.starts_with(format!("bindings/{language}"))
                    && path.to_string_lossy().contains("generated/")
            })
            .unwrap();
        let text = std::str::from_utf8(bytes).unwrap();
        for op in fixture()["ops"].as_array().unwrap() {
            assert!(text.contains(op["name"].as_str().unwrap()));
        }
        assert!(text.contains(&digest(&fixture())));
    }
}

#[test]
fn check_detects_missing_stale_and_changed_files_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let outputs = artifacts();
    write(dir.path(), &outputs).unwrap();
    check(dir.path(), &outputs).unwrap();
    fs::remove_file(dir.path().join("bindings/node/generated/ops.ts")).unwrap();
    fs::write(dir.path().join("schema/ops/stale.json"), b"{}").unwrap();
    fs::write(dir.path().join("bindings/go/generated/ops.go"), b"changed").unwrap();
    let packaged = dir.path().join("bindings/flutter/native/aimux_ops.h");
    fs::write(&packaged, b"changed").unwrap();
    let error = check(dir.path(), &outputs).unwrap_err().to_string();
    assert!(error.contains("missing: bindings/node/generated/ops.ts"));
    assert!(error.contains("stale: schema/ops/stale.json"));
    assert!(error.contains("differs: bindings/go/generated/ops.go"));
    assert!(error.contains("differs: bindings/flutter/native/aimux_ops.h"));
    assert_eq!(fs::read(packaged).unwrap(), b"changed");
    assert_eq!(
        fs::read(dir.path().join("bindings/go/generated/ops.go")).unwrap(),
        b"changed"
    );
    write(dir.path(), &outputs).unwrap();
    assert!(!dir.path().join("schema/ops/stale.json").exists());
    check(dir.path(), &outputs).unwrap();
}

#[test]
fn digest_and_previous_constant_drift_are_detected() {
    let dir = tempfile::tempdir().unwrap();
    let outputs = artifacts();
    write(dir.path(), &outputs).unwrap();
    fs::write(
        dir.path().join("tools/aimux-codegen/generated-files.json"),
        b"{\"manifestDigest\":\"wrong\"}",
    )
    .unwrap();
    fs::write(
        dir.path().join("bindings/python/generated/ops-old.py"),
        b"OLD = 1",
    )
    .unwrap();
    let error = check(dir.path(), &outputs).unwrap_err().to_string();
    assert!(error.contains("differs: tools/aimux-codegen/generated-files.json"));
    assert!(error.contains("stale: bindings/python/generated/ops-old.py"));
}

#[test]
fn writes_preserve_unowned_binding_types_and_reject_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let outputs = artifacts();
    write(dir.path(), &outputs).unwrap();
    let types = dir.path().join("bindings/node/generated/types.ts");
    fs::write(&types, b"export type OwnedElsewhere = string;").unwrap();
    let native = dir.path().join("bindings/flutter/native/unowned.h");
    fs::write(&native, b"binding-owned").unwrap();
    let cache = dir.path().join("bindings/python/generated/__pycache__");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("ops.pyc"), b"compiled cache").unwrap();
    write(dir.path(), &outputs).unwrap();
    check(dir.path(), &outputs).unwrap();
    assert!(types.exists());
    assert_eq!(fs::read(native).unwrap(), b"binding-owned");
    assert!(cache.join("ops.pyc").exists());
    let mut invalid = outputs.clone();
    invalid.insert("schema/ops/../../README.md".into(), b"bad".to_vec());
    assert!(write(dir.path(), &invalid).is_err());
}

#[test]
fn schemas_resolve_locally_and_validate_as_draft_2020_12() {
    let outputs = artifacts();
    for path in outputs.keys().filter(|path| path.starts_with("schema/ops")) {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let document = schema(&outputs, name);
        assert_eq!(
            document["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert!(jsonschema::meta::is_valid(&document));
        let _validator = validator(&outputs, name);
    }
}

#[test]
fn request_controls_are_closed_case_sensitive_and_keep_provider_json_open() {
    let outputs = artifacts();
    let requests = validator(&outputs, "request");
    let mut request = json!({"handle": "9", "op": "text.generateOpenAI", "abort": "0",
        "params": {"providerOptions": {"vendor": {"future": true}}, "metadata": {"unknown": 1}}});
    assert!(requests.is_valid(&request));
    request["extra"] = json!(1);
    assert!(!requests.is_valid(&request));
    request.as_object_mut().unwrap().remove("extra");
    request["op"] = json!("text.generateOpenAi");
    assert!(!requests.is_valid(&request));
    assert!(requests.is_valid(&json!({"handle": "0", "op": "runtime.describe", "params": {}})));
    assert!(!requests.is_valid(&json!({"handle": "9", "op": "runtime.describe", "params": {}})));
    assert!(!requests.is_valid(&json!({"handle": "0", "op": "text.stream", "params": {}})));
    assert!(requests.is_valid(&json!({"handle": "0", "op": "handle.drop", "params": {}})));
    assert!(
        !requests
            .is_valid(&json!({"handle": "1", "op": "operation.push", "params": {}, "abort": "0"}))
    );
    assert!(
        validator(&outputs, "provider-options").is_valid(&json!({"future": {"nested": [1, null]}}))
    );
}

#[test]
fn handles_cover_u64_strings_and_reject_precision_loss_and_overflow() {
    let validator = validator(&artifacts(), "handle-id");
    for handle in [
        "0",
        "1",
        "01",
        "00018446744073709551615",
        "9999999999999999999",
        "10000000000000000000",
        "18446744073709551614",
        "18446744073709551615",
    ] {
        assert!(validator.is_valid(&json!(handle)), "rejected {handle}");
    }
    for handle in [
        "",
        "1\n",
        "1\r",
        "١",
        "-1",
        "+1",
        "1.0",
        "1e2",
        "18446744073709551616",
        "20000000000000000000",
        "184467440737095516150",
    ] {
        assert!(!validator.is_valid(&json!(handle)), "accepted {handle}");
    }
    assert!(!validator.is_valid(&json!(1)));
}

#[test]
fn frame_and_error_envelope_use_contract_discriminators() {
    let outputs = artifacts();
    let frames = validator(&outputs, "stream-frame");
    let error = json!({"kind": "aimux", "code": 1, "message": "failure", "error": {"name": "AI_APICallError", "isRetryable": true}, "retryAfterMs": 0});
    assert!(
        frames.is_valid(&json!({"type": "end", "seq": "2", "outcome": "error", "error": error}))
    );
    assert!(frames.is_valid(&json!({"type": "event", "seq": "1", "event": {"vendor": null}})));
    assert!(frames.is_valid(&json!({"type": "end", "seq": "2", "outcome": "cancelled"})));
    assert!(
        !frames.is_valid(&json!({"type": "end", "seq": "2", "outcome": "success", "extra": true}))
    );
    assert!(!frames.is_valid(&json!({"type": "unknown", "seq": "1"})));
    let errors = validator(&outputs, "error-envelope");
    assert!(errors.is_valid(&error));
    let mut wrong = error.clone();
    wrong["retryable"] = json!(true);
    assert!(!errors.is_valid(&wrong));
    wrong = error.clone();
    wrong["retryAfterMs"] = Value::Null;
    assert!(!errors.is_valid(&wrong));
}

#[test]
fn malformed_manifest_identifiers_kinds_modes_and_error_codes_fail() {
    for mutation in ["collision", "kind", "mode", "identifier", "code", "domain"] {
        let mut manifest = fixture();
        match mutation {
            "collision" => manifest["ops"][1]["name"] = json!("text.generateOpenAI"),
            "kind" => manifest["ops"][0]["allowedHandleKinds"] = json!(["Unknown"]),
            "mode" => manifest["ops"][0]["mode"] = json!("futureMode"),
            "identifier" => manifest["ops"][0]["name"] = json!("text.\"injection"),
            "code" => manifest["errorCodes"][0]["code"] = json!(99),
            "domain" => manifest["errorCodes"][0]["kind"] = json!("binding"),
            _ => unreachable!(),
        }
        assert!(
            generate(&manifest, &digest(&manifest)).is_err(),
            "accepted {mutation}"
        );
    }
    let mut collision = fixture();
    collision["ops"][0]["name"] = json!("text.fooBar");
    collision["ops"][1]["name"] = json!("text.foo_bar");
    assert!(generate(&collision, &digest(&collision)).is_err());
    let mut version = fixture();
    version["opsVersion"] = json!("${injected}");
    assert!(generate(&version, &digest(&version)).is_err());
}

#[test]
fn real_manifest_control_schemas_are_emitted_and_enforced() {
    let value = aimux_ops::manifest::manifest();
    let outputs = generate(&value, &aimux_ops::manifest::manifest_digest()).unwrap();
    for (id, name) in [
        ("HandleId", "handle-id"),
        ("ErrorEnvelope", "error-envelope"),
        ("StreamFrame", "stream-frame"),
    ] {
        let expected = &value["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ty| ty["id"] == id)
            .unwrap()["schema"];
        let mut emitted = schema(&outputs, name);
        let object = emitted.as_object_mut().unwrap();
        object.remove("$id");
        object.remove("$schema");
        object.remove("$comment");
        assert_eq!(&emitted, expected, "{id}");
    }
    let handles = validator(&outputs, "handle-id");
    for wire in ["01", "00018446744073709551615"] {
        let value = json!(wire);
        let _: aimux_ops::handle::HandleId = serde_json::from_value(value.clone()).unwrap();
        assert!(handles.is_valid(&value));
    }
    let unknown = json!({"kind":"aimux", "code":21, "message":"future error",
        "error":{"name":"AI_FutureError", "data":{"secret":null}}});
    let envelope: aimux_ops::envelope::ErrorEnvelope =
        serde_json::from_value(unknown.clone()).unwrap();
    assert!(validator(&outputs, "error-envelope").is_valid(&unknown));
    let frame = aimux_ops::frame::StreamFrame::End {
        seq: "01".into(),
        outcome: aimux_ops::frame::Outcome::Error,
        error: Some(envelope),
    };
    assert!(validator(&outputs, "stream-frame").is_valid(&serde_json::to_value(frame).unwrap()));
    for ty in value["types"].as_array().unwrap() {
        if ty.get("schema").is_some() {
            assert!(
                outputs.values().any(|bytes| {
                    serde_json::from_slice::<Value>(bytes)
                        .ok()
                        .is_some_and(|document| {
                            let mut document = document;
                            if let Some(object) = document.as_object_mut() {
                                object.remove("$schema");
                                object.remove("$id");
                                if ty["schema"].get("$comment").is_none() {
                                    object.remove("$comment");
                                }
                            }
                            document == ty["schema"]
                        })
                }),
                "missing schema {}",
                ty["id"]
            );
        }
    }
    let requests = validator(&outputs, "request");
    assert!(
        requests.is_valid(&json!({"handle":"1","op":"operation.next","params":{"timeoutMs":0}}))
    );
    assert!(
        !requests.is_valid(&json!({"handle":"1","op":"operation.next","params":{"extra":true}}))
    );
    for path in outputs.keys().filter(|p| p.starts_with("schema/ops")) {
        let name = path.file_stem().unwrap().to_str().unwrap();
        assert!(
            jsonschema::meta::is_valid(&schema(&outputs, name)),
            "{name}"
        );
        let _ = validator(&outputs, name);
    }
}

#[test]
fn manifest_digest_ignores_its_own_field_and_operation_order() {
    let mut value = fixture();
    let expected = generate(&value, &digest(&value)).unwrap();
    value["digest"] = json!("excluded from digest");
    value["ops"].as_array_mut().unwrap().reverse();
    assert_eq!(generate(&value, &digest(&fixture())).unwrap(), expected);
}
