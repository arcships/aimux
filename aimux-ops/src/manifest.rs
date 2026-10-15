mod controls;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{ABI_VERSION, HandleKind, OPS_VERSION};

/// Describe all registered operations and their wire metadata.
#[must_use]
pub fn manifest() -> Value {
    let ops: Vec<_> = crate::dispatch::registrations()
        .iter()
        .map(|(descriptor, _)| json!(descriptor))
        .collect();
    let mut error_codes = Vec::new();
    for (kind, entries) in ERROR_CODES {
        for (code, name) in *entries {
            error_codes.push(
                json!({ "kind": kind, "code": code, "name": name, "aliases": aliases(*code) }),
            );
        }
    }
    json!({
        "abiVersion": ABI_VERSION,
        "opsVersion": OPS_VERSION,
        "manifestRevision": "1",
        "ops": ops,
        "handleKinds": HandleKind::ALL,
        "errorCodes": error_codes,
        "types": types(),
        "registeredGaps": [
            "G-a: recording/session/infer state remains process-wide",
            "G-b: recording schema migration belongs to RFC-0041",
            "G-c: replay transport capabilities and credential creation are future",
            "G-d: business JSON schemas and language DTO generation belong to RFC-0042",
            "G-e: per-asset platform install verification is not claimed"
        ],
    })
}

/// SHA-256 of recursively key-sorted compact JSON, independent of map insertion order.
#[must_use]
pub fn manifest_digest() -> String {
    digest(manifest())
}

fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.sort_keys();
            for value in object.values_mut() {
                canonicalize(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize(value);
            }
        }
        _ => {}
    }
}

fn digest(mut value: Value) -> String {
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
    }
    canonicalize(&mut value);
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

// Slot 4 remains retired; protocol codes occupy a separate range.
const ERROR_CODES: &[(&str, &[(i32, &str)])] = &[
    (
        "aimux",
        &[
            (1, "Other"),
            (2, "JsonParse"),
            (3, "InvalidResponseData"),
            (5, "InvalidArgument"),
            (6, "InvalidPrompt"),
            (7, "TokenExpired"),
            (8, "UnsupportedFunctionality"),
            (9, "NoSuchModel"),
            (10, "NoSuchProvider"),
            (11, "ApiCall"),
            (12, "Timeout"),
            (13, "Aborted"),
            (14, "Retry"),
            (15, "NoSuchTool"),
            (16, "InvalidToolInput"),
            (17, "ToolCallRepair"),
            (18, "NoOutputGenerated"),
            (19, "LoadApiKey"),
            (20, "LoadSetting"),
        ],
    ),
    (
        "recording",
        &[
            (100, "Init"),
            (101, "OpenFile"),
            (102, "Spawn"),
            (103, "WriterGone"),
            (104, "FlushTimeout"),
            (105, "Write"),
        ],
    ),
    (
        "binding",
        &[
            (200, "NullPointer"),
            (201, "InvalidUtf8"),
            (202, "InvalidWireJson"),
            (203, "InvalidHandle"),
            (204, "ReentrantCall"),
            (205, "ResultSerialization"),
            (206, "CallbackFailure"),
        ],
    ),
    (
        "protocol",
        &[
            (300, "InvalidOp"),
            (301, "InvalidParams"),
            (302, "UnsupportedOpsVersion"),
            (303, "ConcurrentConsumer"),
            (304, "InvalidState"),
            (305, "ResourceLimit"),
            (306, "HandleExhausted"),
        ],
    ),
];

fn aliases(code: i32) -> &'static [&'static str] {
    match code {
        3 => &[
            "InvalidResponseData",
            "ToolCallNotFoundForApproval",
            "ToolChoiceViolation",
        ],
        6 => &["InvalidPrompt", "MissingToolResults"],
        _ => &[],
    }
}

