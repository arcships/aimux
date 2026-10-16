//! Frozen RFC-0039 §3.2 inventory, independent of the implementation table.

use std::collections::{BTreeMap, BTreeSet};

use aimux_ops::dispatch::descriptor;
use aimux_ops::manifest::{manifest, manifest_digest};
use aimux_ops::op::{ABI_VERSION, OPS_VERSION};
use serde_json::Value;
use sha2::{Digest, Sha256};

const RFC_OPS: &[(&str, &str, &[&str])] = &[
    ("decision.decide", "call", &["Decision"]),
    ("decision.capabilities", "call", &["Decision"]),
    ("runtime.describe", "call", &[]),
    ("runtime.configure", "call", &[]),
    ("provider.create", "call", &[]),
    ("provider.default", "call", &[]),
    ("registry.create", "call", &[]),
    ("registry.provider", "call", &["Registry"]),
    ("provider.model", "call", &["Provider"]),
    ("provider.invoke", "call", &["Provider"]),
    ("provider.wrap", "call", &["Provider"]),
    ("provider.custom", "call", &[]),
    ("model.resolve", "call", &["Registry"]),
    (
        "model.info",
        "call",
        &[
            "Language",
            "Embedding",
            "Image",
            "Speech",
            "Transcription",
            "Reranking",
            "Video",
            "Search",
            "Decision",
        ],
    ),
    (
        "model.wrap",
        "call",
        &[
            "Language",
            "Embedding",
            "Image",
            "Speech",
            "Transcription",
            "Reranking",
            "Video",
            "Search",
            "Decision",
        ],
    ),
    ("composite.router", "call", &[]),
    ("composite.moa", "call", &[]),
    ("text.generate", "call", &["Language"]),
    ("text.generateObject", "call", &["Language"]),
    ("text.consume", "call", &["Language"]),
    ("text.generateOpenAI", "call", &["Language"]),
    ("text.stream", "stream", &["Language"]),
    ("text.streamOpenAI", "stream", &["Language"]),
    ("embedding.embed", "call", &["Embedding"]),
    ("embedding.embedMany", "call", &["Embedding"]),
    ("image.generate", "call", &["Image"]),
    ("speech.generate", "call", &["Speech"]),
    ("transcription.generate", "call", &["Transcription"]),
    ("reranking.rerank", "call", &["Reranking"]),
    ("video.generate", "call", &["Video"]),
    ("search.search", "call", &["Search"]),
    ("provider.files", "call", &["Provider"]),
    ("files.upload", "call", &["Files"]),
    ("transcription.start", "call", &["Transcription"]),
    ("operation.push", "binaryPush", &["Operation"]),
    ("operation.inputDone", "call", &["Operation"]),
    ("operation.next", "poll", &["Operation"]),
    ("operation.status", "call", &["Operation"]),
    ("operation.cancel", "call", &["Operation"]),
    ("abort.create", "call", &[]),
    ("abort.signal", "call", &["Abort"]),
    ("transport.create", "call", &[]),
    ("credential.create", "call", &[]),
    ("middleware.create", "call", &[]),
    ("trace.create", "call", &[]),
    ("trace.aggregate", "call", &["TraceStore"]),
    ("trace.sessionChain", "call", &["TraceStore"]),
    ("trace.sessionTrajectory", "call", &["TraceStore"]),
    ("trace.export", "call", &["TraceStore"]),
    ("trace.clear", "call", &["TraceStore"]),
    ("recorder.create", "call", &[]),
    ("recorder.attach", "call", &["Recorder"]),
    ("recorder.stop", "call", &["Recorder"]),
    ("recorder.flush", "call", &["Recorder"]),
    ("recorder.stats", "call", &["Recorder"]),
    ("recorder.export", "call", &["Recorder"]),
    ("sessionStore.create", "call", &[]),
    ("sessionStore.attach", "call", &["SessionStore"]),
    ("sessionStore.list", "call", &["SessionStore"]),
    ("sessionStore.calls", "call", &["SessionStore"]),
    ("replay.create", "call", &[]),
    ("replay.capabilities", "call", &["ReplaySession"]),
    ("replay.inspect", "call", &["ReplaySession"]),
    ("replay.dryRun", "call", &["ReplaySession"]),
    ("replay.live", "call", &["ReplaySession"]),
    ("catalogue.fetch", "call", &[]),
    ("toolRepair.context", "call", &[]),
    ("toolRepair.apply", "call", &[]),
    ("toolRepair.applyResult", "call", &[]),
    ("output.toOpenAI", "call", &["Language"]),
    ("auth.codexRefresh", "call", &[]),
    (
        "handle.drop",
        "call",
        &[
            "Provider",
            "Registry",
            "Language",
            "Embedding",
            "Image",
            "Speech",
            "Transcription",
            "Reranking",
            "Video",
            "Search",
            "Decision",
            "Files",
            "Operation",
            "Abort",
            "Fetch",
            "WebSocket",
            "Download",
            "CredentialResolver",
            "Middleware",
            "TraceStore",
            "Recorder",
            "SessionStore",
            "ReplaySession",
        ],
    ),
];

