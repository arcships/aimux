//! Release artifacts derived from the ops manifest. No build script is involved.

pub mod emit;
pub mod model;

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

pub type Artifacts = BTreeMap<PathBuf, Vec<u8>>;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const INDEX: &str = "tools/aimux-codegen/generated-files.json";
const HEADERS: [&str; 2] = [
    "aimux-ffi/include/aimux.h",
    "bindings/flutter/native/aimux_ops.h",
];
const SCHEMA_BASE: &str = "https://aimux.dev/schema/ops/";
const LANGUAGES: &[(&str, &str)] = &[
    ("go", "ops.go"),
    ("java", "AimuxOps.java"),
    ("kotlin", "AimuxOps.kt"),
    ("swift", "AimuxOps.swift"),
    ("flutter", "ops.dart"),
    ("node", "ops.ts"),
    ("python", "ops.py"),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    abi_version: u32,
    ops_version: String,
    manifest_revision: Value,
    ops: Vec<Op>,
    handle_kinds: Vec<String>,
    error_codes: Vec<ErrorCode>,
    #[serde(default)]
    types: Vec<TypeSchema>,
    type_defs: Value,
}

#[derive(Deserialize)]
struct TypeSchema {
    id: String,
    input: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Op {
    name: String,
    mode: Mode,
    allowed_handle_kinds: Vec<String>,
    request_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Mode {
    Call,
    Stream,
    BinaryPush,
    Poll,
}

#[derive(Deserialize)]
struct ErrorCode {
    name: String,
    code: i32,
    kind: Option<String>,
}

/// Sort object keys recursively, preserving array order and JSON number values.
///
/// # Errors
/// Returns an error if JSON serialization fails.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    let mut value = value.clone();
    value.sort_all_objects();
    Ok(serde_json::to_vec(&value)?)
}

fn pretty_json(value: &Value) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(&serde_json::from_slice::<Value>(&canonical_json(value)?)?)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || !value.as_bytes()[0].is_ascii_alphabetic()
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
    {
        return Err(format!("invalid manifest identifier: {value:?}").into());
    }
    let chars: Vec<_> = value.chars().collect();
    let mut name = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '.' || ch == '_' {
            if !name.ends_with('_') {
                name.push('_');
            }
        } else {
            if ch.is_ascii_uppercase() && i > 0 {
                let previous = chars[i - 1];
                let next_is_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
                if previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next_is_lower)
                {
                    name.push('_');
                }
            }
            name.push(ch.to_ascii_uppercase());
        }
    }
    Ok(name)
}

fn error_domain(code: i32) -> Result<&'static str> {
    match code {
        1..=20 => Ok("aimux"),
        100..=105 => Ok("recording"),
        200..=206 => Ok("binding"),
        300..=399 => Ok("protocol"),
        _ => Err(format!("unregistered error code range: {code}").into()),
    }
}

fn validate(manifest: &Manifest) -> Result<()> {
    let version: Vec<_> = manifest.ops_version.split('.').collect();
    if version.len() != 2
        || version
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        || manifest.manifest_revision.is_null()
    {
        return Err("invalid ops major.minor version or missing manifest revision".into());
    }
    let kinds: BTreeSet<_> = manifest.handle_kinds.iter().collect();
    if kinds.len() != manifest.handle_kinds.len() {
        return Err("duplicate handle kind".into());
    }
    let mut symbols = BTreeSet::new();
    for op in &manifest.ops {
        if !symbols.insert(identifier(&op.name)?) {
            return Err(format!("duplicate or colliding op constant: {}", op.name).into());
        }
        if op.request_type.is_empty() {
            return Err(format!("missing request type: {}", op.name).into());
        }
        for kind in &op.allowed_handle_kinds {
            if !kinds.contains(kind) {
                return Err(format!("unknown handle kind {kind} in {}", op.name).into());
            }
        }
    }
    let mut codes = BTreeSet::new();
    let mut names = BTreeSet::new();
    for error in &manifest.error_codes {
        let domain = error_domain(error.code)?;
        if error.kind.as_deref().is_some_and(|kind| kind != domain) {
            return Err(format!("error domain disagrees with code {}", error.code).into());
        }
        if !codes.insert(error.code)
            || !names.insert(format!("{domain}_{}", identifier(&error.name)?))
        {
            return Err(format!("duplicate error constant: {}", error.name).into());
        }
    }
    Ok(())
}

