//! C export and retired-file guards. Build the cdylib before running this test; set
//! AIMUX_FFI_LIBRARY to audit a specific artifact. New primitive behavior is
//! covered by the ABI primitive tests.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

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

fn dll_name() -> String {
    format!(
        "{}aimux_ffi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

fn find_integration_fingerprint_json(fp_dir: &Path) -> Option<PathBuf> {
    let preferred = fp_dir.join("test-integration-test-exports_smoke_test.json");
    if preferred.is_file() {
        return Some(preferred);
    }
    let entries = std::fs::read_dir(fp_dir).ok()?;
    let mut json_files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() == Some(OsStr::new("json")) && path.is_file() {
            json_files.push(path);
        }
    }
    if json_files.len() == 1 {
        json_files.pop()
    } else {
        None
    }
}

fn expected_aimux_ffi_fingerprint(test_unit_dir: &Path) -> Option<String> {
    let json_path = find_integration_fingerprint_json(&test_unit_dir.join("fingerprint"))?;
    let bytes = std::fs::read(json_path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let deps = value.get("deps")?.as_array()?;
    let mut hashes = Vec::new();
    for entry in deps {
        if entry.get(1).and_then(serde_json::Value::as_str) == Some("aimux_ffi") {
            let hash = entry.get(3)?.as_u64()?;
            let mut hex = String::with_capacity(16);
            for byte in hash.to_le_bytes() {
                let _ = write!(&mut hex, "{byte:02x}");
            }
            hashes.push(hex);
        }
    }
    if hashes.len() == 1 {
        hashes.pop()
    } else {
        None
    }
}

fn resolve_build_dir_cdylib(
    test_unit_dir: &Path,
    pkg_build_dir: &Path,
    profile_dir: Option<&Path>,
    name: &str,
) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(pkg_build_dir) {
        for entry in entries.flatten() {
            let unit_dir = entry.path();
            let cdylib_path = unit_dir.join("out").join(name);
            if cdylib_path.is_file() {
                candidates.push((unit_dir, cdylib_path));
            }
        }
    }
    candidates.sort_by(|left, right| left.1.cmp(&right.1));
    let all_paths: Vec<PathBuf> = candidates.iter().map(|(_, path)| path.clone()).collect();

    if let Some(expected_fp) = expected_aimux_ffi_fingerprint(test_unit_dir) {
        let mut matched = Vec::new();
        for (unit_dir, cdylib_path) in candidates {
            let lib_fp = unit_dir.join("fingerprint").join("lib-aimux_ffi");
            if std::fs::read_to_string(lib_fp)
                .ok()
                .is_some_and(|contents| contents.trim() == expected_fp)
            {
                matched.push(cdylib_path);
            }
        }
        return match matched.len() {
            1 => Ok(matched.swap_remove(0)),
            0 => Err(format!(
                "no cdylib in {} matched test Cargo fingerprint {expected_fp} (unrelated candidates: {all_paths:?}); build the cdylib first, or set AIMUX_FFI_LIBRARY",
                pkg_build_dir.display()
            )),
            _ => Err(format!(
                "multiple cdylib artifacts matched Cargo fingerprint {expected_fp}; set AIMUX_FFI_LIBRARY: {matched:?}"
            )),
        };
    }

    match all_paths.len() {
        1 => Ok(all_paths.into_iter().next().unwrap_or_default()),
        0 => {
            if let Some(profile) = profile_dir {
                let direct = profile.join(name);
                if direct.is_file() {
                    return Ok(direct);
                }
            }
            Err(format!(
                "build the cdylib first, or set AIMUX_FFI_LIBRARY: {}",
                pkg_build_dir.display()
            ))
        }
        _ => Err(format!(
            "ambiguous cdylib candidates in {} without Cargo fingerprint metadata; set AIMUX_FFI_LIBRARY: {all_paths:?}",
            pkg_build_dir.display()
        )),
    }
}

fn build_dir_context(exe_dir: &Path) -> Option<(&Path, &Path, Option<&Path>)> {
    if exe_dir.file_name() != Some(OsStr::new("out")) {
        return None;
    }
    let test_unit_dir = exe_dir.parent()?;
    let pkg_build_dir = test_unit_dir.parent()?;
    let build_dir = pkg_build_dir.parent()?;
    if build_dir.file_name() != Some(OsStr::new("build")) {
        return None;
    }
    Some((test_unit_dir, pkg_build_dir, build_dir.parent()))
}

fn resolve_library_from(
    executable: &Path,
    env_override: Option<PathBuf>,
    name: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = env_override {
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!(
                "build the cdylib first, or set AIMUX_FFI_LIBRARY: {}",
                path.display()
            ))
        };
    }

    let exe_dir = executable.parent().ok_or_else(|| {
        format!(
            "test executable has no parent directory: {}",
            executable.display()
        )
    })?;
    let adjacent = exe_dir.join(name);
    if adjacent.is_file() {
        return Ok(adjacent);
    }

    // Cargo 1.100+ places test executables under `<profile>/build/<pkg>/<test_unit>/out/<exe>`.
    if let Some((test_unit_dir, pkg_build_dir, profile_dir)) = build_dir_context(exe_dir) {
        return resolve_build_dir_cdylib(test_unit_dir, pkg_build_dir, profile_dir, name);
    }

    let profile = exe_dir.parent().ok_or_else(|| {
        format!(
            "test executable must be under a profile directory: {}",
            executable.display()
        )
    })?;
    let candidate = profile.join(name);
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "build the cdylib first, or set AIMUX_FFI_LIBRARY: {}",
            candidate.display()
        ))
    }
}

