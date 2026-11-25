use std::sync::OnceLock;

use serde_json::Value;
#[cfg(feature = "runtime-manifest")]
use serde_json::json;
#[cfg(any(feature = "runtime-manifest", test))]
use sha2::{Digest, Sha256};

#[cfg(feature = "runtime-manifest")]
use crate::{ABI_VERSION, HandleKind, OPS_VERSION};

pub(crate) const MANIFEST_REVISION: &str = "1";

const EMBEDDED_MANIFEST_ZLIB: &[u8] = include_bytes!("manifest.json.zlib");
const EMBEDDED_MANIFEST_DIGEST: &str = include_str!("manifest.digest");

static MANIFEST_JSON: OnceLock<Box<str>> = OnceLock::new();

/// Compact canonical manifest JSON, decompressed once from the precomputed snapshot.
///
/// # Panics
/// Panics only if the build-time generated snapshot embedded in the crate is corrupt.
#[must_use]
pub fn manifest_json() -> &'static str {
    MANIFEST_JSON
        .get_or_init(|| {
            let bytes = miniz_oxide::inflate::decompress_to_vec_zlib(EMBEDDED_MANIFEST_ZLIB)
                .expect("embedded manifest snapshot is valid zlib");
            String::from_utf8(bytes)
                .expect("embedded manifest snapshot is valid UTF-8")
                .into_boxed_str()
        })
        .as_ref()
}

/// Describe all registered operations and their wire metadata.
///
/// # Panics
/// Panics only if the build-time generated snapshot embedded in the crate is corrupt.
#[must_use]
pub fn manifest() -> Value {
    serde_json::from_str(manifest_json()).expect("embedded manifest is valid JSON")
}

/// SHA-256 of recursively key-sorted compact JSON, independent of map insertion order.
#[must_use]
pub fn manifest_digest() -> String {
    EMBEDDED_MANIFEST_DIGEST.trim().to_owned()
}

/// Build the canonical manifest JSON and its SHA-256 digest live from registered
/// operations and `schemars` type contracts. Used by `aimux-codegen` and contract
/// tests to regenerate and drift-check the embedded snapshot.
///
/// # Panics
/// Panics if a registered type schema lacks an `id` or `direction` field.
#[cfg(feature = "runtime-manifest")]
#[must_use]
pub fn generate_manifest() -> (Box<str>, String) {
    let (input_types, input) = type_contract(false);
    let (output_types, output) = type_contract(true);
    let mut types = std::collections::BTreeMap::new();
    for ty in input_types.into_iter().chain(output_types) {
        let id = ty["id"].as_str().unwrap().to_owned();
        let entry = types
            .entry(id.clone())
            .or_insert_with(|| json!({"id": id, "rustPath": id}));
        entry[ty["direction"].as_str().unwrap()] = ty["name"].clone();
    }
    let types: Vec<_> = types.into_values().collect();
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
    let mut value = json!({
        "abiVersion": ABI_VERSION,
        "opsVersion": OPS_VERSION,
        "manifestRevision": MANIFEST_REVISION,
        "ops": ops,
        "handleKinds": HandleKind::ALL,
        "packages": crate::ops::provider::packages(),
        "errorCodes": error_codes,
        "types": types,
        "typeDefs": {"input": input, "output": output},
        "registeredGaps": [
            "G-a: recording/session/infer state remains process-wide",
            "G-b: recording schema migration belongs to RFC-0041",
            "G-c: replay transport capabilities and credential creation are future",
            "G-e: per-asset platform install verification is not claimed"
        ],
    });
    canonicalize(&mut value);
    let digest_str = format!("{:x}", Sha256::digest(value.to_string().as_bytes()));
    value["digest"] = json!(digest_str.clone());
    let compact_json = value.to_string().into_boxed_str();
    (compact_json, digest_str)
}

#[cfg(any(feature = "runtime-manifest", test))]
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

#[cfg(test)]
fn digest(mut value: Value) -> String {
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
    }
    canonicalize(&mut value);
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

// Slot 4 remains retired; protocol codes occupy a separate range.
#[cfg(feature = "runtime-manifest")]
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

#[cfg(feature = "runtime-manifest")]
fn aliases(code: i32) -> &'static [&'static str] {
    match code {
        3 => &[
            "InvalidResponseData",
            "ToolCallNotFoundForApproval",
            "ToolChoiceViolation",
            "TypeValidation",
        ],
        6 => &["InvalidPrompt", "MissingToolResults"],
        _ => &[],
    }
}

#[cfg(feature = "runtime-manifest")]
fn type_contract(serialize: bool) -> (Vec<Value>, Value) {
    use schemars::generate::{Contract, SchemaSettings};
    let direction = if serialize { "output" } else { "input" };
    let mut generator = SchemaSettings::default()
        .with(|settings| {
            settings.contract = if serialize {
                Contract::Serialize
            } else {
                Contract::Deserialize
            };
            settings.definitions_path = format!("#/typeDefs/{direction}/").into();
        })
        .into_generator();
    let mut roots = std::collections::BTreeMap::new();
    for (descriptor, _) in crate::dispatch::registrations() {
        let types = if serialize {
            vec![
                Some(descriptor.result_type),
                descriptor.event_type,
                descriptor.terminal_type,
            ]
        } else {
            vec![Some(descriptor.request_type)]
        };
        for ty in types.into_iter().flatten() {
            roots.insert((ty.id)().into_owned(), ty);
        }
    }
    if serialize {
        // Bindings also read errors, recordings and trace records, which no op returns.
        use crate::op::TypeRef;
        for ty in [
            TypeRef::of::<crate::ErrorEnvelope>(),
            TypeRef::of::<aimux_core::recording::Recording>(),
            TypeRef::of::<aimux_core::trace::TraceRecord>(),
        ] {
            roots.insert((ty.id)().into_owned(), ty);
        }
    }
    let mut types = Vec::new();
    for (id, ty) in roots {
        let schema = (ty.schema)(&mut generator).to_value();
        // Inline primitives still need a named definition for op references.
        let name = if let Some(reference) = schema["$ref"].as_str() {
            reference.rsplit('/').next().unwrap().to_owned()
        } else {
            let name = id
                .replace("::", "_")
                .replace(['<', '>', ' ', '(', ')', '[', ']'], "_");
            generator.definitions_mut().insert(name.clone(), schema);
            name
        };
        types.push(json!({"id": id, "rustPath": id, "direction": direction, "name": name}));
    }
    (types, json!(generator.definitions()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert_eq!(value["digest"], manifest_digest());
        assert_eq!(serde_json::to_string(&value).unwrap(), manifest_json());
    }

    #[cfg(feature = "runtime-manifest")]
    #[test]
    fn embedded_snapshot_matches_live_runtime_manifest_byte_for_byte() {
        let (live_json, live_digest) = generate_manifest();
        assert_eq!(manifest_digest(), live_digest);
        assert_eq!(manifest_json(), live_json.as_ref());
        assert_eq!(
            manifest(),
            serde_json::from_str::<Value>(&live_json).unwrap()
        );
    }
}