/// Generate all owned artifacts, rejecting disagreement with the runtime digest.
///
/// # Errors
/// Returns an error for digest drift, malformed descriptors, constant name
/// collisions, or serialization failure.
pub fn generate(value: &Value, runtime_digest: &str) -> Result<Artifacts> {
    let mut unsigned = value.clone();
    unsigned
        .as_object_mut()
        .ok_or("manifest must be an object")?
        .remove("digest");
    if let Some(ops) = unsigned.get_mut("ops").and_then(Value::as_array_mut) {
        ops.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }
    let digest = format!("{:x}", Sha256::digest(canonical_json(&unsigned)?));
    if runtime_digest != digest {
        return Err(
            format!("manifest digest drift: runtime={runtime_digest}, canonical={digest}").into(),
        );
    }
    let mut manifest: Manifest = serde_json::from_value(value.clone())?;
    validate(&manifest)?;
    manifest.ops.sort_by(|a, b| a.name.cmp(&b.name));
    manifest.error_codes.sort_by_key(|error| error.code);
    let mut artifacts = Artifacts::new();
    let header = header(&manifest, &digest)?.into_bytes();
    let [c_header, flutter_header] = HEADERS;
    artifacts.insert(c_header.into(), header.clone());
    artifacts.insert(flutter_header.into(), header);
    for &(language, filename) in LANGUAGES {
        artifacts.insert(
            format!("bindings/{language}/generated/{filename}").into(),
            constants(&manifest, &digest, language)?.into_bytes(),
        );
    }
    let (mut model, definitions) = model::normalize_with_schemas(&manifest.type_defs)?;
    model.drop_builtin_aliases();
    for emitter in emit::EMITTERS {
        for (path, contents) in emitter.files(&model) {
            artifacts.insert(path, contents.into_bytes());
        }
    }
    for (name, schema) in &definitions {
        artifacts.insert(
            format!("schema/types/{name}.json").into(),
            pretty_json(&type_document(name, schema.clone()))?,
        );
    }
    for (name, schema) in schemas(&manifest, &definitions)? {
        artifacts.insert(
            format!("schema/ops/{name}.json").into(),
            pretty_json(&schema)?,
        );
    }
    let mut files: Vec<_> = artifacts
        .keys()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    files.push(INDEX.into());
    artifacts.insert(
        INDEX.into(),
        pretty_json(&json!({"manifestDigest": digest, "files": files}))?,
    );
    Ok(artifacts)
}