fn strings(value: &Value) -> BTreeSet<&str> {
    let values = value.as_array().expect("manifest string array");
    let result: BTreeSet<_> = values.iter().map(|v| v.as_str().expect("string")).collect();
    assert_eq!(
        values.len(),
        result.len(),
        "duplicate array entries: {value}"
    );
    result
}

#[test]
fn rfc_inventory_has_exact_names_modes_and_handle_scopes() {
    let manifest = manifest();
    assert_eq!(manifest["abiVersion"], ABI_VERSION);
    assert_eq!(manifest["opsVersion"], OPS_VERSION);
    assert!(!manifest["manifestRevision"].is_null());
    let ops = manifest["ops"].as_array().expect("manifest.ops");
    let mut actual = BTreeMap::new();
    for op in ops {
        let name = op["name"].as_str().expect("op.name");
        assert!(actual.insert(name, op).is_none(), "duplicate op: {name}");
    }
    let expected: BTreeSet<_> = RFC_OPS.iter().map(|(name, _, _)| *name).collect();
    assert_eq!(expected.len(), RFC_OPS.len(), "duplicate RFC test fixture");
    assert_eq!(
        actual.keys().copied().collect::<BTreeSet<_>>(),
        expected,
        "missing RFC ops or extra aliases/capabilities"
    );
    let expected_kinds: BTreeSet<_> = RFC_OPS
        .iter()
        .find(|(name, _, _)| *name == "handle.drop")
        .unwrap()
        .2
        .iter()
        .copied()
        .collect();
    assert_eq!(
        strings(&manifest["handleKinds"]),
        expected_kinds,
        "HandleKind inventory"
    );
    for &(name, mode, kinds) in RFC_OPS {
        let op = actual[name];
        assert_eq!(op["mode"], mode, "{name} mode");
        assert_eq!(
            strings(&op["allowedHandleKinds"]),
            kinds.iter().copied().collect(),
            "{name} scope"
        );
        let registered = descriptor(name).unwrap_or_else(|| panic!("{name} has no dispatcher"));
        assert_eq!(registered.name, name);
        let registered_kinds = serde_json::to_value(registered.allowed_handle_kinds).unwrap();
        assert_eq!(
            strings(&registered_kinds),
            strings(&op["allowedHandleKinds"]),
            "{name}: manifest and dispatcher disagree"
        );
        assert_eq!(
            (registered.request_type.id)().into_owned(),
            op["requestType"].as_str().unwrap()
        );
        assert_eq!(
            (registered.result_type.id)().into_owned(),
            op["resultType"].as_str().unwrap()
        );
        assert_eq!(
            registered.event_type.map(|ty| (ty.id)().into_owned()),
            op.get("eventType")
                .and_then(Value::as_str)
                .map(str::to_owned)
        );
    }
}

#[test]
fn names_are_case_sensitive_and_have_no_implicit_aliases() {
    for &(name, _, _) in RFC_OPS {
        assert!(descriptor(name).is_some(), "missing {name}");
        let first_uppercase = format!("{}{}", name[..1].to_ascii_uppercase(), &name[1..]);
        for wrong in [
            name.to_ascii_uppercase(),
            first_uppercase,
            format!(" {name}"),
            format!("{name} "),
            name.replace('.', "_"),
        ] {
            assert!(descriptor(&wrong).is_none(), "unexpected alias: {wrong}");
        }
    }
    for unknown in [
        "",
        "text.Generate",
        "text.streamopenai",
        "Text.generate",
        "not.an.op",
    ] {
        assert!(descriptor(unknown).is_none(), "unexpected op: {unknown}");
    }
}

