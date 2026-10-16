//! Pin the binding to its checked-in generated metadata.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pyo3_build_config::add_extension_module_link_args();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=python/aimux/ops.py");
    let metadata = fs::read_to_string("python/aimux/ops.py")?;
    let mut constants = String::new();
    for (key, rust_type) in [
        ("ABI_VERSION", "u32"),
        ("OPS_VERSION", "&str"),
        ("MANIFEST_DIGEST", "&str"),
    ] {
        let prefix = format!("{key} = ");
        let value = metadata
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .ok_or_else(|| format!("generated metadata missing {key}"))?;
        constants += &format!("pub const EXPECTED_{key}: {rust_type} = {value};\n");
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("contract.rs"),
        constants,
    )?;
    Ok(())
}