fn header(manifest: &Manifest, digest: &str) -> Result<String> {
    let mut text = format!(
        "/* Generated by aimux-codegen. Do not edit. Manifest SHA-256: {digest} */\n#ifndef AIMUX_H\n#define AIMUX_H\n\n#include <stddef.h>\n#include <stdint.h>\n\n#ifdef __cplusplus\nextern \"C\" {{\n#endif\n\n#define AIMUX_ABI_VERSION UINT32_C({})\n#define AIMUX_OPS_VERSION {}\n#define AIMUX_MANIFEST_DIGEST \"{digest}\"\n\n",
        manifest.abi_version,
        serde_json::to_string(&manifest.ops_version)?
    );
    for op in &manifest.ops {
        text.push_str(&format!(
            "#define AIMUX_OP_{} \"{}\"\n",
            identifier(&op.name)?,
            op.name
        ));
    }
    text.push_str("\ntypedef struct aimux_error aimux_error_t;\n\ntypedef enum aimux_error_code {\n    AIMUX_ERROR_NONE = 0");
    for error in &manifest.error_codes {
        text.push_str(&format!(
            ",\n    AIMUX_ERROR_{}_{} = {}",
            error_domain(error.code)?.to_ascii_uppercase(),
            identifier(&error.name)?,
            error.code
        ));
    }
    text.push_str("\n} aimux_error_code_t;\n\ntypedef enum aimux_poll_state {\n    AIMUX_POLL_PART = 1,\n    AIMUX_POLL_ENDED = 2,\n    AIMUX_POLL_TIMEOUT = 3\n} aimux_poll_state_t;\n\n/* frame_json is borrowed until the callback returns; copy it to retain it.\n * context must remain valid until aimux_stream returns. */\ntypedef void (*aimux_event_cb)(const char *frame_json, void *context);\n\n/* ABI = (major << 16) | minor. Verify ABI and manifest before calling ops.\n * JSON inputs are borrowed NUL-terminated UTF-8, never implicit empty objects.\n * Handle 0 is global scope; nonzero IDs are process-owned and never reused.\n * Fallible calls return NULL on success or an owned error on failure.\n * Outputs are reset to NULL/0 before validation, including on failure.\n * All output JSON and error getter strings are owned: aimux_free_string.\n * Release errors with aimux_error_free. Do not free callback JSON.\n * Blocking calls cannot be reentered from a callback (ReentrantCall). */\nuint32_t aimux_abi_version(void);\naimux_error_t *aimux_manifest_json(char **out_json);\naimux_error_t *aimux_call(uint64_t handle, const char *op,\n                         const char *params_json, uint64_t abort, char **out_json);\naimux_error_t *aimux_stream(uint64_t handle, const char *op,\n                           const char *params_json, uint64_t abort,\n                           aimux_event_cb on_event, void *context);\n/* Input bytes are copied/consumed before return. NULL is valid only at length 0. */\naimux_error_t *aimux_operation_push(uint64_t operation,\n                                   const uint8_t *data, size_t length);\n/* PART allocates out_part; ENDED/TIMEOUT do not. timeout_ms must be nonnegative. */\naimux_error_t *aimux_operation_next_part(uint64_t operation, int64_t timeout_ms,\n                                        char **out_part, int32_t *out_state);\n/* Signal only; callback-safe. Use abort.signal to report an invalid handle. */\nvoid aimux_abort(uint64_t abort);\n/* Drop is idempotent, accepts 0, and cancels resources without implying flush. */\nvoid aimux_drop(uint64_t handle);\n/* Non-blocking finalizer; token encodes the handle in uintptr_t (never dereferenced). */\nvoid aimux_handle_finalizer(void *token);\n/* NULL-safe. Each owned allocation must be freed exactly once. */\nvoid aimux_free_string(char *string);\nvoid aimux_error_free(aimux_error_t *error);\nint32_t aimux_error_code(const aimux_error_t *error);\nchar *aimux_error_message(const aimux_error_t *error);\nchar *aimux_error_json(const aimux_error_t *error);\n\n#ifdef __cplusplus\n}\n#endif\n\n#endif /* AIMUX_H */\n");
    Ok(text)
}