// Serialize recursively instead of relying on serde_json::Map ordering: the
// workspace enables preserve_order, which must not affect canonical hashing.
fn canonical(value: &Value, output: &mut String) {
    match value {
        Value::Object(object) => {
            output.push('{');
            for (index, (key, value)) in object
                .iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .enumerate()
            {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).unwrap());
                output.push(':');
                canonical(value, output);
            }
            output.push('}');
        }
        Value::Array(array) => {
            output.push('[');
            for (index, value) in array.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                canonical(value, output);
            }
            output.push(']');
        }
        _ => output.push_str(&serde_json::to_string(value).unwrap()),
    }
}

#[test]
fn digest_is_stable_and_hashes_the_complete_canonical_manifest() {
    let first = manifest();
    let digest = manifest_digest();
    let mut bytes = String::new();
    assert_eq!(first["digest"], digest);
    let mut unsigned = first.clone();
    unsigned.as_object_mut().unwrap().remove("digest");
    canonical(&unsigned, &mut bytes);
    assert_eq!(digest, format!("{:x}", Sha256::digest(bytes.as_bytes())));
    for _ in 0..8 {
        assert_eq!(manifest(), first);
        assert_eq!(manifest_digest(), digest);
    }
}

#[test]
fn every_descriptor_type_reference_names_an_existing_type() {
    let manifest = manifest();
    for op in manifest["ops"].as_array().unwrap() {
        for field in ["requestType", "resultType", "eventType", "terminalType"] {
            if let Some(id) = op[field].as_str() {
                let ty = manifest["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|ty| ty["id"] == id)
                    .unwrap();
                let direction = if field == "requestType" {
                    "input"
                } else {
                    "output"
                };
                let name = ty[direction].as_str().unwrap();
                assert!(
                    manifest["typeDefs"][direction].get(name).is_some(),
                    "{} {field} {id}",
                    op["name"]
                );
            }
        }
    }
}

#[test]
fn metadata_and_owned_control_schemas_are_complete() {
    let manifest = manifest();
    let operations = manifest["ops"].as_array().unwrap();
    assert!(
        operations
            .windows(2)
            .all(|pair| pair[0]["name"].as_str() < pair[1]["name"].as_str())
    );
    let types: BTreeMap<_, _> = manifest["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ty| (ty["id"].as_str().unwrap(), ty))
        .collect();
    assert_eq!(types.len(), manifest["types"].as_array().unwrap().len());
    for op in operations {
        for field in [
            "scope",
            "sourceSymbolInventory",
            "terminalType",
            "runtimeHandleFields",
            "requiredCapabilities",
            "cancellation",
            "resultOwnership",
            "requestBytePolicy",
        ] {
            assert!(op.get(field).is_some(), "{} missing {field}", op["name"]);
        }
        assert!(op.get("cancellable").is_none());
        assert!(op.get("sourceSymbols").is_none());
        for field in ["requestType", "resultType", "eventType", "terminalType"] {
            if let Some(id) = op[field].as_str() {
                assert!(types.contains_key(id), "{} missing type {id}", op["name"]);
            }
        }
    }
    for ty in types.values() {
        for direction in ["input", "output"] {
            if let Some(name) = ty[direction].as_str() {
                assert!(manifest["typeDefs"][direction].get(name).is_some());
            }
        }
    }
    for (id, valid, invalid) in [
        (
            "EmptyParams",
            serde_json::json!({}),
            serde_json::json!({"unknown":true}),
        ),
        (
            "OperationNextParams",
            serde_json::json!({"timeoutMs":0}),
            serde_json::json!({"timeoutMs":-1}),
        ),
        (
            "OperationNextParams",
            serde_json::json!({}),
            serde_json::json!({"unknown":true}),
        ),
        (
            "TranscriptionStartParams",
            serde_json::json!({"providerOptions":{"vendor":{"open":true}}}),
            serde_json::json!({"unknown":true}),
        ),
        (
            "TranscriptionStartParams",
            serde_json::json!({"timeout":{"chunkMs":1}}),
            serde_json::json!({"timeout":{"unknown":true}}),
        ),
        (
            "CancelResult",
            serde_json::json!("alreadyTerminal"),
            serde_json::json!("success"),
        ),
        (
            "OperationPollResult",
            serde_json::json!({"type":"part","part":{"type":"transcript-delta","delta":"hi"}}),
            serde_json::json!({"type":"part"}),
        ),
    ] {
        let validator = type_validator(&manifest, id);
        assert!(validator.is_valid(&valid), "{id} rejects {valid}");
        assert!(!validator.is_valid(&invalid), "{id} admits {invalid}");
    }
    let codes = manifest["errorCodes"].as_array().unwrap();
    assert_eq!(
        codes.iter().find(|code| code["code"] == 3).unwrap()["aliases"],
        serde_json::json!([
            "InvalidResponseData",
            "ToolCallNotFoundForApproval",
            "ToolChoiceViolation"
        ])
    );
    assert_eq!(
        codes.iter().find(|code| code["code"] == 6).unwrap()["aliases"],
        serde_json::json!(["InvalidPrompt", "MissingToolResults"])
    );
}

#[test]
fn all_owned_types_publish_schemas_and_family_controls_are_strict() {
    let value = manifest();
    let types: BTreeMap<_, _> = value["types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ty| (ty["id"].as_str().unwrap(), ty))
        .collect();
    for ty in types.values() {
        for direction in ["input", "output"] {
            if let Some(name) = ty[direction].as_str() {
                assert!(value["typeDefs"][direction].get(name).is_some());
            }
        }
    }
    for (id, valid) in [
        (
            "runtime::RuntimeConfigureParams",
            serde_json::json!({"logging":{"level":"warn"},"proxy":{}}),
        ),
        (
            "provider::ProviderCreateParams",
            serde_json::json!({"package":"openai","settings":{"headers":{"x-custom":null},"fetch":"1"}}),
        ),
        (
            "registry::RegistryCreateParams",
            serde_json::json!({"entries":[{"key":"vendor","provider":"1"}]}),
        ),
        (
            "registry::ModelResolveParams",
            serde_json::json!({"modelRef":{"namespace":"ns","key":"key","method":"decision","modelId":"m"}}),
        ),
        (
            "composite::RouterParams",
            serde_json::json!({"children":["1"],"strategy":{"type":"rule"}}),
        ),
        (
            "composite::MoaParams",
            serde_json::json!({"references":[],"aggregator":"1"}),
        ),
        (
            "text::TextRequest",
            serde_json::json!({"prompt":"hello","options":{"providerOptions":{"vendor":{"open":true}}},"abort":null}),
        ),
        (
            "text::OpenAiTextRequest",
            serde_json::json!({"prompt":"hello","streamOptions":{"includeUsage":true}}),
        ),
        (
            "decision::DecisionDecideRequest",
            serde_json::json!({"options":{"state":"state","questions":{}},"abort":"18446744073709551615"}),
        ),
        (
            "files::UploadFileParams",
            serde_json::json!({"data":{"type":"text","text":"hello"},"mediaType":"text/plain","providerOptions":{"vendor":{"open":true}}}),
        ),
        (
            "handles::TransportParams",
            serde_json::json!({"kind":"fetch","proxy":{"http_url":null}}),
        ),
        (
            "handles::CredentialParams",
            serde_json::json!({"kind":"future"}),
        ),
        (
            "handles::MiddlewareParams",
            serde_json::json!({"kind":"trace","traceStore":"1","auditMode":"strict"}),
        ),
        (
            "trace::TraceCreateParams",
            serde_json::json!({"auditMode":"shared","capacity":1}),
        ),
        (
            "trace::TraceAggregateParams",
            serde_json::json!({"filter":{"sessionId":"s","sinceUnixMs":-1}}),
        ),
        (
            "session::SessionStoreCreateParams",
            serde_json::json!({"infer":true}),
        ),
        (
            "recording::CreateRecorderParams",
            serde_json::json!({"backend":{"type":"ring","capacity":1},"capturePolicy":"full"}),
        ),
        (
            "replay::ReplayCreateParams",
            serde_json::json!({"recordingsJsonl":""}),
        ),
        (
            "replay::ReplayLiveParams",
            serde_json::json!({"callId":"id","target":{"kind":"model","handle":"1"},"abort":null}),
        ),
        (
            "misc::CatalogueFetchRequest",
            serde_json::json!({"sourceUrl":null,"abort":null}),
        ),
        (
            "misc::CodexRefreshRequest",
            serde_json::json!({"refreshToken":"token","clientId":"client"}),
        ),
        (
            "misc::OpenAiOutputRequest",
            serde_json::json!({"result":{"steps":[{
                "callId":"call", "stepNumber":0, "model":{"provider":"test","modelId":"fallback"},
                "content":[{"type":"text","text":"hello"}], "finishReason":"stop", "usage":{},
                "performance":aimux_core::step_result::StepResultPerformance::default(),
                "response":{"id":"response","timestamp":"2026-01-01T00:00:00.000Z","modelId":"","messages":[]}
            }], "totalUsage":{}},"metadataNamespace":"openai"}),
        ),
    ] {
        let full_id = format!("aimux_ops::ops::{id}");
        let validator = type_validator(&value, &full_id);
        assert!(validator.is_valid(&valid), "{id} rejects {valid}");
        let mut unknown = valid.clone();
        unknown["unknown"] = serde_json::json!(true);
        assert!(
            !validator.is_valid(&unknown),
            "{id} admits unknown controls"
        );
        assert!(
            !validator.is_valid(&serde_json::json!([])),
            "{id} admits non-object"
        );
    }
    for (id, invalid) in [
        (
            "provider::ProviderCreateParams",
            serde_json::json!({"package":"openai","settings":{"unknown":true}}),
        ),
        (
            "registry::RegistryCreateParams",
            serde_json::json!({"entries":[{"key":"k","provider":"1","unknown":true}]}),
        ),
        (
            "composite::RouterParams",
            serde_json::json!({"children":["1"],"strategy":{"type":"rule","weights":[]}}),
        ),
        (
            "handles::TransportParams",
            serde_json::json!({"kind":"fetch","proxy":{"unknown":true}}),
        ),
        (
            "recording::CreateRecorderParams",
            serde_json::json!({"backend":{"type":"ring","unknown":true},"capturePolicy":"full"}),
        ),
        (
            "replay::ReplayLiveParams",
            serde_json::json!({"callId":"id","target":{"kind":"model","handle":"1","unknown":true}}),
        ),
        (
            "text::OpenAiTextRequest",
            serde_json::json!({"prompt":"hi","streamOptions":{"unknown":true}}),
        ),
        (
            "files::UploadFileParams",
            serde_json::json!({"data":{"type":"text","text":"hi","data":"aGk="},"mediaType":"text/plain"}),
        ),
        (
            "trace::TraceAggregateParams",
            serde_json::json!({"filter":{"unknown":true}}),
        ),
    ] {
        let full_id = format!("aimux_ops::ops::{id}");
        assert!(
            !type_validator(&value, &full_id).is_valid(&invalid),
            "{id} admits {invalid}"
        );
    }
    let handles = jsonschema::validator_for(&serde_json::json!({
        "typeDefs": value["typeDefs"], "$ref": "#/typeDefs/input/HandleId"
    }))
    .unwrap();
    assert!(handles.is_valid(&serde_json::json!("001")));
    assert!(!handles.is_valid(&serde_json::json!("18446744073709551616")));
    assert!(!handles.is_valid(&serde_json::json!(1)));
}

fn type_validator(manifest: &Value, id: &str) -> jsonschema::Validator {
    let ty = manifest["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ty| {
            let actual = ty["id"].as_str().unwrap();
            actual == id || actual.ends_with(&format!("::{id}"))
        })
        .unwrap_or_else(|| panic!("missing type {id}"));
    let direction = if ty.get("input").is_some() {
        "input"
    } else {
        "output"
    };
    let root = serde_json::json!({
        "typeDefs": manifest["typeDefs"],
        "$ref": format!("#/typeDefs/{}/{}", direction, ty[direction].as_str().unwrap())
    });
    jsonschema::validator_for(&root).unwrap()
}

#[test]
fn packages_list_every_provider_create_package_once() {
    let manifest = manifest();
    let packages: Vec<&str> = manifest["packages"]
        .as_array()
        .expect("manifest.packages")
        .iter()
        .map(|name| name.as_str().expect("package name"))
        .collect();
    assert!(packages.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(packages.contains(&"jev"));
    for name in aimux_providers::provider_names() {
        assert!(packages.contains(&name), "{name}");
    }
}