fn types() -> Vec<Value> {
    let mut ids = std::collections::BTreeSet::from(["ErrorEnvelope"]);
    for (descriptor, _) in crate::dispatch::registrations() {
        ids.insert(descriptor.request_type);
        ids.insert(descriptor.result_type);
        ids.extend(descriptor.event_type);
        ids.extend(descriptor.terminal_type);
    }
    ids.into_iter().map(|id| {
        let path = match id {
            "EmptyParams" | "OperationNextParams" | "TranscriptionStartParams" | "BinaryInput" | "OperationPollResult" => format!("aimux_ops::ops::operation::{id}"),
            "OperationStatus" | "CancelResult" => format!("aimux_ops::operation::{id}"),
            "PollResult" => "aimux_ops::dispatch::PollResult".into(),
            "HandleId" => "aimux_ops::handle::HandleId".into(),
            "StreamFrame" => "aimux_ops::frame::StreamFrame".into(),
            "ErrorEnvelope" => "aimux_ops::envelope::ErrorEnvelope".into(),
            _ => id.to_owned(),
        };
        let mut entry = json!({"id": id, "rustPath": path});
        let schema = match id {
            "HandleId" => Some(controls::handle()),
            "BinaryInput" => Some(json!({"type":"array","items":{"type":"integer","minimum":0,"maximum":255},"description":"Native binary push bytes; generic JSON dispatch is not admitted."})),
            "ErrorEnvelope" => Some(error_envelope_schema()),
            "StreamFrame" => Some(json!({"$defs":{"ErrorEnvelope":error_envelope_schema()},"oneOf":[
                {"type":"object","properties":{"type":{"const":"event"},"seq":{"type":"string","pattern":"^[0-9]+$"},"event":{}},"required":["type","seq","event"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"const":"end"},"seq":{"type":"string","pattern":"^[0-9]+$"},"outcome":{"enum":["success","error","cancelled"]},"error":{"$ref":"#/$defs/ErrorEnvelope"}},"required":["type","seq","outcome"],"additionalProperties":false,"allOf":[
                    {"if":{"properties":{"outcome":{"const":"error"}}},"then":{"required":["error"]}},
                    {"if":{"properties":{"outcome":{"const":"success"}}},"then":{"not":{"required":["error"]}}}
                ]}
            ]})),
            "EmptyParams" => Some(json!({"type":"object", "properties":{}, "additionalProperties":false})),
            "OperationNextParams" => Some(json!({"type":"object", "properties":{"timeoutMs":{"type":"integer","minimum":0,"maximum":u64::MAX}}, "additionalProperties":false})),
            "TranscriptionStartParams" => Some(json!({"type":"object", "properties":{
                "inputAudioFormat":{"type":["object","null"],"properties":{"formatType":{"type":"string"},"rate":{"type":["integer","null"],"minimum":0,"maximum":u32::MAX}},"required":["formatType"],"additionalProperties":false},
                "providerOptions":{"type":["object","null"]}, "headers":{"type":["object","null"]},
                "includeRawChunks":{"type":"boolean"}, "timeout":{"type":["object","null"],"properties":{
                    "totalMs":{"type":["integer","null"],"minimum":0,"maximum":u64::MAX},
                    "stepMs":{"type":["integer","null"],"minimum":0,"maximum":u64::MAX},
                    "firstChunkMs":{"type":["integer","null"],"minimum":0,"maximum":u64::MAX},
                    "chunkMs":{"type":["integer","null"],"minimum":0,"maximum":u64::MAX}
                },"additionalProperties":false}, "abort":{"type":["string","null"],"pattern":"^[0-9]+$"}
            },"additionalProperties":false})),
            "CancelResult" => Some(json!({"type":"string","enum":["first","alreadyRequested","alreadyTerminal"]})),
            "OperationPollResult" => Some(json!({"oneOf":[
                {"type":"object","properties":{"type":{"const":"part"},"part":{}},"required":["type","part"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"enum":["ended","timeout"]}},"required":["type"],"additionalProperties":false}
            ]})),
            "OperationStatus" => Some(json!({"type":"object","properties":{
                "subkind":{"const":"transcription"},"state":{"enum":["running","inputClosed","terminal"]},
                "capabilities":{"type":"object","properties":{"push":{"type":"boolean"},"inputDone":{"type":"boolean"},"next":{"type":"boolean"},"cancel":{"type":"boolean"}},"required":["push","inputDone","next","cancel"],"additionalProperties":false},
                "consumerClosed":{"type":"boolean"},
                "queuedInputItems":{"type":"integer","minimum":0},"queuedInputBytes":{"type":"integer","minimum":0},
                "queuedOutputItems":{"type":"integer","minimum":0},"queuedOutputBytes":{"type":"integer","minimum":0},
                "inputDoneSequence":{"type":"string","pattern":"^[0-9]+$"},
                "terminal":{"type":"object","properties":{"outcome":{"enum":["success","error","cancelled"]},"error":{"type":"object"}},"required":["outcome"],"additionalProperties":false},
                "cleanupFailure":{"type":"string"}
            },"required":["subkind","state","capabilities","consumerClosed","queuedInputItems","queuedInputBytes","queuedOutputItems","queuedOutputBytes"],"additionalProperties":false})),
            _ => controls::schema(id),
        };
        if let Some(schema) = schema { entry["schema"] = schema; }
        entry
    }).collect()
}

fn error_envelope_schema() -> Value {
    json!({"type":"object","properties":{
        "kind":{"enum":["aimux","recording","binding","protocol"]},
        "code":{"type":"integer"},"message":{"type":"string"},
        "error":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]},
        "retryAfterMs":{"type":"integer","minimum":0,"maximum":u64::MAX}
    },"required":["kind","code","message","error"],"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_sorts_objects_recursively_and_preserves_arrays() {
        let first: Value = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":3}"#).unwrap();
        let second: Value = serde_json::from_str(r#"{"a":3,"z":[{"a":1,"b":2}]}"#).unwrap();
        assert_eq!(digest(first), digest(second));
        assert_ne!(digest(json!([1, 2])), digest(json!([2, 1])));
        assert_eq!(
            digest(json!({})),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn manifest_has_stable_versions_kinds_codes_and_registered_ops() {
        let value = manifest();
        assert_eq!(value["abiVersion"], 65536);
        assert_eq!(value["opsVersion"], "1.0");
        assert_eq!(value["handleKinds"].as_array().unwrap().len(), 23);
        let codes = value["errorCodes"].as_array().unwrap();
        assert!(
            codes
                .iter()
                .any(|code| code["code"] == 306 && code["name"] == "HandleExhausted")
        );
        assert!(!codes.iter().any(|code| code["code"] == 4));
        let ops = value["ops"].as_array().unwrap();
        assert_eq!(ops.len(), crate::dispatch::registrations().len());
        assert!(
            ops.windows(2)
                .all(|pair| pair[0]["name"].as_str() < pair[1]["name"].as_str())
        );
        assert_eq!(manifest_digest().len(), 64);
        assert_eq!(manifest_digest(), manifest_digest());
    }
}
