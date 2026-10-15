use std::{env, path::PathBuf, process::ExitCode};

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
    let artifacts = aimux_codegen::generate(
        &aimux_ops::manifest::manifest(),
        &aimux_ops::manifest::manifest_digest(),
    )?;
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
