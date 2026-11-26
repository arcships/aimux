use std::{collections::BTreeMap, env, fs, path::Path, path::PathBuf, process::ExitCode};

use serde::Deserialize;

#[derive(Deserialize)]
struct VersionEntry {
    version: String,
    integrity: String,
}

fn load_versions(
    versions_path: &Path,
) -> Result<Vec<aimux_codegen::UpstreamSource>, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string(versions_path)?;
    let parsed: BTreeMap<String, VersionEntry> = serde_json::from_str(&raw)?;
    Ok(parsed
        .into_iter()
        .map(|(package, entry)| aimux_codegen::UpstreamSource {
            package,
            version: entry.version,
            integrity: entry.integrity,
        })
        .collect())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut check = false;
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--root" => root = PathBuf::from(args.next().ok_or("--root requires a path")?),
            "--help" | "-h" => {
                println!("aimux-codegen [--check] [--root PATH]");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let (manifest_json, digest) = aimux_ops::manifest::generate_manifest();
    let manifest_value: serde_json::Value = serde_json::from_str(&manifest_json)?;
    let repo_versions =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/aisdk/VERSIONS.json");
    let root_versions = root.join("fixtures/aisdk/VERSIONS.json");
    let versions_path = if root_versions.exists() {
        root_versions
    } else {
        repo_versions
    };
    let expected_sources = load_versions(&versions_path)?;
    let compiled_sources: Vec<aimux_codegen::UpstreamSource> =
        serde_json::from_value(manifest_value["sources"].clone())?;
    if compiled_sources != expected_sources {
        if check {
            return Err(format!(
                "aimux-ops/src/sources.rs is out of sync with {}; run aimux-codegen to update",
                versions_path.display()
            )
            .into());
        }
        aimux_codegen::write_sources(&root, &expected_sources)?;
        return Err(
            "updated aimux-ops/src/sources.rs from fixtures/aisdk/VERSIONS.json; re-run aimux-codegen to regenerate artifacts with the new manifest digest"
                .into(),
        );
    }
    let artifacts = aimux_codegen::generate(&manifest_value, &digest)?;
    if check {
        aimux_codegen::check(&root, &artifacts)?;
        println!("generated files and manifest digest match");
    } else {
        aimux_codegen::write(&root, &artifacts)?;
        println!("generated {} files", artifacts.len());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aimux-codegen: {error}");
            ExitCode::FAILURE
        }
    }
}
