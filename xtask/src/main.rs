use anyhow::{Context, Result, ensure};
use std::{fs, path::PathBuf};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3 && args[0] == "package",
        "Usage: cargo run -p cranpose-build-xtask -- package <target-triple> <output-directory>"
    );
    let triple = &args[1];
    ensure!(
        cranpose_build::Platform::ALL
            .iter()
            .any(|p| p.triple() == triple),
        "Unknown target triple"
    );
    let filename = if triple.contains("windows") {
        "cranpose-build.exe"
    } else {
        "cranpose-build"
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Workspace directory")?
        .to_owned();
    let source = root
        .join("target")
        .join(triple)
        .join("release")
        .join(filename);
    ensure!(source.is_file(), "Build {} first", source.display());
    let output = PathBuf::from(&args[2]);
    fs::create_dir_all(&output)?;
    let zip = output.join(format!("cranpose-build-{triple}.zip"));
    cranpose_build::engine::archive_path(&source, &zip)?;
    fs::write(
        zip.with_extension("zip.sha256"),
        format!(
            "{}  {}\n",
            cranpose_build::engine::hash(&zip)?,
            zip.file_name()
                .context("Archive filename")?
                .to_string_lossy()
        ),
    )?;
    println!("{}", zip.display());
    Ok(())
}