fn constants(manifest: &Manifest, digest: &str, language: &str) -> Result<String> {
    let comment = if language == "python" { "#" } else { "//" };
    let mut text =
        format!("{comment} Generated by aimux-codegen. Do not edit. Manifest SHA-256: {digest}\n");
    let (prefix, suffix, indent) = match language {
        "go" => ("package generated\n\nconst (\n", ")\n", "\t"),
        "java" => (
            "package ai.arcships.aimux.generated;\n\npublic final class AimuxOps {\n    private AimuxOps() {}\n",
            "}\n",
            "    ",
        ),
        "kotlin" => (
            "package ai.arcships.aimux.generated\n\nobject AimuxKotlinOps {\n",
            "}\n",
            "    ",
        ),
        "swift" => ("public enum AimuxOps {\n", "}\n", "    "),
        "flutter" | "node" | "python" => ("", "", ""),
        _ => return Err(format!("unsupported language: {language}").into()),
    };
    text.push_str(prefix);
    let mut values = vec![
        ("ABI_VERSION".into(), manifest.abi_version.to_string(), true),
        (
            "OPS_VERSION".into(),
            serde_json::to_string(&manifest.ops_version)?,
            false,
        ),
        ("MANIFEST_DIGEST".into(), format!("\"{digest}\""), false),
    ];
    for op in &manifest.ops {
        values.push((
            format!("OP_{}", identifier(&op.name)?),
            format!("\"{}\"", op.name),
            false,
        ));
    }
    for (name, value, number) in values {
        let declaration = match language {
            "go" => format!("{name} = {value}"),
            "java" => format!(
                "public static final {} {name} = {value}{};",
                if number { "long" } else { "String" },
                if number { "L" } else { "" }
            ),
            "kotlin" => format!(
                "const val {name}{} = {value}",
                if number { ": Long" } else { "" }
            ),
            "swift" => format!(
                "public static let {name}{} = {value}",
                if number { ": UInt32" } else { "" }
            ),
            "flutter" => format!(
                "const {} {name} = {value};",
                if number { "int" } else { "String" }
            ),
            "node" => format!("export const {name} = {value} as const;"),
            "python" => format!("{name} = {value}"),
            _ => return Err(format!("unsupported language: {language}").into()),
        };
        text.push_str(&format!("{indent}{declaration}\n"));
    }
    text.push_str(suffix);
    Ok(text)
}

fn schema_document(name: &str, schema: Value) -> Value {
    let mut schema = if schema == false {
        json!({"not": {}})
    } else {
        schema
    };
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$id"] = json!(format!("{SCHEMA_BASE}{name}.json"));
    if schema.get("$comment").is_none() {
        schema["$comment"] = json!("Generated by aimux-codegen. Do not edit.");
    }
    schema
}

fn closed_object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn type_document(name: &str, schema: Value) -> Value {
    let mut schema = if schema == true { json!({}) } else { schema };
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$id"] = json!(format!("https://aimux.dev/schema/types/{name}.json"));
    schema["$comment"] = json!("Generated by aimux-codegen. Do not edit.");
    schema
}

fn root_name<'a>(manifest: &'a Manifest, id: &str, direction: &str) -> Result<&'a str> {
    manifest
        .types
        .iter()
        .find(|ty| ty.id == id)
        .and_then(|ty| ty.input.as_deref())
        .ok_or_else(|| format!("missing {direction} root: {id}").into())
}

fn schemas(
    manifest: &Manifest,
    definitions: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let mut documents = BTreeMap::new();
    for (file, name) in [
        ("handle-id", "HandleId"),
        ("error-envelope", "ErrorEnvelope"),
        ("stream-frame", "StreamFrame"),
    ] {
        if definitions.contains_key(name) {
            documents.insert(
                file.into(),
                schema_document(file, json!({"$ref": format!("../types/{name}.json")})),
            );
        }
    }
    let names = model::contract_names(&manifest.type_defs)?;
    let mut requests = Vec::new();
    for op in &manifest.ops {
        let mut handle = json!({"$ref": "handle-id.json"});
        if op.allowed_handle_kinds.is_empty() {
            handle["const"] = json!("0");
        } else if op.name != "handle.drop" {
            handle["not"] = json!({"const": "0"});
        }
        let base = root_name(manifest, &op.request_type, "input")?;
        let name = &names[&format!("#/typeDefs/input/{base}")];
        let params = json!({"$ref": format!("../types/{name}.json")});
        let mut properties = json!({"handle": handle, "op": {"const": op.name}, "params": params});
        if matches!(op.mode, Mode::Call | Mode::Stream) {
            properties["abort"] = json!({"$ref": "handle-id.json"});
        }
        requests.push(closed_object(properties, &["handle", "op", "params"]));
    }
    let request = if requests.is_empty() {
        json!(false)
    } else {
        json!({"oneOf": requests})
    };
    documents.insert("request".into(), schema_document("request", request));
    Ok(documents)
}

