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
