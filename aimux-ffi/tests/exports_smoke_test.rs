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

fn library() -> PathBuf {
    if let Some(path) = std::env::var_os("AIMUX_FFI_LIBRARY") {
        return path.into();
    }
    let executable = std::env::current_exe().expect("test executable path");
    let profile = executable
        .parent()
        .and_then(Path::parent)
        .expect("test executable must be in the profile's deps directory");
    let name = format!(
        "{}aimux_ffi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let dependency = executable.parent().unwrap().join(&name);
    if dependency.exists() {
        dependency
    } else {
        profile.join(name)
    }
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