fn exported_aimux_symbols(library: &Path) -> BTreeSet<String> {
    let mut command = Command::new("nm");
    if cfg!(target_os = "macos") {
        command.arg("-gU");
    } else {
        command.args(["-D", "-g", "--defined-only"]);
    }
    let output = command.arg(library).output().expect("run nm on cdylib");
    assert!(
        output.status.success(),
        "nm failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let symbols = String::from_utf8(output.stdout).expect("nm output must be UTF-8");
    symbols
        .lines()
        .filter_map(|line| line.split_whitespace().next_back())
        .map(|symbol| symbol.strip_prefix('_').unwrap_or(symbol))
        .filter(|symbol| symbol.starts_with("aimux_"))
        .map(ToOwned::to_owned)
        .collect()
}

#[test]
fn cdylib_exports_only_abi_primitives() {
    let name = dll_name();
    let executable = std::env::current_exe().expect("test executable path");
    let env_override = std::env::var_os("AIMUX_FFI_LIBRARY").map(PathBuf::from);
    let library = resolve_library_from(&executable, env_override, &name)
        .unwrap_or_else(|err| panic!("{err}"));
    let actual = exported_aimux_symbols(&library);
    let expected: BTreeSet<String> = PRIMITIVES
        .iter()
        .map(|symbol| (*symbol).to_owned())
        .collect();
    assert_eq!(
        actual,
        expected,
        "C export allowlist mismatch in {}",
        library.display()
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

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("aimux-ffi-{label}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_file_with_mtime(path: &Path, bytes: &[u8], modified: SystemTime) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(path, bytes).expect("write fixture file");
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open file for mtime");
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .expect("set mtime");
}

fn time_at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

#[cfg(unix)]
fn compile_fixture_cdylib(src_dir: &Path, file_name: &str, out_path: &Path, extra_export: bool) {
    let src_path = src_dir.join(file_name);
    let mut source = String::new();
    for symbol in PRIMITIVES {
        let _ = writeln!(
            &mut source,
            "#[no_mangle] pub extern \"C\" fn {symbol}() {{}}"
        );
    }
    if extra_export {
        source.push_str("#[no_mangle] pub extern \"C\" fn aimux_unexpected_export() {}\n");
    }
    std::fs::write(&src_path, source).expect("write rustc cdylib fixture source");
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("create output parent dir");
    }
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .args(["--edition=2021", "--crate-type", "cdylib"])
        .arg(&src_path)
        .arg("-o")
        .arg(out_path)
        .status()
        .expect("invoke rustc for cdylib fixture");
    assert!(status.success(), "rustc failed compiling cdylib fixture");
}

#[test]
fn deps_cdylib_takes_precedence_over_newer_profile_and_build_cdylibs() {
    let tmp = TempDir::new("deps-precedence");
    let name = dll_name();
    let exe = tmp.path().join("debug/deps/exports_smoke_test-abc");
    let deps_lib = tmp.path().join("debug/deps").join(&name);
    let top_lib = tmp.path().join("debug").join(&name);
    let build_lib = tmp
        .path()
        .join("debug/build/aimux-ffi/unit1/out")
        .join(&name);

    write_file_with_mtime(&exe, b"", time_at(1_000));
    #[cfg(unix)]
    {
        compile_fixture_cdylib(tmp.path(), "bad.rs", &deps_lib, true);
        compile_fixture_cdylib(tmp.path(), "clean.rs", &top_lib, false);
        let deps_bytes = std::fs::read(&deps_lib).expect("read bad fixture");
        let clean_bytes = std::fs::read(&top_lib).expect("read clean fixture");
        write_file_with_mtime(&deps_lib, &deps_bytes, time_at(1_000));
        write_file_with_mtime(&top_lib, &clean_bytes, time_at(2_000));
        write_file_with_mtime(&build_lib, &clean_bytes, time_at(3_000));
    }
    #[cfg(not(unix))]
    {
        write_file_with_mtime(&deps_lib, b"bad", time_at(1_000));
        write_file_with_mtime(&top_lib, b"clean", time_at(2_000));
        write_file_with_mtime(&build_lib, b"clean", time_at(3_000));
    }

    let resolved = resolve_library_from(&exe, None, &name).expect("resolve deps cdylib");
    assert_eq!(resolved, deps_lib);
    #[cfg(unix)]
    assert!(
        exported_aimux_symbols(&resolved).contains("aimux_unexpected_export"),
        "must audit older adjacent deps library rather than newer top-level library"
    );
}

#[test]
fn cargo_build_dir_selects_older_fingerprint_matched_unit_over_newer_candidates() {
    let tmp = TempDir::new("build-dir-fp");
    let name = dll_name();
    let test_unit = tmp.path().join("debug/build/aimux-ffi/test_unit");
    let exe = test_unit.join("out/exports_smoke_test-111");
    let fp_json = test_unit.join("fingerprint/test-integration-test-exports_smoke_test.json");

    write_file_with_mtime(&exe, b"", time_at(1_000));
    write_file_with_mtime(
        &fp_json,
        br#"{"deps":[[1,"aimux_ffi",false,3892163839031152530]]}"#,
        time_at(1_000),
    );

    let lib_old = tmp.path().join("debug/build/aimux-ffi/lib_old");
    let lib_old_out = lib_old.join("out").join(&name);
    write_file_with_mtime(
        &lib_old.join("fingerprint/lib-aimux_ffi"),
        b"928f12d963be0336\n",
        time_at(1_000),
    );

    let lib_new = tmp.path().join("debug/build/aimux-ffi/lib_new");
    let lib_new_out = lib_new.join("out").join(&name);
    write_file_with_mtime(
        &lib_new.join("fingerprint/lib-aimux_ffi"),
        b"d6d3e4bf8df4f035\n",
        time_at(2_000),
    );

    let top_lib = tmp.path().join("debug").join(&name);

    #[cfg(unix)]
    {
        compile_fixture_cdylib(tmp.path(), "bad.rs", &lib_old_out, true);
        compile_fixture_cdylib(tmp.path(), "clean.rs", &lib_new_out, false);
        let old_bytes = std::fs::read(&lib_old_out).expect("read bad fixture");
        let clean_bytes = std::fs::read(&lib_new_out).expect("read clean fixture");
        write_file_with_mtime(&lib_old_out, &old_bytes, time_at(1_000));
        write_file_with_mtime(&lib_new_out, &clean_bytes, time_at(2_000));
        write_file_with_mtime(&top_lib, &clean_bytes, time_at(3_000));
    }
    #[cfg(not(unix))]
    {
        write_file_with_mtime(&lib_old_out, b"old", time_at(1_000));
        write_file_with_mtime(&lib_new_out, b"new", time_at(2_000));
        write_file_with_mtime(&top_lib, b"top", time_at(3_000));
    }

    let resolved = resolve_library_from(&exe, None, &name).expect("resolve matched Cargo unit");
    assert_eq!(resolved, lib_old_out);
    #[cfg(unix)]
    assert!(
        exported_aimux_symbols(&resolved).contains("aimux_unexpected_export"),
        "must audit older fingerprint-matched unit rather than newer unrelated candidate"
    );
}

#[test]
fn cargo_build_dir_rejects_ambiguous_units_without_fingerprint() {
    let tmp = TempDir::new("build-dir-ambiguous");
    let name = dll_name();
    let test_unit = tmp.path().join("debug/build/aimux-ffi/test_unit");
    let exe = test_unit.join("out/exports_smoke_test-111");
    let unit_a_out = tmp
        .path()
        .join("debug/build/aimux-ffi/lib_a/out")
        .join(&name);
    let unit_b_out = tmp
        .path()
        .join("debug/build/aimux-ffi/lib_b/out")
        .join(&name);

    write_file_with_mtime(&exe, b"", time_at(1_000));
    write_file_with_mtime(&unit_a_out, b"a", time_at(1_000));
    write_file_with_mtime(&unit_b_out, b"b", time_at(2_000));

    let err = resolve_library_from(&exe, None, &name)
        .expect_err("multiple units without fingerprint must be rejected");
    assert!(
        err.contains("AIMUX_FFI_LIBRARY"),
        "error should instruct setting AIMUX_FFI_LIBRARY: {err}"
    );

    let fp_json = test_unit.join("fingerprint/test-integration-test-exports_smoke_test.json");
    write_file_with_mtime(
        &fp_json,
        br#"{"deps":[[1,"aimux_ffi",false,3892163839031152530]]}"#,
        time_at(1_000),
    );
    write_file_with_mtime(
        &tmp.path()
            .join("debug/build/aimux-ffi/lib_b/fingerprint/lib-aimux_ffi"),
        b"d6d3e4bf8df4f035\n",
        time_at(2_000),
    );
    let mismatch_err = resolve_library_from(&exe, None, &name)
        .expect_err("non-matching fingerprint must be rejected");
    assert!(
        mismatch_err.contains("AIMUX_FFI_LIBRARY"),
        "error should instruct setting AIMUX_FFI_LIBRARY: {mismatch_err}"
    );
}
