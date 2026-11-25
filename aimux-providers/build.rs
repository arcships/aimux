use std::env;
use std::fs;
use std::path::PathBuf;

fn minify_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in input.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if !matches!(ch, ' ' | '\t' | '\r' | '\n') {
            out.push(ch);
        }
    }
    assert!(
        !in_string,
        "unterminated JSON string while minifying embedded provider data"
    );
    out
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    for (src, dst) in [
        ("src/provider_registry.json", "provider_registry.min.json"),
        ("src/provider_aliases.json", "provider_aliases.min.json"),
    ] {
        println!("cargo:rerun-if-changed={src}");
        let raw =
            fs::read_to_string(src).unwrap_or_else(|err| panic!("failed to read {src}: {err}"));
        let compact = minify_json(&raw);
        let dest = out_dir.join(dst);
        fs::write(&dest, compact)
            .unwrap_or_else(|err| panic!("failed to write {}: {err}", dest.display()));
    }
}
