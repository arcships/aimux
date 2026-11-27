#[cfg(feature = "runtime-manifest")]
use std::collections::BTreeMap;
#[cfg(any(feature = "runtime-manifest", test))]
use std::collections::{BTreeSet, VecDeque};
use std::sync::OnceLock;

use serde_json::Value;
#[cfg(any(feature = "runtime-manifest", test))]
use serde_json::{Map, json};
use sha2::{Digest, Sha256};

#[cfg(feature = "runtime-manifest")]
use crate::{ABI_VERSION, HandleKind, OPS_VERSION};

pub(crate) const MANIFEST_REVISION: &str = "1";
#[cfg(any(feature = "runtime-manifest", test))]
const TYPE_REVISION: &str = "1";

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

/// Compute a deterministic SHA-256 hash for one entry in `manifest.types`.
///
/// Formula: collect every definition in `typeDefs.input` and `typeDefs.output`
/// transitively reachable via `"$ref": "#/typeDefs/<direction>/<defName>"` from
/// the entry's `"input"` and `"output"` definition roots (cycle-safe, RFC 6901
/// pointer unescaped), construct
/// `{"id", "revision", optional "input", optional "output", "typeDefs": {"input", "output"}}`,
/// recursively sort all JSON object keys (preserving array order), serialize as
/// compact JSON, and return lowercase hex SHA-256.
#[cfg(any(feature = "runtime-manifest", test))]
fn compute_type_schema_hash(entry: &Value, type_defs: &Value) -> String {
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for direction in ["input", "output"] {
        if let Some(name) = entry.get(direction).and_then(Value::as_str) {
            let key = (direction.to_owned(), name.to_owned());
            if visited.insert(key.clone()) {
                queue.push_back(key);
            }
        }
    }

    let mut reachable_input = Map::new();
    let mut reachable_output = Map::new();
    while let Some((direction, name)) = queue.pop_front() {
        let Some(def) = type_defs
            .get(&direction)
            .and_then(|defs| defs.get(&name))
            .cloned()
        else {
            continue;
        };
        collect_type_refs(&def, &mut visited, &mut queue);
        if direction == "input" {
            reachable_input.insert(name, def);
        } else {
            reachable_output.insert(name, def);
        }
    }

    let mut spec = json!({
        "id": entry["id"],
        "revision": entry["revision"],
        "typeDefs": {
            "input": Value::Object(reachable_input),
            "output": Value::Object(reachable_output),
        },
    });
    if let Some(input_name) = entry.get("input") {
        spec["input"] = input_name.clone();
    }
    if let Some(output_name) = entry.get("output") {
        spec["output"] = output_name.clone();
    }
    canonicalize(&mut spec);
    format!("{:x}", Sha256::digest(spec.to_string().as_bytes()))
}

#[cfg(any(feature = "runtime-manifest", test))]
fn collect_type_refs(
    value: &Value,
    visited: &mut BTreeSet<(String, String)>,
    queue: &mut VecDeque<(String, String)>,
) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                && let Some((direction, name)) = parse_type_def_ref(reference)
            {
                let key = (direction, name);
                if visited.insert(key.clone()) {
                    queue.push_back(key);
                }
            }
            for child in object.values() {
                collect_type_refs(child, visited, queue);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_type_refs(item, visited, queue);
            }
        }
        _ => {}
    }
}

#[cfg(any(feature = "runtime-manifest", test))]
fn parse_type_def_ref(reference: &str) -> Option<(String, String)> {
    let rest = reference.strip_prefix("#/typeDefs/")?;
    let (direction, tail) = rest.split_once('/')?;
    if !matches!(direction, "input" | "output") {
        return None;
    }
    let raw_name = tail.split('/').next()?;
    if raw_name.is_empty() {
        return None;
    }
    Some((direction.to_owned(), unescape_json_pointer(raw_name)))
}