fn managed_path(path: &Path) -> bool {
    if path == Path::new(INDEX) || HEADERS.iter().any(|header| path == Path::new(header)) {
        return true;
    }
    if path.starts_with("schema/ops") || path.starts_with("schema/types") {
        return path.extension().is_some_and(|ext| ext == "json");
    }
    if emit::owned_paths().any(|owned| path.starts_with(owned)) {
        return true;
    }
    let constants = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with("ops.") || name.starts_with("ops-") || name.starts_with("AimuxOps")
        });
    constants
        && LANGUAGES.iter().any(|(language, filename)| {
            path.starts_with(format!("bindings/{language}/generated"))
                && path.extension() == Path::new(filename).extension()
        })
}

fn collect_files(root: &Path, relative: &Path, output: &mut BTreeSet<PathBuf>) -> Result<()> {
    let full = root.join(relative);
    if !full.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(&full)?;
    if metadata.file_type().is_symlink() {
        return Err(format!("symlink in generated files: {}", full.display()).into());
    }
    if metadata.is_file() {
        output.insert(relative.to_path_buf());
    } else if metadata.is_dir() {
        for entry in fs::read_dir(&full)? {
            collect_files(root, &relative.join(entry?.file_name()), output)?;
        }
    }
    Ok(())
}

fn existing_files(root: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for path in [INDEX]
        .into_iter()
        .chain(HEADERS)
        .chain(["schema/ops", "schema/types"])
        .chain(emit::owned_paths())
    {
        collect_files(root, Path::new(path), &mut files)?;
    }
    for &(language, _) in LANGUAGES {
        collect_files(
            root,
            Path::new(&format!("bindings/{language}/generated")),
            &mut files,
        )?;
    }
    files.retain(|path| managed_path(path));
    Ok(files)
}

/// Write the complete set and remove obsolete artifacts only in owned locations.
///
/// # Errors
/// Returns an error for an unowned path, a generated symlink, or filesystem I/O.
pub fn write(root: &Path, artifacts: &Artifacts) -> Result<()> {
    for path in artifacts.keys() {
        if !managed_path(path)
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(format!("unowned artifact path: {}", path.display()).into());
        }
    }
    let existing = existing_files(root)?;
    for (path, bytes) in artifacts {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full, bytes)?;
    }
    for stale in existing {
        if !artifacts.contains_key(&stale) {
            fs::remove_file(root.join(stale))?;
        }
    }
    Ok(())
}

/// Regenerate into a temporary directory, then compare file sets and bytes.
///
/// # Errors
/// Returns an error listing missing, stale, or differing files, or on I/O failure.
pub fn check(root: &Path, artifacts: &Artifacts) -> Result<()> {
    let temporary = tempfile::tempdir()?;
    write(temporary.path(), artifacts)?;
    let expected = existing_files(temporary.path())?;
    let actual = existing_files(root)?;
    let mut differences = Vec::new();
    for missing in expected.difference(&actual) {
        differences.push(format!("missing: {}", missing.display()));
    }
    for stale in actual.difference(&expected) {
        differences.push(format!("stale: {}", stale.display()));
    }
    for path in expected.intersection(&actual) {
        if fs::read(root.join(path))? != fs::read(temporary.path().join(path))? {
            differences.push(format!("differs: {}", path.display()));
        }
    }
    if differences.is_empty() {
        Ok(())
    } else {
        Err(differences.join("\n").into())
    }
}
