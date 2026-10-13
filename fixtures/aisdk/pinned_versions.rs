//! Test-only reader for `fixtures/aisdk/VERSIONS.json`, shared by every
//! caller through `#[path]` (it is compiled only into test targets, never
//! into a published crate). Every entry must be the unit 1.11 object
//! `{"version": "...", "integrity": "sha512-..."}`.

/// Every entry of VERSIONS.json as `(package, version)`; panics unless each
/// entry is an object with a string `version` and a `sha512-` `integrity`.
pub fn all() -> Vec<(String, String)> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/aisdk/VERSIONS.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let doc: serde_json::Value = serde_json::from_str(&text).expect("VERSIONS.json is JSON");
    let entries = doc.as_object().expect("VERSIONS.json is an object");
    assert!(!entries.is_empty(), "VERSIONS.json is empty");
    entries
        .iter()
        .map(|(package, entry)| {
            let entry = entry
                .as_object()
                .unwrap_or_else(|| panic!("{package}: entry is not {{version, integrity}}"));
            let version = entry["version"]
                .as_str()
                .unwrap_or_else(|| panic!("{package}: no string version"));
            let integrity = entry.get("integrity").and_then(|v| v.as_str());
            assert!(
                integrity.is_some_and(|i| i.starts_with("sha512-")),
                "{package}: no sha512 integrity"
            );
            (package.clone(), version.to_string())
        })
        .collect()
}

/// The version VERSIONS.json pins for `package`.
pub fn pinned(package: &str) -> String {
    all()
        .into_iter()
        .find(|(name, _)| name == package)
        .unwrap_or_else(|| panic!("{package} is not pinned in VERSIONS.json"))
        .1
}

/// The exact version the pinned `package` depends on for `dependency`, read
/// from the recorder's `scripts/aisdk-fixtures/package-lock.json` (the
/// published package.json `dependencies`; npm installs that exact version
/// for it). The lock's copy of `package` must be the pinned version.
#[allow(dead_code)] // not every crate that includes this file reads it
pub fn pinned_dependency(package: &str, dependency: &str) -> String {
    let packages = recorder_lock_packages();
    let entry = &packages[format!("node_modules/{package}")];
    assert_eq!(
        entry["version"].as_str(),
        Some(pinned(package).as_str()),
        "{package}: package-lock.json and VERSIONS.json disagree"
    );
    entry["dependencies"][dependency]
        .as_str()
        .unwrap_or_else(|| panic!("{package}: no dependency {dependency} in package-lock.json"))
        .to_string()
}

/// The exact version that `nested`, as the pinned `parent` installs it,
/// depends on for `dependency`: `parent`'s own copy of `nested` when npm nests
/// one, else the top-level copy. Either way it must be the version `parent`
/// pins.
#[allow(dead_code)] // not every crate that includes this file reads it
pub fn nested_dependency(parent: &str, nested: &str, dependency: &str) -> String {
    let packages = recorder_lock_packages();
    let own = &packages[format!("node_modules/{parent}/node_modules/{nested}")];
    let entry = if own.is_object() {
        own
    } else {
        &packages[format!("node_modules/{nested}")]
    };
    assert_eq!(
        entry["version"].as_str(),
        Some(pinned_dependency(parent, nested).as_str()),
        "{parent}: package-lock.json does not install the {nested} it pins"
    );
    entry["dependencies"][dependency]
        .as_str()
        .unwrap_or_else(|| panic!("{parent} > {nested}: no dependency {dependency}"))
        .to_string()
}

/// The `packages` map of the recorder's `scripts/aisdk-fixtures/package-lock.json`.
#[allow(dead_code)] // not every crate that includes this file reads it
fn recorder_lock_packages() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scripts/aisdk-fixtures/package-lock.json"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut lock: serde_json::Value =
        serde_json::from_str(&text).expect("package-lock.json is JSON");
    lock["packages"].take()
}