#[cfg(any(feature = "runtime-manifest", test))]
fn unescape_json_pointer(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                Some(other) => {
                    out.push('~');
                    out.push(other);
                }
                None => out.push('~'),
            }
        } else {
            out.push(ch);
        }
    }
    out
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
///
/// Self-verification of either runtime `manifest()` or emitted `schema/manifest.json`
/// strips both `"digest"` and `"inputHash"` from the top-level object, sorts `"ops"`
/// by `"name"`, recursively sorts all object keys, and computes SHA-256 over the
/// compact JSON bytes.
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
    let type_defs = json!({"input": input, "output": output});
    let mut types = BTreeMap::new();
    for ty in input_types.into_iter().chain(output_types) {
        let id = ty["id"].as_str().unwrap().to_owned();
        let entry = types
            .entry(id.clone())
            .or_insert_with(|| json!({"id": id, "rustPath": id, "revision": TYPE_REVISION}));
        entry[ty["direction"].as_str().unwrap()] = ty["name"].clone();
    }
    let types: Vec<_> = types
        .into_values()
        .map(|mut entry| {
            entry["schemaHash"] = json!(compute_type_schema_hash(&entry, &type_defs));
            entry
        })
        .collect();
    let sources: Vec<_> = crate::sources::UPSTREAM_SOURCES
        .iter()
        .map(|(package, version, integrity)| {
            json!({
                "package": package,
                "version": version,
                "integrity": integrity,
            })
        })
        .collect();
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
        "generatorVersion": env!("CARGO_PKG_VERSION"),
        "sources": sources,
        "ops": ops,
        "handleKinds": HandleKind::ALL,
        "packages": crate::ops::provider::packages(),
        "errorCodes": error_codes,
        "types": types,
        "typeDefs": type_defs,
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

/// Compute the canonical manifest digest from a parsed manifest JSON value.
///
/// Strips top-level `"digest"` and `"inputHash"` (added by `aimux-codegen` to
/// `schema/manifest.json`), sorts `"ops"` by `"name"`, recursively sorts all
/// JSON object keys, and returns lowercase hex SHA-256 of the compact JSON.
#[must_use]
pub fn canonical_digest(mut value: Value) -> String {
    if let Some(object) = value.as_object_mut() {
        object.remove("digest");
        object.remove("inputHash");
    }
    if let Some(ops) = value.get_mut("ops").and_then(Value::as_array_mut) {
        ops.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }
    canonicalize(&mut value);
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
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

#[cfg(test)]
fn digest(value: Value) -> String {
    canonical_digest(value)
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
        // Bindings also use these standalone DTOs outside operation results.
        use crate::op::TypeRef;
        for ty in [
            TypeRef::of::<crate::ErrorEnvelope>(),
            TypeRef::of::<aimux_core::recording::Recording>(),
            TypeRef::of::<aimux_core::trace::TraceRecord>(),
            // These content projections were previously reachable from the text snapshot.
            TypeRef::of::<aimux_core::result::ReasoningPart>(),
            TypeRef::of::<aimux_core::result::TaggedToolResult>(),
            TypeRef::of::<aimux_core::result::TypedSource>(),
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
        assert_eq!(
            manifest_digest(),
            "6914772fdc03df08cba497def41cb354fe52e40eba49844a66938b4cd38c5d83",
        );
        assert_eq!(value["abiVersion"], 65536);
        assert_eq!(value["opsVersion"], "1.0");
        assert_eq!(value["generatorVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["sources"].as_array().unwrap().len(), 22);
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

    #[test]
    fn type_schema_hash_tracks_transitive_and_cyclic_refs_only() {
        let entry = json!({
            "id": "demo::Node",
            "revision": TYPE_REVISION,
            "input": "Node",
        });
        let defs_a = json!({
            "input": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "leaf": {"$ref": "#/typeDefs/input/Leaf"},
                        "next": {"$ref": "#/typeDefs/input/Node"}
                    }
                },
                "Leaf": {"type": "string"},
                "Unrelated": {"type": "number"}
            },
            "output": {}
        });
        let defs_reordered_and_extra = json!({
            "input": {
                "Unrelated": {"type": "boolean"},
                "Leaf": {"type": "string"},
                "Node": {
                    "properties": {
                        "next": {"$ref": "#/typeDefs/input/Node"},
                        "leaf": {"$ref": "#/typeDefs/input/Leaf"}
                    },
                    "type": "object"
                }
            },
            "output": {}
        });
        let mut defs_leaf_changed = defs_a.clone();
        defs_leaf_changed["input"]["Leaf"] = json!({"type": "integer"});

        let hash_a = compute_type_schema_hash(&entry, &defs_a);
        assert_eq!(hash_a.len(), 64);
        assert_eq!(
            hash_a,
            compute_type_schema_hash(&entry, &defs_reordered_and_extra)
        );
        assert_ne!(hash_a, compute_type_schema_hash(&entry, &defs_leaf_changed));
    }
}
