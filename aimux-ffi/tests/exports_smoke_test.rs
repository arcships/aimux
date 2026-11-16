//! C export and retired-file guards. Build the cdylib before running this test; set
//! AIMUX_FFI_LIBRARY to audit a specific artifact. New primitive behavior is
//! covered by the ABI primitive tests.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const PRIMITIVES: &[&str] = &[
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
];

fn collect_build_dir_cdylibs(dir: &Path, name: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let candidate = entry.path().join("out").join(name);
        if candidate.is_file() {
            out.push(candidate);
        }
    }
}

fn push_profile_candidates(profile: &Path, name: &str, out: &mut Vec<PathBuf>) {
    let direct = profile.join(name);
    if direct.is_file() {
        out.push(direct);
    }
    // Cargo 1.100+ writes `cargo test` cdylib artifacts inside `<profile>/build/aimux-ffi/<hash>/out/`.
    collect_build_dir_cdylibs(&profile.join("build").join("aimux-ffi"), name, out);
}

fn profile_dir(executable: &Path) -> Option<&Path> {
    let exe_dir = executable.parent()?;
    if exe_dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
        return exe_dir.parent();
    }
    // Cargo 1.100+ places test executables under `<profile>/build/<pkg>/<hash>/out/`.
    for ancestor in exe_dir.ancestors() {
        if ancestor.file_name() == Some(std::ffi::OsStr::new("build")) {
            return ancestor.parent();
        }
    }
    exe_dir.parent()
}

fn library() -> PathBuf {
    if let Some(path) = std::env::var_os("AIMUX_FFI_LIBRARY") {
        return path.into();
    }
    let executable = std::env::current_exe().expect("test executable path");
    let name = format!(
        "{}aimux_ffi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let mut candidates = Vec::new();
    if let Some(exe_dir) = executable.parent() {
        let dependency = exe_dir.join(&name);
        if dependency.is_file() {
            candidates.push(dependency);
        }
    }

    let profile =
        profile_dir(&executable).expect("test executable must be under a Cargo profile directory");
    push_profile_candidates(profile, &name, &mut candidates);

    // When Cargo redirects intermediate artifacts via `build.build-dir`, final
    // cdylib artifacts still land under `CARGO_TARGET_DIR` (or `<workspace>/target`).
    if let Some(profile_name) = profile.file_name() {
        let maybe_triple = profile.parent().and_then(Path::file_name);
        let target_root = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .map(|root| root.join("target"))
            });
        if let Some(root) = target_root {
            if let Some(triple) = maybe_triple {
                push_profile_candidates(
                    &root.join(triple).join(profile_name),
                    &name,
                    &mut candidates,
                );
            }
            push_profile_candidates(&root.join(profile_name), &name, &mut candidates);
        }
    }

    candidates
        .into_iter()
        .max_by_key(|path| {
            path.metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .unwrap_or_else(|| profile.join(name))
}

#[test]
fn cdylib_exports_only_abi_primitives() {
    let library = library();
    assert!(
        library.is_file(),
        "build the cdylib first, or set AIMUX_FFI_LIBRARY: {}",
        library.display()
    );
    let mut command = Command::new("nm");
    if cfg!(target_os = "macos") {
        command.arg("-gU");
    } else {
        command.args(["-D", "-g", "--defined-only"]);
    }
    let output = command.arg(&library).output().expect("run nm on cdylib");
    assert!(
        output.status.success(),
        "nm failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let symbols = String::from_utf8(output.stdout).expect("nm output must be UTF-8");
    let actual: BTreeSet<_> = symbols
        .lines()
        .filter_map(|line| line.split_whitespace().next_back())
        .map(|symbol| symbol.strip_prefix('_').unwrap_or(symbol))
        .filter(|symbol| symbol.starts_with("aimux_"))
        .collect();
    assert_eq!(
        actual,
        PRIMITIVES.iter().copied().collect(),
        "C export allowlist mismatch"
    );
}

#[test]
fn retired_headers_and_session_are_absent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for retired in [
        "aimux-ffi.h",
        "aimux-error.h",
        "src/transcription_session.rs",
    ] {
        assert!(!root.join(retired).exists(), "retired file: {retired}");
    }
}
